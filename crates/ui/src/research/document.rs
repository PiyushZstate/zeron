//! A rollout read as a document (docs/research-view.md, "Trace as a document"), drawn
//! in the locked boards' parts: a page head naming the task, its turns as ruled rows
//! with the chosen one marked as a table's chosen row, code and tool output in the
//! board's insets, and what scored it in the detail column. The system prompt,
//! reasoning and long tool output start folded. The host sends one rollout's document
//! on asking; a run's file can be gigabytes, so `Runs` never carries them. An episode
//! line is written whole once it ends, so a document needs no refresh.

use std::collections::{BTreeMap, BTreeSet};

use gpui::{AnyElement, Context, Div, FontWeight, ScrollHandle, Stateful, div, prelude::*, px};
use seamlessh_runs::tamper::{Suspicion, suspicions};
use seamlessh_runs::{Document, Role, Rollout, Turn};

use super::ResearchView;
use super::style::{self as theme, DIM, EMBER, INK, INK_2, MONO, SOFT, white};

use super::screen::{
    TABLE_PAD, accent_edge, aside, aside_actions, aside_head, aside_section, body, button,
    dot_note, fact_grid, page_head, quiet, section_head,
};
use super::{RunKey, count, outcome_status, reward};

/// Lines a long text shows until it is opened.
const FOLD_LINES: usize = 12;
/// A system prompt is long and seldom what is being read, so it folds sooner.
const FOLD_SYSTEM: usize = 3;
/// Characters a folded text shows at most, for output that is one enormous line.
const FOLD_CHARS: usize = 2_000;
/// Rows above the first turn on the page: its head and the trace's head.
const LEAD_ROWS: usize = 2;

pub(super) enum Loaded {
    Asking,
    Ready(Document),
    Failed(String),
}

/// The rollout being read: where it is, its summary from the run, the document once
/// the host sends it, and the reader's place in it.
pub(super) struct Reading {
    pub(super) run: RunKey,
    pub(super) rollout: Rollout,
    pub(super) loaded: Loaded,
    /// The turn the keys act on.
    pub(super) at: usize,
    /// Turns opened past their fold.
    pub(super) open: BTreeSet<usize>,
    pub(super) scroll: ScrollHandle,
}

impl Reading {
    pub(super) fn new(run: RunKey, rollout: Rollout) -> Self {
        Self {
            run,
            rollout,
            loaded: Loaded::Asking,
            at: 0,
            open: BTreeSet::new(),
            scroll: ScrollHandle::new(),
        }
    }

    /// Whether the host's answer is for this rollout.
    pub(super) fn is(&self, place: super::PlaceId, path: &str, rollout: &str) -> bool {
        self.run.place == place && self.run.path == path && self.rollout.id == rollout
    }

    pub(super) fn heard(&mut self, document: Option<Document>, error: Option<String>) {
        self.loaded = match (document, error) {
            (Some(doc), _) => Loaded::Ready(doc),
            (None, Some(why)) => Loaded::Failed(why),
            (None, None) => Loaded::Failed("The host sent no document.".to_owned()),
        };
        self.at = 0;
        self.open.clear();
    }

    fn turns(&self) -> usize {
        match &self.loaded {
            Loaded::Ready(doc) => doc.turns.len(),
            _ => 0,
        }
    }

    /// Move to another turn and bring it into view.
    pub(super) fn step(&mut self, by: isize) {
        let n = self.turns();
        if n == 0 {
            return;
        }
        self.at = super::matrix::step_index(self.at, by, n);
        // The page's first children are its head and the trace's head.
        self.scroll.scroll_to_item(self.at + LEAD_ROWS);
    }

    /// Open the turn past its fold, or fold it again.
    pub(super) fn toggle(&mut self, at: usize) {
        if !self.open.remove(&at) {
            self.open.insert(at);
        }
    }
}

