//! The run matrix, the Research screen's main page: every verifiers rollout of the
//! places in scope, one env at a time, tasks down and harness × model across. Harbor
//! runs stay on the Runs page (docs/research-view.md).

use gpui::{AnyElement, Context, Div, Stateful, div, prelude::*, px, rgb};
use zeron_engine::research::ResearchRun;

use super::PlaceId;
use seamlessh_runs::matrix::{self as runs_matrix, Axis, Matrix, MatrixSpec};
use seamlessh_runs::{Framework, Rollout, axis_text};

use super::ResearchView;
use super::style as kit;
use super::style::{self as theme, DIM, EMBER, INK, INK_2, MONO, white};

use super::document::seconds;
use super::screen::{
    aside, aside_actions, aside_head, aside_section, body, button, cell_box, fact_grid, name_cell,
    page_head, quiet, section_head, stats, table_row, tally_parts, th,
};
use super::tally::{ending, tally};
use super::{RunKey, outcome_status, reward, run_name};

/// A column is wide enough for a reward, its count and an error count side by side.
const COL_W: f32 = 150.;
/// The task column's width. Columns keep their widths and the row's spare room goes
/// after the last, so a figure stays beside its task as the board's table keeps it.
const TASK_W: f32 = 280.;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CellKey {
    pub row: String,
    pub column: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Choice {
    pub env: Option<String>,
    pub cell: Option<CellKey>,
    pub opened: bool,
    /// The opened cell's chosen rollout, by place in the cell; clamped where it is read,
    /// since a refresh can shrink the cell.
    pub pick: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Totals {
    pub mean: Option<f64>,
    pub rollouts: u32,
    pub errored: u32,
}

pub(super) struct Snapshot {
    pub rollouts: Vec<Rollout>,
    /// The run each rollout came from, index for index.
    pub origins: Vec<RunKey>,
    pub envs: Vec<String>,
    pub env: Option<String>,
    pub grid: Matrix,
}

/// Gather the verifiers rollouts of `runs`, keep the run each came from, and group by
/// the default axes filtered to `env` (or the first env seen).
pub(super) fn snapshot(runs: &[(PlaceId, &ResearchRun)], env: Option<&str>) -> Snapshot {
    let mut rollouts = Vec::new();
    let mut origins = Vec::new();
    for (place, run) in runs {
        if run.summary.framework != Framework::Verifiers {
            continue;
        }
        for r in &run.rollouts {
            rollouts.push(r.clone());
            origins.push(RunKey::of(*place, run));
        }
    }
    let mut envs: Vec<String> = Vec::new();
    for r in &rollouts {
        let env = axis_text(r.axes.env.as_deref());
        if !envs.iter().any(|e| e == env) {
            envs.push(env.to_owned());
        }
    }
    let env = env
        .filter(|e| envs.iter().any(|x| x == e))
        .map(str::to_owned)
        .or_else(|| envs.first().cloned());
    let mut spec = MatrixSpec::default();
    if let Some(env) = &env {
        spec.filters.push((Axis::Env, env.clone()));
    }
    let grid = runs_matrix::matrix(&rollouts, &spec);
    Snapshot {
        rollouts,
        origins,
        envs,
        env,
        grid,
    }
}

impl Snapshot {
    pub(super) fn contains(&self, key: &CellKey) -> bool {
        self.index(key).is_some()
    }

    pub(super) fn first_cell(&self) -> Option<CellKey> {
        Some(CellKey {
            row: self.grid.rows.first()?.clone(),
            column: self.grid.columns.first()?.clone(),
        })
    }

    pub(super) fn index(&self, key: &CellKey) -> Option<(usize, usize)> {
        let row = self.grid.rows.iter().position(|r| r == &key.row)?;
        let col = self.grid.columns.iter().position(|c| c == &key.column)?;
        Some((row, col))
    }

    /// Each column over every task: the mean of its rollouts that count (not the mean of
    /// its cells' means, so a task run more often weighs more, as in verifiers' own
    /// headline), with its rollouts and errors.
    pub(super) fn column_totals(&self) -> Vec<Totals> {
        (0..self.grid.columns.len())
            .map(|ci| {
                let cells: Vec<&runs_matrix::Cell> = (0..self.grid.rows.len())
                    .filter_map(|ri| self.grid.cell(ri, ci))
                    .collect();
                let counted: Vec<f64> = cells
                    .iter()
                    .flat_map(|c| c.members.iter())
                    .filter_map(|&i| self.rollouts.get(i))
                    .filter(|r| r.counts_in_mean())
                    .filter_map(|r| r.reward)
                    .collect();
                Totals {
                    mean: (!counted.is_empty())
                        .then(|| counted.iter().sum::<f64>() / counted.len() as f64),
                    rollouts: cells.iter().map(|c| c.rollouts).sum(),
                    errored: cells.iter().map(|c| c.errored).sum(),
                }
            })
            .collect()
    }

    /// A task's words: its label, else its key.
    fn row_label(&self, key: &str) -> String {
        self.rollouts
            .iter()
            .find(|r| r.task.key == key && !r.task.label.is_empty())
            .map_or_else(|| key.to_owned(), |r| r.task.label.clone())
    }
}

pub(super) fn step_index(at: usize, by: isize, len: usize) -> usize {
    at.saturating_add_signed(by).min(len.saturating_sub(1))
}

pub(super) fn wrap_index(at: usize, by: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    // why: lengths are list sizes on screen, far below isize::MAX, so the casts hold.
    (at as isize + by).rem_euclid(len as isize) as usize
}

impl ResearchView {
    /// The matrix page, laid out as the board's overview: its head, a row of totals,
    /// then the table under its own head with the envs as tabs; the opened cell is the
    /// detail column.
    pub(super) fn matrix_page(&self, cx: &mut Context<Self>) -> (Div, Option<Div>) {
        let ui_state = &self.ui;
        let snap = ui_state.snapshot();
        let choice = &ui_state.matrix;
        let runs = ui_state
            .runs()
            .iter()
            .filter(|(_, r)| r.summary.framework == Framework::Verifiers)
            .count();
        let places = snap
            .origins
            .iter()
            .map(|o| o.place)
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let subtitle = match (runs, places) {
            (0, _) => "No verifiers runs".to_owned(),
            (1, _) => "1 verifiers run".to_owned(),
            (n, 1) => format!("{n} verifiers runs in 1 folder"),
            (n, t) => format!("{n} verifiers runs in {t} folders"),
        };
        let to_runs =
            button("research-to-runs", "Runs", false).on_click(cx.listener(|this, _, _, cx| {
                this.ui.page = super::Page::Runs;
                cx.notify();
            }));
        let opened_key = choice.opened.then_some(choice.cell.as_ref()).flatten();
        let section = div()
            .flex()
            .flex_col()
            .gap(px(if opened_key.is_some() { 24. } else { 32. }))
            .child(page_head(
                "Matrix",
                subtitle,
                false,
                vec![to_runs.into_any_element()],
            ));
        if snap.grid.rows.is_empty() {
            return (
                section.child(body(
                    "No verifiers rollouts to place yet. Runs appear from outputs/*/traces.jsonl \
                     in a folder's worktree; Harbor jobs are on the Runs page.",
                    DIM,
                )),
                None,
            );
        }
        let rollouts: u32 = snap.grid.cells.iter().map(|c| c.rollouts).sum();
        let errors: u32 = snap.grid.cells.iter().map(|c| c.errored).sum();
        let counted: Vec<f64> = snap
            .grid
            .cells
            .iter()
            .flat_map(|c| c.members.iter())
            .filter_map(|&i| snap.rollouts.get(i))
            .filter(|r| r.counts_in_mean())
            .filter_map(|r| r.reward)
            .collect();
        let mean =
            (!counted.is_empty()).then(|| counted.iter().sum::<f64>() / counted.len() as f64);
        let section = section
            .child(stats(vec![
                ("Tasks", snap.grid.rows.len().to_string()),
                ("Rollouts", rollouts.to_string()),
                ("Mean reward", reward(mean)),
                ("Errors", errors.to_string()),
            ]))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(section_head(
                        "Reward by task",
                        self.env_tabs(&snap, cx),
                        Some("harness × model".to_owned()),
                    ))
                    .child(self.matrix_table(&snap, choice, cx))
                    .child(legend(snap.grid.rows.len())),
            );
        let opened = opened_key.map(|key| self.opened_cell(&snap, key, cx));
        (section, opened)
    }

    /// The envs as tabs, each with its rollout count; one env is named in a tab alone.
    fn env_tabs(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Option<Div> {
        if snap.envs.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .children(snap.envs.iter().enumerate().map(|(i, env)| {
                    let lit = snap.env.as_deref() == Some(env.as_str());
                    let n = snap
                        .rollouts
                        .iter()
                        .filter(|r| seamlessh_runs::axis_text(r.axes.env.as_deref()) == env)
                        .count();
                    let pick = env.clone();
                    kit::tab(
                        ("research-env", i as u64),
                        env.clone(),
                        Some(n.to_string().into()),
                        lit,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.set_env(pick.clone());
                        cx.notify();
                    }))
                })),
        )
    }

    fn matrix_table(
        &self,
        snap: &Snapshot,
        choice: &Choice,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let header = div()
            .flex()
            .flex_none()
            .child(th("Task", Some(TASK_W), true))
            .children(
                snap.grid
                    .columns
                    .iter()
                    .map(|col| th(col.join(" · "), Some(COL_W), false).font_family(MONO)),
            );
        // The first question a matrix answers is which column scores best, so the
        // column means over every task come before the tasks.
        let all = div()
            .flex()
            .flex_none()
            .py(px(10.))
            .border_t_1()
            .border_color(white(0.06))
            .child(
                cell_box(Some(TASK_W), true)
                    .child(body("All tasks", INK).font_weight(gpui::FontWeight::MEDIUM)),
            )
            .children(snap.column_totals().into_iter().map(|c| {
                cell_box(Some(COL_W), false)
                    .flex()
                    .child(figures(&c).font_weight(gpui::FontWeight::MEDIUM))
            }));
        let rows = snap.grid.rows.iter().enumerate().map(|(ri, row_key)| {
            let row_chosen = choice.cell.as_ref().is_some_and(|k| &k.row == row_key);
            table_row(("research-mrow", ri as u64), row_chosen)
                .child(
                    cell_box(Some(TASK_W), true)
                        .child(body(snap.row_label(row_key), INK).truncate()),
                )
                .children(snap.grid.columns.iter().enumerate().map(|(ci, col_key)| {
                    let key = CellKey {
                        row: row_key.clone(),
                        column: col_key.clone(),
                    };
                    let chosen = choice.cell.as_ref() == Some(&key);
                    matrix_cell(ri, ci, key, snap.grid.cell(ri, ci), chosen, cx)
                }))
        });
        let min_w = TASK_W + COL_W * snap.grid.columns.len() as f32;
        div()
            .id("research-matrix")
            .w_full()
            .overflow_x_scroll()
            .child(
                div()
                    .min_w(px(min_w))
                    .flex()
                    .flex_col()
                    .child(header)
                    .child(all)
                    .children(rows),
            )
    }

    /// The opened cell as the board's detail column: the task and where its rollouts
    /// ran, a line of what they scored, its figures, a row per rollout with the chosen
    /// one marked, and reading it at the foot.
    fn opened_cell(&self, snap: &Snapshot, key: &CellKey, cx: &mut Context<Self>) -> Div {
        let cell = snap.index(key).and_then(|(r, c)| snap.grid.cell(r, c));
        let members: &[usize] = cell.map_or(&[], |c| c.members.as_slice());
        let mut about: Vec<String> = key.column.clone();
        about.extend(snap.env.clone());
        let pick = self.ui.matrix.pick.min(members.len().saturating_sub(1));
        let rows: Vec<AnyElement> = members
            .iter()
            .enumerate()
            .filter_map(|(at, &i)| {
                let rollout = snap.rollouts.get(i)?;
                let origin = snap.origins.get(i)?;
                let run = self
                    .ui
                    .heard
                    .get(&origin.place)
                    .and_then(|h| h.runs.iter().find(|r| r.path == origin.path))
                    .map_or_else(|| origin.path.clone(), run_name);
                let place = self.place_name(origin.place);
                // The word says how it ended (its stop condition or error type), which
                // is what tells two completed rollouts apart; the dot keeps the outcome.
                let (color, _) = outcome_status(rollout.outcome);
                let (word, _) = ending(rollout);
                Some(
                    table_row(("research-cell-rollout", at as u64), at == pick)
                        .py(px(8.))
                        .child(
                            cell_box(None, true)
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(name_cell(rollout.id.clone()))
                                .child(quiet(format!("{run} · {place}")).truncate()),
                        )
                        .child(
                            cell_box(Some(150.), false)
                                .min_w_0()
                                .child(kit::status(color, word).min_w_0().truncate()),
                        )
                        .child(
                            div()
                                .w(px(44.))
                                .flex_none()
                                .flex()
                                .justify_end()
                                .pr(px(super::screen::TABLE_PAD))
                                .child(body(reward(rollout.reward), INK).font_family(MONO)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ui.matrix.pick = at;
                            this.ui.read_pick();
                            this.ask_document(cx);
                            cx.notify();
                        }))
                        .into_any_element(),
                )
            })
            .collect();
        let t = tally(members.iter().filter_map(|&i| snap.rollouts.get(i)));
        let (n, errored, counted) = cell.map_or((0, 0, 0), |c| (c.rollouts, c.errored, c.counted));
        let summary = match (n, errored) {
            (0, _) => "No rollouts ran here.".to_owned(),
            (n, 0) => format!(
                "{n} rollout{}, mean reward {} over the {counted} that count.",
                if n == 1 { "" } else { "s" },
                reward(cell.and_then(|c| c.mean))
            ),
            (n, e) => format!(
                "{n} rollouts, mean reward {} over the {counted} that count; {e} errored, \
                 which are left out rather than counted as zero.",
                reward(cell.and_then(|c| c.mean))
            ),
        };
        let close = button("research-cell-close", "Close", false).on_click(cx.listener(
            |this, _, _, cx| {
                this.ui.matrix.opened = false;
                cx.notify();
            },
        ));
        let read = button("research-cell-read", "Read rollout", true).on_click(cx.listener(
            |this, _, _, cx| {
                this.ui.read_pick();
                this.ask_document(cx);
                cx.notify();
            },
        ));
        aside()
            .child(aside_head(snap.row_label(&key.row), about.join(" · ")))
            .child(body(summary, INK_2))
            .child(fact_grid(
                None,
                vec![
                    ("Mean reward", reward(cell.and_then(|c| c.mean)), INK),
                    ("Rollouts", n.to_string(), INK),
                    ("Counted", counted.to_string(), INK),
                    (
                        "Errors",
                        errored.to_string(),
                        if errored > 0 { EMBER } else { INK },
                    ),
                    (
                        "Mean time",
                        t.mean_secs.map_or_else(|| "–".to_owned(), seconds),
                        INK_2,
                    ),
                ],
            ))
            .children(tally_parts(&t))
            .child(
                aside_section("Rollouts").child(div().flex().flex_col().children(
                    if rows.is_empty() {
                        vec![body("No rollouts in this cell.", DIM).into_any_element()]
                    } else {
                        rows
                    },
                )),
            )
            .child(quiet("↑↓ choose · ↩ read · ⎋ close"))
            .child(aside_actions(if members.is_empty() {
                vec![close]
            } else {
                vec![close, read]
            }))
    }
}

