//! chat3 transcript plane wire codec and transport seam (docs/chat3-sync.md).
//!
//! The plane rides the chat2 frame layout (`chat_frames`): stored rows are
//! `item` frames, open parts are `live` frames, a page or feed ends with
//! `itemsDone`. Host upserts go up as a length-prefixed run of `itemPut`
//! frames in one `POST /items` (the JSON reply is the ack); pages and the
//! change feed come back the same way from `GET /items`, the `/rows` body
//! encoding. Live frames ride the chat's existing socket.

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use zeron_doc::transcript_plane::{Item, ItemKey, Live, LiveOp, ReceivedItem};

use crate::SyncError;
use crate::chat_frames::{self as wire, frame_type};

/// `GET/POST /chat3/{id}/items` and `GET/PUT /chat3/{id}/outputs/{part}`.
/// Engine and app clients implement it over their own HTTP stacks.
pub trait TranscriptTransport: Send + Sync + 'static {
    /// POST a length-prefixed `itemPut` body; returns the JSON ack.
    fn post_items(&self, body: Vec<u8>) -> BoxFuture<'static, Result<String, SyncError>>;
    /// GET with a query (`limit=…&before=…` or `since=…`); returns the body.
    fn fetch_items(&self, query: String) -> BoxFuture<'static, Result<Vec<u8>, SyncError>>;
    fn put_output(&self, part: String, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), SyncError>>;
    /// `None` = the room has no output for this part (404).
    fn get_output(&self, part: String) -> BoxFuture<'static, Result<Option<Vec<u8>>, SyncError>>;
}

#[derive(Serialize)]
struct ItemPutHeader<'a> {
    ord: f64,
    idx: i64,
    entry: &'a str,
}

#[derive(Deserialize)]
struct ItemHeader {
    ord: f64,
    idx: i64,
    entry: String,
    rev: u64,
}

#[derive(Serialize, Deserialize)]
struct LiveHeader {
    entry: String,
    ord: f64,
    idx: i64,
    op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    at: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemsDone {
    pub head_rev: u64,
    #[serde(default)]
    pub more: bool,
}

/// One `GET /items` body, decoded.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ItemsResponse {
    pub items: Vec<ReceivedItem>,
    pub live: Vec<Live>,
    /// `None` when the server truncated the body at its cap: apply what
    /// arrived and ask again from the new head.
    pub done: Option<ItemsDone>,
}

/// The ack of one `POST /items`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostAck {
    pub head_rev: u64,
    pub results: Vec<PostResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PostResult {
    #[serde(default)]
    pub ord: Option<f64>,
    #[serde(default)]
    pub idx: Option<i64>,
    #[serde(default)]
    pub rev: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// u32-LE length-prefixed frames (the `/rows` body encoding).
pub fn length_prefixed(frames: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.iter().map(|f| f.len() + 4).sum());
    for frame in frames {
        out.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        out.extend_from_slice(frame);
    }
    out
}

pub fn split_length_prefixed(body: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 4 <= body.len() {
        let len = u32::from_le_bytes(body[off..off + 4].try_into().unwrap_or_default()) as usize;
        off += 4;
        if off + len > body.len() {
            break;
        }
        out.push(&body[off..off + len]);
        off += len;
    }
    out
}

pub fn encode_item_puts(items: &[Item]) -> Vec<u8> {
    let frames: Vec<Vec<u8>> = items
        .iter()
        .map(|item| {
            wire::encode(
                frame_type::ITEM_PUT,
                &ItemPutHeader {
                    ord: item.key.ord,
                    idx: item.key.idx,
                    entry: &item.entry,
                },
                &item.json,
            )
        })
        .collect();
    length_prefixed(&frames)
}

pub fn encode_live(live: &Live) -> Vec<u8> {
    let (op, at, payload) = match &live.op {
        LiveOp::Part(json) => ("part", None, json.clone()),
        LiveOp::Append { at, text } => ("append", Some(*at), text.as_bytes().to_vec()),
    };
    wire::encode(
        frame_type::LIVE,
        &LiveHeader {
            entry: live.entry.clone(),
            ord: live.key.ord,
            idx: live.key.idx,
            op: op.into(),
            at,
        },
        &payload,
    )
}

pub fn decode_live(header: &serde_json::Value, payload: &[u8]) -> Option<Live> {
    let header: LiveHeader = serde_json::from_value(header.clone()).ok()?;
    let op = match header.op.as_str() {
        "part" => LiveOp::Part(payload.to_vec()),
        "append" => LiveOp::Append {
            at: header.at?,
            text: String::from_utf8(payload.to_vec()).ok()?,
        },
        _ => return None,
    };
    Some(Live {
        key: ItemKey {
            ord: header.ord,
            idx: header.idx,
        },
        entry: header.entry,
        op,
    })
}

pub fn decode_item(header: &serde_json::Value, payload: &[u8]) -> Option<ReceivedItem> {
    let header: ItemHeader = serde_json::from_value(header.clone()).ok()?;
    Some(ReceivedItem {
        key: ItemKey {
            ord: header.ord,
            idx: header.idx,
        },
        entry: header.entry,
        rev: header.rev,
        json: payload.to_vec(),
    })
}