/// `text` cut to `lines` lines and [`FOLD_CHARS`], with the words for what was left
/// out; the whole text and nothing when it fits.
fn folded(text: &str, lines: usize) -> (String, Option<String>) {
    let total = text.lines().count();
    let mut shown = text.lines().take(lines).collect::<Vec<_>>().join("\n");
    let mut cut = total > lines;
    if let Some((at, _)) = shown.char_indices().nth(FOLD_CHARS) {
        shown.truncate(at);
        cut = true;
    }
    if !cut {
        return (text.to_owned(), None);
    }
    let more = match total.saturating_sub(lines) {
        0 => "the rest of the line".to_owned(),
        1 => "1 more line".to_owned(),
        n => format!("{n} more lines"),
    };
    (shown, Some(more))
}

/// A turn's fold line: what is hidden, and the key that shows it on the chosen turn.
fn fold_line(more: &str, chosen: bool) -> Div {
    quiet(if chosen {
        format!("… {more} · ↩ show")
    } else {
        format!("… {more}")
    })
}

fn role_word(role: Role) -> &'static str {
    match role {
        Role::System => "System",
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::Tool => "Tool",
        Role::Log => "Log",
    }
}

/// Code and tool output in the boards' inset.
fn code_block(text: String) -> Div {
    super::style::code(text, INK_2)
}

fn quiet_prose(text: String) -> Div {
    body(text, DIM)
}

fn plain_prose(text: String) -> Div {
    body(text, INK_2)
}

fn bright_prose(text: String) -> Div {
    body(text, INK)
}

/// A text with its fold: shown whole when open or short, else cut with a fold line.
fn foldable(text: &str, lines: usize, open: bool, chosen: bool, draw: fn(String) -> Div) -> Div {
    let (shown, more) = if open {
        (text.to_owned(), None)
    } else {
        folded(text, lines)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(draw(shown))
        .children(more.map(|m| fold_line(&m, chosen)))
}

/// One turn's body, by role. Reasoning shows as one line until the turn is opened.
fn turn_body(turn: &Turn, open: bool, chosen: bool) -> Div {
    let out = div().flex().flex_col().gap(px(8.));
    match turn.role {
        Role::System => out.child(foldable(&turn.text, FOLD_SYSTEM, open, chosen, quiet_prose)),
        Role::User => out.child(foldable(&turn.text, FOLD_LINES, open, chosen, plain_prose)),
        Role::Tool | Role::Log => {
            let text = if turn.text.is_empty() {
                "(no output)".to_owned()
            } else {
                turn.text.clone()
            };
            out.child(foldable(&text, FOLD_LINES, open, chosen, code_block))
        }
        Role::Assistant => {
            let reasoning = turn.reasoning.as_deref().map(|r| {
                if open {
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(quiet("Reasoning"))
                        .child(body(r.to_owned(), DIM))
                } else {
                    let words = match r.lines().count() {
                        1 => "Reasoning · 1 line".to_owned(),
                        n => format!("Reasoning · {n} lines"),
                    };
                    quiet(if chosen {
                        format!("{words} · ↩ show")
                    } else {
                        words
                    })
                }
            });
            // An assistant's own words are what is being read: never folded.
            let words = (!turn.text.is_empty()).then(|| bright_prose(turn.text.clone()));
            let calls = turn.tool_calls.iter().map(|c| {
                let call = if c.arguments.is_empty() {
                    format!("{}()", c.name)
                } else {
                    format!("{}({})", c.name, c.arguments)
                };
                foldable(&call, FOLD_LINES, open, chosen, code_block)
            });
            out.children(reasoning).children(words).children(calls)
        }
    }
}

/// Whether opening the turn would show more than it does folded.
fn has_fold(turn: &Turn) -> bool {
    let cut = |text: &str, lines| folded(text, lines).1.is_some();
    match turn.role {
        Role::System => cut(&turn.text, FOLD_SYSTEM),
        Role::User | Role::Tool | Role::Log => cut(&turn.text, FOLD_LINES),
        Role::Assistant => {
            turn.reasoning.is_some()
                || turn
                    .tool_calls
                    .iter()
                    .any(|c| cut(&c.arguments, FOLD_LINES))
        }
    }
}

/// Tool results name the call they answer by id; the call's name is in an earlier turn.
fn call_names(turns: &[Turn]) -> BTreeMap<&str, &str> {
    turns
        .iter()
        .flat_map(|t| t.tool_calls.iter())
        .map(|c| (c.id.as_str(), c.name.as_str()))
        .collect()
}

pub(super) fn seconds(s: f64) -> String {
    if s < 60. {
        format!("{s:.1}s")
    } else {
        format!("{}m {:02}s", (s / 60.).floor(), (s % 60.).floor())
    }
}

/// A ruled line of a list in the detail column: a name and its value in monospace.
fn value_row(name: String, value: String, color: u32) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(7.))
        .border_t_1()
        .border_color(white(0.06))
        .child(body(name, INK_2).flex_1().min_w_0().truncate())
        .child(body(value, color).font_family(MONO))
}

