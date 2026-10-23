# chat3: a transcript plane beside the chat2 log

Status: IN PROGRESS · Flag: `preferences/labs.chat3` (per user, synced) · Room gen: 3
Prior art: `docs/chat2-sync.md` (the relay this keeps), `docs/registry-sync.md`.

## Why

chat2 made the edge dumb and unwedgeable. Two costs remain, and both scale with how
long a chat runs, not with what the user sees:

1. **Every streamed commit is a billed row.** The host commits the folded segment every
   `STREAM_COMMIT_MS` (120 ms), every commit publishes one Loro update, and every update
   is one `rows` insert in the room. On top of that the room rewrites `headSeq`,
   `backupDirty` and the `pushOutcomes` JSON per push, the `batch_id UNIQUE` index costs a
   row, and a threshold checkpoint deletes every 200 rows (deletes are billed writes too).
   About 7 rows written per push, up to ~8 pushes/s while text streams.
2. **A cold open downloads the whole chat.** The transcript is one Loro doc, so a new
   device, the web client or an evicted chat fetches the full checkpoint and imports
   every entry before it can paint, no matter how far back the history goes.

And one gap: tool outputs never reach the transcript. claude, codex and cursor drop
them at the harness, the fold clears what ACP sends, and the parked R2 sidecar never
uploads. Users see one-line chips for commands that printed pages.

## What changes

A gen-3 chat keeps the chat2 log for the state the devices actually co-edit, and moves
the transcript into a **plane of server-ordered rows** that only the host writes.

| Data | Writers | gen 2 | gen 3 |
|---|---|---|---|
| `meta`, `commands`, `queue` | every device | chat2 Loro log | chat2 Loro log (unchanged, smaller doc) |
| `messages` (entries + parts) | the host only | same Loro log | **items**: one row per entry header / part |
| in-progress text, running tools | the host only | a log row every 120 ms | **live frames**: relayed, never stored |
| full tool output | — | dropped | **outputs**: one row per tool, fetched on expand |

Clients never write `messages` (the inventory behind this doc found no client-side
writer outside the in-process demo host), so a single-writer row model needs no CRDT.

### Local model: one `SessionDoc`, two Loro docs

`SessionDoc` gains a second `LoroDoc` for the transcript. For gen ≤ 2 both handles are
the same doc, so nothing changes. For gen 3 the transcript doc is **local only**:

- On the host the engine writes it exactly as today (`SegmentWriter` at 120 ms,
  `push_message`, status flips). Its commits are cheap SQLite snapshots, not edge rows.
- On readers it is a cache the room fills: items and live frames are applied as local
  writes (commit origin `zeron:transcript-replay` for history, so it does not animate).
- Every transcript reader in the codebase (the engine's `publish_messages_locked` for
  desktop, the client `Tracker` for mobile, `TranscriptHistory`) keeps reading
  `messages` through `SessionDoc`; only their subscriptions move to the transcript doc.

Only the state doc is published through the chat2 `ChatClient`, so its log carries
commands and queue edits, not tokens.

### Items

Each item is one row keyed by `(ord, idx)`:

- `idx = -1` is the **entry header**: `{id, ord, role, createdAt, deviceId, status?,
  continuationOf?, durationMs?}`.
- `idx ≥ 0` is **part `idx`** of that entry, as its doc JSON (the same shape
  `read_entries` decodes).

`ord` is the entry's position, a float the host assigns once when the entry is created
(`last + 1` for appends, the midpoint for the provider-switch seam's
`insert_message_before_last`). It is stored on the local entry map and never changes,
so a row's key is stable for its whole life.

Rows are upserts. Each write takes the next `rev` (a room-wide counter, indexed), which
is the change feed readers resume from.

### When the host persists

The host's publisher watches the transcript doc and classifies each changed part:

| Part state | Stored | Live |
|---|---|---|
| last text/reasoning part of a streaming entry | every 15 s if changed (durability) | `append` frames each commit |
| running tool (unresolved) | once when it appears | `part` frames as its output tail grows |
| anything else (settled text, resolved tool, input, error, image…) | on every change | — |
| entry header | on first sight and when status/duration change | — |

A text part settles when a later part appears or the entry leaves `streaming`. So a
turn costs about one row per part plus a header or two, regardless of how long it
streamed. Stored writes go through a durable local outbox keyed by `(doc, ord, idx)`
that coalesces to the latest bytes, and are POSTed in batches. A crash replays the
outbox; a dropped live frame only delays text until the part settles.

### Live frames

