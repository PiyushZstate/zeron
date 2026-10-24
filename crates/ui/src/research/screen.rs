//! The Research screen's frame, drawn to the locked boards (`plain-Home` and
//! `plain-RLEnv`, docs/research-view.md): a sidebar of pages, the machine and the
//! places that hold runs; one opaque main panel with a page head, a row of stat cells,
//! and ruled tables whose chosen row takes the accent's tint and a 2 px edge; and, when
//! something is chosen, a detail column split off by a hairline. The Runs page lives
//! here too; the matrix is `matrix.rs` and a rollout's document `document.rs`.
//!
//! The board's measures are kept as constants here rather than rounded into the kit's,
//! so a side-by-side check reads the same numbers.

use gpui::{
    AnyElement, Context, Div, FontWeight, SharedString, Stateful, div, prelude::*, px, rgb,
};
use zeron_engine::research::ResearchRun;

use super::PlaceId;
use seamlessh_runs::{Comparison, Framework, Outcome, TaskRow, compare};

use super::ResearchView;
use super::style::{
    self as theme, DIM, EMBER, INK, INK_2, MIST, MONO, SOFT, WAITING, WORKING, meta, white,
};
use super::style::{self as kit, Kind};
use crate::icons::{self, icon};

use super::tally::{Tally, tally};
use super::{Page, RunKey, count, delta, reward, run_name};

/// A table's first cell sits this far in, so a chosen row's tint reaches past its words
/// (the board's `td:first-child { padding-left: 10px }`).
pub(super) const TABLE_PAD: f32 = 10.;
/// The room after each cell (`td { padding-right: 20px }`).
const CELL_GAP: f32 = 20.;
/// A row's padding above and below its words (`td { padding: 10px 0 }`).
const ROW_PAD: f32 = 10.;
const ASIDE_W: f32 = 360.;
const CELL: f32 = 9.;
/// The board's body type: 13 px on 20.
const BODY: f32 = 13.;
const BODY_LINE: f32 = 20.;
/// The board's raised grey: buttons and the chosen tab.
const RAISED: u32 = 0x1f1f22;

/// Body text in the board's size.
pub(super) fn body(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .text_size(px(BODY))
        .line_height(px(BODY_LINE))
        .text_color(rgb(color))
        .child(text.into())
}

/// The board's quiet line: 12 on 16 in the dim grey.
pub(super) fn quiet(text: impl Into<SharedString>) -> Div {
    meta(text, DIM)
}

/// A page's head: an 18 px title, a quiet line beside it, and its actions at the right.
/// `mono` sets the title in monospace, as the board names a thing (`sql-repair-v2`).
pub(super) fn page_head(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    mono: bool,
    actions: Vec<AnyElement>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex_none()
                .text_size(px(if mono { 16. } else { 18. }))
                .line_height(px(if mono { 24. } else { 26. }))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK))
                .when(mono, |t| t.font_family(MONO))
                .child(title.into()),
        )
        .child(body(subtitle, DIM).min_w_0().truncate())
        .child(div().flex_1())
        .child(div().flex_none().flex().gap(px(8.)).children(actions))
}

/// A section's head: a 15 px title, what sits beside it (tabs), and a quiet note at
/// the right.
pub(super) fn section_head(
    title: impl Into<SharedString>,
    beside: Option<Div>,
    note: Option<String>,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            div()
                .mr(px(8.))
                .text_size(px(15.))
                .line_height(px(22.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(INK))
                .child(title.into()),
        )
        .children(beside)
        .child(div().flex_1())
        .children(note.map(quiet))
}

/// The board's button: 30 tall, 13 px medium words, the raised grey or the blue.
pub(super) fn button(
    id: impl Into<gpui::ElementId>,
    words: &'static str,
    primary: bool,
) -> Stateful<Div> {
    let b = kit::button(
        id,
        words,
        if primary {
            Kind::Primary
        } else {
            Kind::Secondary
        },
    )
    .h(px(30.))
    .text_size(px(BODY))
    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
    if primary { b } else { b.bg(rgb(RAISED)) }
}