impl ResearchView {
    /// The document page's rows, its head and the trace's head first, so the page can
    /// scroll a turn into view by its index; and its column of scores beside.
    pub(super) fn document_page(
        &self,
        reading: &Reading,
        cx: &mut Context<Self>,
    ) -> (Vec<AnyElement>, Div) {
        let r = &reading.rollout;
        let run = self.reading_run_name(reading);
        let title = if r.task.label.is_empty() {
            r.task.key.clone()
        } else {
            r.task.label.clone()
        };
        let back =
            button("research-doc-back", "Back", false).on_click(cx.listener(|this, _, _, cx| {
                this.ui.reading = None;
                cx.notify();
            }));
        let head = div()
            .pb(px(24.))
            .child(page_head(
                title,
                format!("{run} · {}", r.id),
                false,
                vec![back.into_any_element()],
            ))
            .into_any_element();
        let turns = match &reading.loaded {
            Loaded::Ready(doc) => doc.turns.len(),
            _ => 0,
        };
        let trace_head = div()
            .pb(px(12.))
            .child(section_head(
                "Trace",
                None,
                Some(match turns {
                    0 => String::new(),
                    1 => "1 turn".to_owned(),
                    n => format!("{n} turns"),
                }),
            ))
            .into_any_element();
        let mut rows = vec![head, trace_head];
        match &reading.loaded {
            Loaded::Asking => rows.push(body("Reading the rollout…", DIM).into_any_element()),
            Loaded::Failed(why) => rows.push(body(why.clone(), EMBER).into_any_element()),
            Loaded::Ready(doc) if doc.turns.is_empty() => {
                rows.push(body("This rollout recorded no messages.", DIM).into_any_element())
            }
            Loaded::Ready(doc) => {
                let names = call_names(&doc.turns);
                let flagged = suspicions(doc);
                for (i, turn) in doc.turns.iter().enumerate() {
                    let why = flagged.iter().find(|s| s.turn == i).map(|s| s.why.clone());
                    rows.push(
                        self.turn_row(i, turn, reading, &names, why, cx)
                            .into_any_element(),
                    );
                }
                rows.push(
                    div()
                        .pt(px(12.))
                        .border_t_1()
                        .border_color(white(0.06))
                        .child(quiet("↑↓ turn · ↩ show or fold · ⎋ back"))
                        .into_any_element(),
                );
            }
        }
        (rows, self.document_scores(reading, cx))
    }

    fn reading_run_name(&self, reading: &Reading) -> String {
        self.ui
            .heard
            .get(&reading.run.place)
            .and_then(|h| h.runs.iter().find(|x| x.path == reading.run.path))
            .map_or_else(|| reading.run.path.clone(), super::run_name)
    }