pub fn decode_items_body(body: &[u8]) -> ItemsResponse {
    let mut out = ItemsResponse::default();
    for raw in split_length_prefixed(body) {
        let Some(frame) = wire::decode(raw) else { continue };
        match frame.kind {
            frame_type::ITEM => {
                if let Some(item) = decode_item(&frame.header, &frame.payload) {
                    out.items.push(item);
                }
            }
            frame_type::LIVE => {
                if let Some(live) = decode_live(&frame.header, &frame.payload) {
                    out.live.push(live);
                }
            }
            frame_type::ITEMS_DONE => {
                out.done = serde_json::from_value(frame.header).ok();
            }
            _ => {}
        }
    }
    out
}

/// Query for the newest page before `before` (or the end).
pub fn page_query(before: Option<ItemKey>, limit: usize) -> String {
    match before {
        Some(key) => format!("limit={limit}&before={}", key.cursor()),
        None => format!("limit={limit}"),
    }
}

pub fn feed_query(since: u64) -> String {
    format!("since={since}")
}

/// The ref a gen-3 tool part's `outputRef` carries: `chat3:{chatId}/{partId}`.
pub fn output_ref(chat_id: &str, part_id: &str) -> String {
    format!("chat3:{chat_id}/{part_id}")
}

/// Split a `chat3:` output ref back into `(chatId, partId)`.
pub fn parse_output_ref(reference: &str) -> Option<(&str, &str)> {
    let rest = reference.strip_prefix("chat3:")?;
    let (chat, part) = rest.split_once('/')?;
    (!chat.is_empty() && !part.is_empty()).then_some((chat, part))
}

/// The edge's part id charset (`PART_RE` in chat-transcript.ts).
pub fn valid_part_id(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 200
        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.:#~-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_puts_round_trip_through_the_page_decoder_shape() {
        let item = Item {
            key: ItemKey { ord: 1.5, idx: 3 },
            entry: "e1".into(),
            json: br#"{"kind":"text","text":"hi"}"#.to_vec(),
        };
        let body = encode_item_puts(std::slice::from_ref(&item));
        let frames = split_length_prefixed(&body);
        assert_eq!(frames.len(), 1);
        let frame = wire::decode(frames[0]).unwrap();
        assert_eq!(frame.kind, frame_type::ITEM_PUT);
        assert_eq!(frame.header, serde_json::json!({"ord": 1.5, "idx": 3, "entry": "e1"}));
        assert_eq!(frame.payload, item.json);
    }

    #[test]
    fn live_frames_round_trip() {
        for op in [
            LiveOp::Part(br#"{"kind":"tool"}"#.to_vec()),
            LiveOp::Append {
                at: 7,
                text: "wörld".into(),
            },
        ] {
            let live = Live {
                key: ItemKey { ord: 2.0, idx: 0 },
                entry: "e".into(),
                op,
            };
            let frame = wire::decode(&encode_live(&live)).unwrap();
            assert_eq!(frame.kind, frame_type::LIVE);
            assert_eq!(decode_live(&frame.header, &frame.payload), Some(live));
        }
    }

    #[test]
    fn an_items_body_decodes_rows_live_and_done() {
        let item = wire::encode(
            frame_type::ITEM,
            &serde_json::json!({"ord": 1.0, "idx": -1, "entry": "e", "rev": 9}),
            b"{}",
        );
        let live = encode_live(&Live {
            key: ItemKey { ord: 1.0, idx: 0 },
            entry: "e".into(),
            op: LiveOp::Append {
                at: 0,
                text: "x".into(),
            },
        });
        let done = wire::encode(frame_type::ITEMS_DONE, &serde_json::json!({"headRev": 9, "more": true}), &[]);
        let body = length_prefixed(&[item, live, done]);
        let response = decode_items_body(&body);
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].rev, 9);
        assert_eq!(response.live.len(), 1);
        assert_eq!(response.done, Some(ItemsDone { head_rev: 9, more: true }));
        // Truncated at the cap: no done frame.
        let truncated = decode_items_body(&body[..body.len() - 10]);
        assert_eq!(truncated.done, None);
    }

    #[test]
    fn output_refs_name_their_room() {
        let reference = output_ref("chat-1", "toolu_01");
        assert_eq!(parse_output_ref(&reference), Some(("chat-1", "toolu_01")));
        assert_eq!(parse_output_ref("chat-1/toolu_01"), None);
        assert!(valid_part_id("toolu_01:abc"));
        assert!(!valid_part_id("a/b"));
    }

    #[test]
    fn page_cursors_use_the_rooms_key_syntax() {
        assert_eq!(page_query(Some(ItemKey { ord: 3.0, idx: -1 }), 50), "limit=50&before=3:-1");
        assert_eq!(page_query(Some(ItemKey { ord: 1.5, idx: 4 }), 50), "limit=50&before=1.5:4");
        assert_eq!(page_query(None, 200), "limit=200");
    }
}