/// A row of stat cells, each an equal share of the width.
pub(super) fn stats(cells: Vec<(&'static str, String)>) -> Div {
    div().flex().gap(px(8.)).children(
        cells
            .into_iter()
            .map(|(name, figure)| kit::stat(name, figure).flex_1().rounded(px(8.))),
    )
}

/// A table's header cell, `w` wide or the rest of the row.
pub(super) fn th(words: impl Into<SharedString>, w: Option<f32>, first: bool) -> Div {
    cell_box(w, first).pb(px(8.)).child(quiet(words).truncate())
}

/// A table cell's box: its width, the room after it, and the first cell's inset.
pub(super) fn cell_box(w: Option<f32>, first: bool) -> Div {
    let d = div().pr(px(CELL_GAP)).when(first, |d| d.pl(px(TABLE_PAD)));
    match w {
        Some(w) => d.w(px(w)).flex_none(),
        None => d.flex_1().min_w_0(),
    }
}

/// A table row: ruled above, the board's padding, and the selection's tint and edge.
pub(super) fn table_row(id: impl Into<gpui::ElementId>, chosen: bool) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .py(px(ROW_PAD))
        .border_t_1()
        .border_color(white(0.06))
        .cursor_pointer()
        .when(chosen, |r| r.bg(theme::accent_tint()).child(accent_edge()))
        .when(!chosen, |r| r.hover(|s| s.bg(theme::row_hover())))
}

/// A name in a table's first column: monospace, the brightest ink.
pub(super) fn name_cell(text: impl Into<SharedString>) -> Div {
    body(text, INK).font_family(MONO).truncate()
}

/// The locked design's selection edge: 2 px of the accent down a chosen row's left
/// side. It is its own strip, since a GPUI border takes one colour on every side and
/// would turn the row's rule above accent too. The row must be `relative`.
pub(super) fn accent_edge() -> Div {
    div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(px(2.))
        .bg(rgb(theme::ACCENT))
}

/// The detail column: split off the panel by a hairline on its left.
pub(super) fn aside() -> Div {
    div()
        .w(px(ASIDE_W))
        .flex_none()
        .min_h_full()
        .flex()
        .flex_col()
        .gap(px(20.))
        .px(px(20.))
        .py(px(24.))
        .border_l_1()
        .border_color(white(0.06))
}

/// The detail column's head: the thing's name in monospace and a quiet line under it.
pub(super) fn aside_head(name: impl Into<SharedString>, under: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            body(name, INK)
                .font_family(MONO)
                .font_weight(FontWeight::MEDIUM)
                .truncate(),
        )
        .child(quiet(under).truncate())
}

/// A titled part of the detail column: the title at 600, then what it holds.
pub(super) fn aside_section(title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(body(title, INK).font_weight(FontWeight::SEMIBOLD))
}

/// The detail column's actions, at its foot, sharing the width.
pub(super) fn aside_actions(buttons: Vec<Stateful<Div>>) -> Div {
    div()
        .mt_auto()
        .pt(px(4.))
        .flex()
        .gap(px(8.))
        .children(buttons.into_iter().map(|b| b.flex_1().h(px(32.))))
}