    /// A turn as a ruled row: who spoke over what they said, the chosen one tinted
    /// with the accent's edge as a table's chosen row is.
    fn turn_row(
        &self,
        i: usize,
        turn: &Turn,
        reading: &Reading,
        names: &BTreeMap<&str, &str>,
        tamper: Option<String>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let chosen = reading.at == i;
        let open = reading.open.contains(&i);
        let name = match turn.role {
            Role::Tool => turn
                .tool_call_id
                .as_deref()
                .and_then(|id| names.get(id).copied())
                .or(turn.name.as_deref()),
            Role::Log => turn.name.as_deref(),
            _ => None,
        };
        let label = match name {
            Some(n) => format!("{} · {n}", role_word(turn.role)),
            None => role_word(turn.role).to_owned(),
        };
        let fold = has_fold(turn);
        div()
            .id(("research-turn", i as u64))
            .relative()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(TABLE_PAD))
            .py(px(12.))
            .border_t_1()
            .border_color(white(0.06))
            .when(chosen, |d| d.bg(theme::accent_tint()).child(accent_edge()))
            .when(!chosen, |d| d.hover(|s| s.bg(theme::row_hover())))
            .when(fold, |d| d.cursor_pointer())
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        body(label, if chosen { INK } else { SOFT })
                            .font_weight(FontWeight::MEDIUM)
                            .flex_1(),
                    )
                    .child(quiet(format!("{}", i + 1)).font_family(MONO)),
            )
            .children(tamper.map(|why| dot_note(EMBER, format!("May change its grader: {why}"))))
            .child(turn_body(turn, open, chosen))
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(reading) = &mut this.ui.reading {
                    reading.at = i;
                    if fold {
                        reading.toggle(i);
                    }
                }
                cx.notify();
            }))
    }

    /// What scored the rollout, as the board's detail column: its id and run, a line
    /// of how it ended, its facts, each reward part, metrics, timing and errors, and
    /// the way back at the foot. The summary's figures show while the document is on
    /// its way.
    fn document_scores(&self, reading: &Reading, cx: &mut Context<Self>) -> Div {
        let r = &reading.rollout;
        let (color, word) = outcome_status(r.outcome);
        let doc = match &reading.loaded {
            Loaded::Ready(doc) => Some(doc),
            _ => None,
        };
        let tokens = match (r.tokens.input, r.tokens.output) {
            (None, None) => "–".to_owned(),
            (i, o) => format!(
                "{} in · {} out",
                i.map_or_else(|| "–".to_owned(), count),
                o.map_or_else(|| "–".to_owned(), count)
            ),
        };
        let summary = {
            let mut line = format!("{word} with reward {}", reward(r.reward));
            if let Some(d) = doc {
                match d.turns.len() {
                    0 => {}
                    1 => line.push_str(" after 1 turn"),
                    n => line.push_str(&format!(" after {n} turns")),
                }
            }
            if let Some(stop) = &r.stop {
                line.push_str(&format!("; stopped on {stop}"));
            }
            line.push('.');
            line
        };
        let section = |title: &'static str, items: Vec<(String, String, u32)>| {
            (!items.is_empty()).then(|| {
                aside_section(title).child(
                    div()
                        .flex()
                        .flex_col()
                        .children(items.into_iter().map(|(k, v, c)| value_row(k, v, c))),
                )
            })
        };
        let rewards = doc.map_or(&r.rewards, |d| &d.rewards);
        let parts = rewards
            .iter()
            .map(|p| {
                let v = match p.weight {
                    Some(w) => format!("{:.3} × {w}", p.score),
                    None => format!("{:.3}", p.score),
                };
                (p.name.clone(), v, INK)
            })
            .collect();
        let metrics = doc.map_or_else(Vec::new, |d| {
            d.metrics
                .iter()
                .map(|(k, v)| (k.clone(), format!("{v:.3}"), INK))
                .collect()
        });
        let timing = doc.map_or_else(Vec::new, |d| {
            d.timing
                .iter()
                .map(|(k, v)| (k.clone(), seconds(*v), INK))
                .collect()
        });
        let errors: Vec<_> = match doc {
            Some(d) => d.errors.clone(),
            None => r.error.iter().cloned().collect(),
        };
        let notes = doc
            .map(|d| {
                let mut notes = Vec::new();
                match d.other_branches {
                    0 => {}
                    1 => notes.push("1 other branch, not shown: the last one is.".to_owned()),
                    n => notes.push(format!("{n} other branches, not shown: the last one is.")),
                }
                if let Some(n) = &d.notice {
                    notes.push(n.clone());
                }
                if !d.raw_keys.is_empty() {
                    notes.push(format!("Keys: {}", d.raw_keys.join(", ")));
                }
                notes
            })
            .unwrap_or_default();
        let tampering: Vec<Suspicion> = doc.map(suspicions).unwrap_or_default();
        let back = button("research-doc-back-foot", "Back to the cell", false).on_click(
            cx.listener(|this, _, _, cx| {
                this.ui.reading = None;
                cx.notify();
            }),
        );
        aside()
            .child(aside_head(
                r.id.clone(),
                format!(
                    "{} · {}",
                    self.reading_run_name(reading),
                    self.place_name(reading.run.place)
                ),
            ))
            .child(body(summary, INK_2))
            .child(fact_grid(
                Some((color, word)),
                vec![
                    ("Reward", reward(r.reward), INK),
                    (
                        "Stop",
                        r.stop.clone().unwrap_or_else(|| "–".to_owned()),
                        INK,
                    ),
                    (
                        "Duration",
                        r.duration_secs.map_or_else(|| "–".to_owned(), seconds),
                        INK,
                    ),
                    ("Tokens", tokens, INK_2),
                    (
                        "Cost",
                        r.cost
                            .map_or_else(|| "–".to_owned(), |c| format!("${c:.4}")),
                        INK_2,
                    ),
                ],
            ))
            // Before the rewards: if the grader was changed, they mean nothing.
            .when(!tampering.is_empty(), |a| {
                a.child(aside_section("Possible grader tampering").children(
                    tampering.into_iter().map(|s| {
                        dot_note(
                            EMBER,
                            format!("Turn {}: {} ({})", s.turn + 1, s.why, s.tool),
                        )
                    }),
                ))
            })
            .children(section("Rewards", parts))
            .children(section("Metrics", metrics))
            .children(section("Timing", timing))
            .when(!errors.is_empty(), |a| {
                a.child(
                    aside_section("Errors").children(errors.into_iter().map(|e| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(body(e.kind, EMBER).font_family(MONO))
                            .child(code_block(e.message))
                    })),
                )
            })
            .children(notes.into_iter().map(|n| dot_note(theme::WAITING, n)))
            .child(aside_actions(vec![back]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_whole_and_long_text_folds_with_a_count() {
        assert_eq!(folded("a\nb", 3), ("a\nb".to_owned(), None));
        let (shown, more) = folded("1\n2\n3\n4\n5", 2);
        assert_eq!(shown, "1\n2");
        assert_eq!(more.as_deref(), Some("3 more lines"));
        assert_eq!(folded("1\n2\n3", 2).1.as_deref(), Some("1 more line"));
    }

    #[test]
    fn one_enormous_line_folds_by_characters() {
        let line = "x".repeat(FOLD_CHARS + 50);
        let (shown, more) = folded(&line, FOLD_LINES);
        assert_eq!(shown.chars().count(), FOLD_CHARS);
        assert_eq!(more.as_deref(), Some("the rest of the line"));
    }

    #[test]
    fn tool_results_take_their_calls_name() {
        let call = Turn {
            role: Role::Assistant,
            text: String::new(),
            reasoning: Some("think".into()),
            tool_calls: vec![seamlessh_runs::ToolCall {
                id: "c1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            }],
            tool_call_id: None,
            name: None,
        };
        let turns = [call];
        assert_eq!(call_names(&turns).get("c1"), Some(&"bash"));
        assert!(has_fold(&turns[0]), "reasoning always folds");
    }
}