/// What a cell's figures are and which keys the page answers, as the board's quiet
/// line under its table.
fn legend(tasks: usize) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(quiet(format!(
            "{tasks} task{} · mean reward, then rollouts;",
            if tasks == 1 { "" } else { "s" }
        )))
        .child(quiet("errors").text_color(rgb(EMBER)))
        .child(quiet("never count as zero"))
        .child(div().flex_1())
        .child(quiet("←↑↓→ move · ↩ open · [ ] env · , . folder"))
}

/// A mean, its rollouts and its errors, as a cell and the "All tasks" row print them.
fn figures(t: &Totals) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(8.))
        .whitespace_nowrap()
        .child(body(reward(t.mean), INK).font_family(MONO))
        .child(body(t.rollouts.to_string(), DIM).font_family(MONO))
        .when(t.errored > 0, |d| {
            d.child(body(t.errored.to_string(), EMBER).font_family(MONO))
        })
}

/// A cell: the mean in monospace, the rollout count quieter, errors in ember. The
/// chosen one sits on a raised grey inside its row's tint, so the row and the column
/// both read.
fn matrix_cell(
    row: usize,
    col: usize,
    key: CellKey,
    cell: Option<&runs_matrix::Cell>,
    chosen: bool,
    cx: &mut Context<ResearchView>,
) -> Stateful<Div> {
    // why: row and column counts are on-screen table sizes, so the pair is unique.
    let id = (row as u64) << 32 | col as u64;
    let figures = match cell {
        None => div().child(body("–", DIM).font_family(MONO)),
        Some(c) => figures(&Totals {
            mean: c.mean,
            rollouts: c.rollouts,
            errored: c.errored,
        }),
    };
    cell_box(Some(COL_W), false)
        .flex()
        .id(("research-cell", id))
        .cursor_pointer()
        .child(
            div()
                .flex()
                .ml(px(-8.))
                .px(px(8.))
                .rounded(px(theme::R_ROW))
                .when(chosen, |d| d.bg(white(0.1)))
                .child(figures),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            this.ui.pick_cell(key.clone());
            cx.notify();
        }))
}