`live{entry, ord, idx, op}`: `op: "part"` carries the full part JSON; `op: "append"`
carries `at` (the UTF-8 length the sender had before) and the appended text. The room
relays them to other sockets and keeps the current open parts in memory (bounded) so a
viewer who joins mid-stream gets them after its backfill. Nothing is written. A stored
row for the same key drops the live copy. A reader applies an `append` only when `at`
matches its local length; otherwise it waits for the next `part` frame or stored row.

### Pagination

`GET /chat3/{id}/items?limit=N[&before=ord:idx]` returns the newest `N` rows before the
cursor in ascending order, plus the header of a partially included first entry, and
`more` when older rows exist. `?since=rev` returns the change feed. A fresh reader paints
from one page; older pages load when the transcript asks for them (`LoadEarlierMessages`
on desktop, `load_earlier` on mobile). A reader keeps `partBase` on an entry it only
holds the tail of, so partially loaded entries render their loaded parts. Updates to
rows older than the loaded window are ignored; paging returns their current version.

A host that does not hold the whole transcript (a moved or restored host) pages
everything in before it runs anything that needs history (resume, fork, provider
switch replay).

### Tool outputs

- Harnesses now emit output: claude reads `tool_result` content, codex reads
  `aggregatedOutput` and streams `item/commandExecution/outputDelta` as the new
  `AgentEvent::ToolOutputDelta`. Output is capped at `TOOL_OUTPUT_CAP` (256 KiB,
  head and tail kept) at the harness.
- gen 2 is unchanged: the fold still keeps outputs out of the doc, and output deltas
  are not journaled.
- gen 3: while a tool runs, its part carries the last lines of output (local doc only,
  streamed as `part` live frames). On `ToolResult` the host PUTs the full text to
  `/chat3/{id}/outputs/{partId}` and the part settles with the one-line summary,
  `outputBytes` and `outputRef = chat3:{chatId}/{partId}`. The desktop's existing
  "Show full output (N KB)" affordance fetches it through `FetchToolBlob`, which routes
  `chat3:` refs to the room.

Outputs live in the user's owner-only room, the same trust boundary as the transcript.
Tool *inputs* keep their existing `sanitize_tool_call` strip.

## Flag and rollout

- `preferences/labs` registry row, field `chat3: bool`. It syncs to every device of the
  user. Only chat minting reads it: a new chat is born gen 3 when it is on. Turning it
  off affects new chats only; gen-3 chats stay gen 3.
- **Old apps read `roomGen >= 2` as chat2.** This release teaches every client the exact
  generations it knows and shows "update to view" for newer ones. Enable the flag for a
  user only once all their apps run this release, or an old app will dial an empty chat2
  room for a gen-3 chat.
- No migration of existing chats in this change. gen-2 chats keep working untouched.

## Edge

Same `ChatRoom` class and `CHAT_ROOMS` namespace (no new DO migration); a gen-3 room is
named `chat3/{chatId}`. The chat2 surface (`ws`, `rows`, `checkpoint`, `stats`, `reset`)
is served unchanged on `/chat3/...` for the state log. New:

| Route / frame | What |
|---|---|
| `POST /items` | host batch upsert (framed `itemPut`s); relays `item` to sockets |
| `GET /items?limit&before` / `?since` | page / change feed (framed `item`s + `itemsDone`) |
| `PUT/GET /outputs/{partId}` | full tool output, chunked like blobs |
| WS `live` (0x11) | relay + in-memory open parts; never stored |
| WS `item` (0x12), `itemsDone` (0x13) | server → client |

Tables (`WITHOUT ROWID` so the key is the table, no second b-tree):
`items(ord REAL, idx INTEGER, entry TEXT, rev INTEGER, bytes BLOB, PRIMARY KEY(ord, idx))`
with `items_rev ON items(rev)`, and `outputs(part TEXT, chunk INTEGER, bytes BLOB,
PRIMARY KEY(part, chunk))`.

Both generations meter their own SQL (`rowsWritten`/`rowsRead` from each cursor, the
numbers Cloudflare bills) and report them in `/stats`.

## Proving the cost

`edge/test/workerd/chat3-cost.workerd.test.ts` replays the same recorded turn traces
into a gen-2 and a gen-3 room on real DO SQLite and compares the metered rows written,
stored bytes, and cold-open bytes. The traces come from
`crates/engine/examples/chat3_trace.rs`, which runs a real run journal through the real
fold, `SegmentWriter` and gen-3 publisher on a virtual clock. Results are recorded in
"Measured" below.

## Measured

_Filled in from the benchmark when it runs._