/// Facts two to a line, each a quiet key over its value (the board's machine
/// section): a status first, when given, as a dot and a word, then values in
/// monospace.
pub(super) fn fact_grid(
    status: Option<(u32, &'static str)>,
    facts: Vec<(&'static str, String, u32)>,
) -> Div {
    let cell = |key: &'static str, value: Div| {
        div()
            .w(gpui::relative(0.5))
            .pr(px(12.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .min_w_0()
            .child(quiet(key))
            .child(value)
    };
    div()
        .flex()
        .flex_wrap()
        .gap_y(px(16.))
        .children(status.map(|(color, word)| cell("Status", kit::status(color, word))))
        .children(
            facts.into_iter().map(|(key, value, color)| {
                cell(key, body(value, color).font_family(MONO).truncate())
            }),
        )
}

/// What a set of rollouts adds up to, as two parts of a detail column: each reward's
/// mean raw score with its weight, then how the rollouts ended. A part with nothing to
/// list is left out, so a run without rewards shows no empty heading.
pub(super) fn tally_parts(t: &Tally) -> Vec<Div> {
    let line = || {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .py(px(6.))
            .border_t_1()
            .border_color(white(0.06))
    };
    let figure = |text: String, color: u32| {
        div()
            .w(px(56.))
            .flex_none()
            .flex()
            .justify_end()
            .child(body(text, color).font_family(MONO))
    };
    let mut parts = Vec::new();
    if !t.rewards.is_empty() {
        parts.push(
            aside_section("Rewards").child(div().flex().flex_col().children(t.rewards.iter().map(
                |m| {
                    line()
                        .child(body(m.name.clone(), INK_2).font_family(MONO).truncate())
                        .child(div().flex_1())
                        .children(m.weight.map(|w| quiet(format!("× {w}")).flex_none()))
                        .child(figure(reward(Some(m.mean)), INK))
                },
            ))),
        );
    }
    if !t.endings.is_empty() {
        parts.push(
            aside_section("How they ended").child(div().flex().flex_col().children(
                t.endings.iter().map(|e| {
                    line()
                        .child(kit::dot(if e.error { EMBER } else { MIST }))
                        .child(body(e.word.clone(), INK_2).font_family(MONO).truncate())
                        .child(div().flex_1())
                        .child(figure(
                            e.count.to_string(),
                            if e.error { EMBER } else { INK },
                        ))
                }),
            )),
        );
    }
    parts
}

/// A line with a status dot, for notes about what was read.
pub(super) fn dot_note(color: u32, text: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .child(kit::dot(color))
        .child(body(text, DIM))
}

/// A run's state as a dot and a word.
fn run_state(run: &ResearchRun) -> (u32, &'static str) {
    if run.live {
        (WORKING, "Writing")
    } else if run.summary.facts.ended == Some(true) {
        (MIST, "Finished")
    } else if run.summary.errored > 0 {
        // Nothing is writing it and it never recorded an end: it stopped early.
        (EMBER, "Stopped")
    } else {
        (MIST, "Stopped")
    }
}

fn framework(run: &ResearchRun) -> &'static str {
    match run.summary.framework {
        Framework::Verifiers => "verifiers",
        Framework::Harbor => "Harbor",
    }
}

/// A rollout's square in a task row: green as bright as its reward, ember for an error,
/// an outline for one still running.
fn cell_ink(outcome: Outcome, value: Option<f64>) -> (u32, bool) {
    match (outcome, value) {
        (Outcome::Errored, _) => (EMBER, true),
        (Outcome::Unfinished, _) => (MIST, false),
        (_, None) => (0x3a3a3a, true),
        (_, Some(v)) if v >= 0.999 => (WORKING, true),
        (_, Some(v)) if v > 0. => (0x2f6e51, true),
        (_, Some(_)) => (0x4a4a46, true),
    }
}

/// A place in the Research sidebar: its id, its name, and how many runs it has.
type PlaceRow = (PlaceId, String, usize);

impl ResearchView {
    /// The screen's own column, as the board's sidebar: its pages, then the folders that
    /// hold runs under their project's name. Zeron's sidebar stays beside it, so the
    /// board's machine line and its way back to the coding view are left to Zeron.
    pub(crate) fn research_sidebar(&self, cx: &mut Context<Self>) -> Div {
        let ui_state = &self.ui;
        // The board's `.nav`: 30 tall, 10 in, a 16 px glyph, lit white when on.
        let nav = |id: &'static str, glyph: &'static str, name: &'static str, on: bool| {
            div()
                .id(id)
                .flex_none()
                .min_h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(theme::R_ROW))
                .cursor_pointer()
                .when(on, |r| r.bg(white(0.1)))
                .when(!on, |r| r.hover(|s| s.bg(white(0.06))))
                .child(
                    icon(glyph)
                        .size(px(16.))
                        .text_color(rgb(if on { INK } else { SOFT })),
                )
                .child(body(name, if on { INK } else { SOFT }).flex_1())
        };
        let page = |id, glyph, name, to: Page| {
            let on = ui_state.page == to && ui_state.reading.is_none();
            nav(id, glyph, name, on).on_click(cx.listener(move |this, _, _, cx| {
                this.ui.page = to;
                this.ui.reading = None;
                cx.notify();
            }))
        };
        // The board's `.t`: 5 by 10, the words cut short, lit white when on.
        let place_row = |id: u64, name: String, n: usize, scope: Option<PlaceId>| {
            let on = ui_state.scope == scope;
            div()
                .id(("research-scope", id))
                .flex()
                .items_center()
                .gap(px(8.))
                .py(px(5.))
                .px(px(10.))
                .rounded(px(theme::R_ROW))
                .cursor_pointer()
                .when(on, |r| r.bg(white(0.1)))
                .when(!on, |r| r.hover(|s| s.bg(white(0.06))))
                .child(
                    body(name, if on { INK } else { SOFT })
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                )
                .child(meta(n.to_string(), DIM).font_family(MONO))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.set_scope(scope);
                    cx.notify();
                }))
        };
        let label = |words: String| div().px(px(10.)).pb(px(4.)).child(quiet(words));
        // Folders with runs, under their project's name, projects in first-seen order.
        let mut groups: Vec<(String, Vec<PlaceRow>)> = Vec::new();
        for (place, n) in ui_state.places_with_runs() {
            let project = self.place_project(place);
            let name = self.place_name(place);
            match groups.iter_mut().find(|(p, _)| *p == project) {
                Some((_, list)) => list.push((place, name, n)),
                None => groups.push((project, vec![(place, name, n)])),
            }
        }
        let total: usize = groups.iter().flat_map(|(_, l)| l.iter().map(|t| t.2)).sum();
        // A project whose one folder carries its name needs no heading repeating it:
        // that folder joins the list under All folders.
        let (lone, groups): (Vec<_>, Vec<_>) = groups.into_iter().partition(
            |(project, places)| matches!(places.as_slice(), [(_, name, _)] if name == project),
        );
        let lone: Vec<PlaceRow> = lone.into_iter().flat_map(|(_, places)| places).collect();
        div()
            .w(px(232.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(20.))
            .px(px(4.))
            .pt(px(8.))
            .pb(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(page(
                        "research-matrix-page",
                        icons::SPLIT_COLUMNS,
                        "Matrix",
                        Page::Matrix,
                    ))
                    .child(page("research-runs-page", icons::LIST, "Runs", Page::Runs))
                    .child(page(
                        "research-checks-page",
                        icons::CHECKLIST,
                        "Checks",
                        Page::Checks,
                    )),
            )
            .child(
                div()
                    .id("research-places")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(20.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().px(px(10.)).pb(px(4.)).child(quiet("Folders")))
                            .child(place_row(0, "All folders".to_owned(), total, None))
                            .children(lone.into_iter().map(|(place, name, n)| {
                                place_row(place.0.saturating_add(1), name, n, Some(place))
                            })),
                    )
                    .children(groups.into_iter().map(|(project, places)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(label(project))
                            .children(places.into_iter().map(|(place, name, n)| {
                                place_row(place.0.saturating_add(1), name, n, Some(place))
                            }))
                    })),
            )
    }

    /// The main panel: the page, and its detail split off to the right. With a detail
    /// the page is a little tighter, as the board's split one is.
    pub(crate) fn research_panel(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        // A document's rows are the scrolling page's own children, so the page can bring
        // a turn into view by its index.
        let (page, detail) = match &self.ui.reading {
            Some(reading) => {
                let (rows, scores) = self.document_page(reading, cx);
                let page = div()
                    .id("research-page")
                    .track_scroll(&reading.scroll)
                    .flex()
                    .flex_col()
                    .children(rows);
                (page, Some(scores))
            }
            None => {
                let (section, detail) = match self.ui.page {
                    Page::Matrix => self.matrix_page(cx),
                    Page::Runs => self.runs_page(cx),
                    Page::Checks => self.checks_page(cx),
                };
                (div().id("research-page").child(section), detail)
            }
        };
        let split = detail.is_some();
        div()
            .id("research-panel")
            .size_full()
            .flex()
            .rounded(px(theme::R_PANEL))
            .border_1()
            .border_color(theme::panel_ring())
            .bg(rgb(theme::GROUND))
            .overflow_hidden()
            .child(
                page.flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .px(px(if split { 28. } else { 32. }))
                    .pt(px(24.))
                    .pb(px(if split { 28. } else { 32. })),
            )
            .children(detail.map(|d| {
                div()
                    .id("research-detail")
                    .flex_none()
                    .h_full()
                    .overflow_y_scroll()
                    .child(d)
            }))
    }

    /// Every run in scope newest first, the chosen one beside the list.
    fn runs_page(&self, cx: &mut Context<Self>) -> (Div, Option<Div>) {
        let ui_state = &self.ui;
        let runs = ui_state.runs();
        let chosen = ui_state.chosen_run();
        let baseline = ui_state.baseline_run();
        let live = runs.iter().filter(|(_, r)| r.live).count();
        let rollouts: u32 = runs.iter().map(|(_, r)| r.summary.rollouts).sum();
        let errored: u32 = runs.iter().map(|(_, r)| r.summary.errored).sum();
        let scope = match ui_state.scope {
            Some(place) => self.place_name(place),
            None => "all folders".to_owned(),
        };
        let errors: Vec<String> = ui_state
            .heard
            .iter()
            .filter(|(t, _)| ui_state.scope.is_none_or(|s| s == **t))
            .filter_map(|(t, h)| Some(format!("{}: {}", self.place_name(*t), h.error.as_ref()?)))
            .collect();
        let to_matrix = button("research-to-matrix", "Matrix", false).on_click(cx.listener(
            |this, _, _, cx| {
                this.ui.page = Page::Matrix;
                cx.notify();
            },
        ));
        let section = div()
            .flex()
            .flex_col()
            .gap(px(if chosen.is_some() { 24. } else { 32. }))
            .child(page_head(
                "Runs",
                match runs.len() {
                    1 => format!("1 run in {scope}"),
                    n => format!("{n} runs in {scope}"),
                },
                false,
                vec![to_matrix.into_any_element()],
            ))
            .child(stats(vec![
                ("Runs", runs.len().to_string()),
                ("Writing", live.to_string()),
                ("Rollouts", rollouts.to_string()),
                ("Errors", errored.to_string()),
            ]))
            .children(
                errors
                    .into_iter()
                    .map(|e| dot_note(WAITING, format!("Could not read the worktree of {e}"))),
            );
        if runs.is_empty() {
            return (
                section.child(body(
                    "No runs yet. verifiers runs appear from outputs/*/traces.jsonl and Harbor \
                     jobs from jobs/*/ in a folder's worktree.",
                    DIM,
                )),
                None,
            );
        }
        // The board's columns: name and state, then what the run scored. Its framework
        // and place are in the detail column, which is always open beside the list.
        const W: [f32; 5] = [110., 64., 80., 72., 64.];
        let head = div()
            .flex()
            .child(th("Name", None, true).min_w(px(200.)))
            .child(th("Status", Some(W[0]), false))
            .child(th("Tasks", Some(W[1]), false))
            .child(th("Rollouts", Some(W[2]), false))
            .child(th("Mean", Some(W[3]), false))
            .child(th("Errors", Some(W[4]), false));
        let rows = runs.iter().enumerate().map(|(at, (place, run))| {
            let key = RunKey::of(*place, run);
            let on = chosen.is_some_and(|(t, c)| t == *place && c.path == run.path);
            let is_base = baseline.is_some_and(|(t, b)| t == *place && b.path == run.path);
            let (color, word) = run_state(run);
            let s = &run.summary;
            let mono = |text: String, color: u32| body(text, color).font_family(MONO);
            table_row(("research-run", at as u64), on)
                .child(
                    cell_box(None, true)
                        .min_w(px(200.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(name_cell(run_name(run)))
                        .when(is_base, |c| c.child(quiet("baseline").flex_none())),
                )
                .child(cell_box(Some(W[0]), false).child(kit::status(color, word)))
                .child(cell_box(Some(W[1]), false).child(mono(s.tasks.to_string(), INK_2)))
                .child(cell_box(Some(W[2]), false).child(mono(s.rollouts.to_string(), INK_2)))
                .child(cell_box(Some(W[3]), false).child(mono(reward(s.mean_reward), INK)))
                .child(cell_box(Some(W[4]), false).child(mono(
                    s.errored.to_string(),
                    if s.errored > 0 { EMBER } else { INK_2 },
                )))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ui.chosen = Some(key.clone());
                    cx.notify();
                }))
        });
        let section = section.child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(section_head(
                    "All runs",
                    None,
                    Some("Newest first".to_owned()),
                ))
                .child(
                    div()
                        .id("research-runs")
                        .w_full()
                        .overflow_x_scroll()
                        .child(
                            div()
                                .min_w(px(590.))
                                .flex()
                                .flex_col()
                                .child(head)
                                .children(rows),
                        ),
                )
                .child(quiet(format!(
                    "{} runs · ↑↓ choose · b baseline · , . folder · 1 2 3 page",
                    runs.len()
                ))),
        );
        let detail = chosen.map(|(place, run)| {
            let against = baseline.filter(|(bt, b)| {
                !(*bt == place && b.path == run.path)
                    && b.summary.framework == run.summary.framework
            });
            let is_base = baseline.is_some_and(|(bt, b)| bt == place && b.path == run.path);
            self.run_detail(place, run, against.map(|(_, b)| b), is_base, cx)
        });
        (section, detail)
    }

    /// The chosen run, as the board's detail column: its name and place, a line of what
    /// it scored, its facts, notes, a row per task (against the baseline when one is
    /// pinned), and pinning it as the baseline at the foot.
    fn run_detail(
        &self,
        place: PlaceId,
        run: &ResearchRun,
        baseline: Option<&ResearchRun>,
        is_base: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let s = &run.summary;
        let f = &s.facts;
        let version = match &f.framework_version {
            Some(v) => format!("{} {v}", framework(run)),
            None => framework(run).to_owned(),
        };
        let tokens = match (s.tokens.input, s.tokens.output) {
            (None, None) => "–".to_owned(),
            (i, o) => format!(
                "{} in · {} out",
                count(i.unwrap_or(0)),
                count(o.unwrap_or(0))
            ),
        };
        let mut notes: Vec<String> = f.notes.clone();
        if f.torn_tail {
            notes.push("Last line still being written".to_owned());
        }
        if f.skipped > 0 {
            notes.push(format!("{} unreadable, skipped", f.skipped));
        }
        if let Some(reported) = &f.reported
            && let Some(errored) = reported.errored
            && errored != s.errored
        {
            notes.push(format!(
                "Job reports {errored} errored; trial files show {}",
                s.errored
            ));
        }
        let comparison = baseline.map(|b| compare(&b.tasks, &run.tasks));
        let head_delta = baseline.and_then(|b| Some(s.mean_reward? - b.summary.mean_reward?));
        let none = || "–".to_owned();
        let (state_color, state_word) = run_state(run);
        let summary = {
            let many = |n: u32, word: &str| {
                if n == 1 {
                    format!("1 {word}")
                } else {
                    format!("{n} {word}s")
                }
            };
            let mut line = format!(
                "{} over {}",
                many(s.rollouts, "rollout"),
                many(s.tasks, "task")
            );
            match (s.mean_reward, s.errored) {
                (Some(mean), 0) => line.push_str(&format!(", mean reward {}.", reward(Some(mean)))),
                (Some(mean), n) => line.push_str(&format!(
                    ", mean reward {}; {n} errored, left out of the mean.",
                    reward(Some(mean))
                )),
                (None, 0) => line.push_str(", no reward yet."),
                (None, n) if n == s.rollouts => line.push_str(", all errored, so no mean."),
                (None, n) => line.push_str(&format!("; {n} errored, and no mean yet.")),
            }
            if let (Some(b), Some(d)) = (baseline, head_delta) {
                line.push_str(&format!(" {} against {}.", delta(d).0, run_name(b)));
            }
            line
        };
        let pin = button(
            "research-baseline",
            if is_base {
                "Stop comparing"
            } else {
                "Set as baseline"
            },
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_baseline();
            cx.notify();
        }));
        aside()
            .child(aside_head(
                run_name(run),
                format!("{} · {}", self.place_name(place), run.path),
            ))
            .child(body(summary, INK_2))
            .child(fact_grid(
                Some((state_color, state_word)),
                vec![
                    ("Framework", version, INK),
                    ("Env", f.env.clone().unwrap_or_else(none), INK),
                    ("Model", f.models.first().cloned().unwrap_or_else(none), INK),
                    ("Mean reward", reward(s.mean_reward), INK),
                    match head_delta {
                        Some(d) => {
                            let (words, ink) = delta(d);
                            ("vs baseline", words, ink)
                        }
                        None => ("Tasks", s.tasks.to_string(), INK),
                    },
                    ("Rollouts", s.rollouts.to_string(), INK),
                    (
                        "Errored",
                        if s.timed_out > 0 {
                            format!("{} · {} timed out", s.errored, s.timed_out)
                        } else {
                            s.errored.to_string()
                        },
                        if s.errored > 0 { EMBER } else { INK },
                    ),
                    ("Tokens", tokens, INK_2),
                    (
                        "Cost",
                        s.cost.map_or_else(none, |c| format!("${c:.2}")),
                        INK_2,
                    ),
                ],
            ))
            .children((!notes.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .children(notes.into_iter().map(|n| dot_note(WAITING, n)))
            }))
            .children(tally_parts(&tally(&run.rollouts)))
            .child(
                aside_section(match baseline {
                    Some(b) => format!("Tasks against {}", run_name(b)),
                    None => "Tasks".to_owned(),
                })
                .child(div().flex().flex_col().children(match &comparison {
                    Some(c) => compare_rows(c),
                    None => run.tasks.iter().map(|t| task_row(t, None)).collect(),
                })),
            )
            .child(aside_actions(vec![pin]))
    }
}

fn compare_rows(c: &Comparison) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = c
        .rows
        .iter()
        .filter_map(|row| match (&row.b, row.delta) {
            (Some(task), d) => Some(task_row(task, Some(d))),
            (None, _) => None,
        })
        .collect();
    if !c.same_tasks {
        out.push(
            div()
                .pt(px(8.))
                .child(dot_note(
                    WAITING,
                    format!(
                        "{} tasks only in the baseline, {} only here",
                        c.only_a, c.only_b
                    ),
                ))
                .into_any_element(),
        );
    }
    out
}

/// A task: its words, a square per rollout, and its mean, or the change against the
/// baseline when comparing (`Some(None)` when the baseline has no mean for it).
fn task_row(task: &TaskRow, vs: Option<Option<f64>>) -> AnyElement {
    let cells = task.rollouts.iter().map(|cell| {
        let (ink, filled) = cell_ink(cell.outcome, cell.reward);
        div()
            .size(px(CELL))
            .rounded(px(2.))
            .when(filled, |c| c.bg(rgb(ink)))
            .when(!filled, |c| c.border_1().border_color(rgb(ink)))
    });
    let right = match vs {
        Some(Some(d)) => {
            let (words, ink) = delta(d);
            body(words, ink)
        }
        Some(None) => body("new", DIM),
        None => body(reward(task.mean), INK),
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .py(px(8.))
        .border_t_1()
        .border_color(white(0.06))
        .child(
            body(task.task.label.clone(), INK_2)
                .flex_1()
                .min_w_0()
                .truncate(),
        )
        .child(div().flex().flex_none().gap(px(3.)).children(cells))
        .child(
            div()
                .w(px(56.))
                .flex_none()
                .flex()
                .justify_end()
                .child(right.font_family(MONO)),
        )
        .into_any_element()
}
