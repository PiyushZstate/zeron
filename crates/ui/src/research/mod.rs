//! The Research view (Seamlessh's D23 and docs/research-view.md), ported from
//! Seamlessh's own GPUI app: its own screen in the window, beside the coding view
//! rather than inside it, as the locked design draws it. A column of its pages and of
//! the folders that hold runs, and one main panel. The Matrix page (the default)
//! places every verifiers rollout of every folder by task × harness × model for one
//! env; a cell opens its rollouts in a column split off the panel. The Runs page
//! lists every run newest first, and the chosen one opens beside the list with its
//! totals and a row per task; another run can be pinned as the baseline it is
//! compared against, task by task.
//!
//! It only draws what the engine read: each folder's engine (this device's or
//! another's, through `targetDeviceId`) reads its `outputs/` and `jobs/` again on
//! each ask, every few seconds while the screen shows. A rollout of an opened cell
//! opens as a document, which the engine reads on asking. Nothing runs in Docker and
//! nothing is copied: the run files are the record.

mod checks;
mod document;
mod matrix;
mod screen;
pub(crate) mod style;
mod tally;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, KeyDownEvent, MouseButton, Task, Window, div,
    prelude::*, px,
};
use seamlessh_runs::Outcome;
use zeron_engine::research::{ResearchRun, ResearchRuns};
use zeron_rpc::methods;

use crate::state::AppState;
use style::{ADDED, DIM, EMBER, MIST, WORKING};

/// How often the open screen asks each folder again. No run format marks its end, and
/// a run being written grows by a line per episode.
const POLL: Duration = Duration::from_secs(3);

/// A folder the screen reads, by its place in [`ResearchView::places`]: ids are
/// handed out once and never reused, so a choice survives the list changing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PlaceId(pub u64);

/// What the engine last said about one place's runs.
pub(crate) struct Heard {
    pub(crate) runs: Vec<ResearchRun>,
    pub(crate) error: Option<String>,
}

/// A run across the places: its place and its directory there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RunKey {
    pub(crate) place: PlaceId,
    pub(crate) path: String,
}

impl RunKey {
    fn of(place: PlaceId, run: &ResearchRun) -> Self {
        Self {
            place,
            path: run.path.clone(),
        }
    }
}

/// The screen's two pages, the matrix first as D23 makes it the main view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Page {
    #[default]
    Matrix,
    Runs,
    Checks,
}

/// What the engine said about each folder, and the choices on the screen. Choices are kept by key rather than by index, so a refresh that
/// reorders runs keeps them.
#[derive(Default)]
pub(crate) struct ResearchUi {
    page: Page,
    asking: BTreeSet<PlaceId>,
    /// By place id, so runs gather in one order every frame.
    heard: BTreeMap<PlaceId, Heard>,
    /// One place's runs, or every place's.
    scope: Option<PlaceId>,
    matrix: matrix::Choice,
    chosen: Option<RunKey>,
    baseline: Option<RunKey>,
    /// The rollout open as a document, over whichever page opened it.
    reading: Option<document::Reading>,
    /// A document to ask the engine for: the keys are handled here, the sending is not.
    wants: Option<(PlaceId, String, String)>,
}

impl ResearchUi {
    pub(crate) fn heard(&mut self, place: PlaceId, runs: Vec<ResearchRun>, error: Option<String>) {
        self.asking.remove(&place);
        let gone = |key: &Option<RunKey>| {
            key.as_ref()
                .is_some_and(|k| k.place == place && !runs.iter().any(|r| r.path == k.path))
        };
        if gone(&self.chosen) {
            self.chosen = None;
        }
        if gone(&self.baseline) {
            self.baseline = None;
        }
        self.heard.insert(place, Heard { runs, error });
        self.resolve_matrix();
    }

    /// Every run in scope, newest first; a run with no start time goes last.
    fn runs(&self) -> Vec<(PlaceId, &ResearchRun)> {
        let mut runs: Vec<(PlaceId, &ResearchRun)> = self
            .heard
            .iter()
            .filter(|(place, _)| self.scope.is_none_or(|s| s == **place))
            .flat_map(|(place, heard)| heard.runs.iter().map(|run| (*place, run)))
            .collect();
        runs.sort_by(|a, b| {
            let (a, b) = (a.1.summary.started_at, b.1.summary.started_at);
            b.unwrap_or(f64::MIN).total_cmp(&a.unwrap_or(f64::MIN))
        });
        runs
    }

    /// Threads with at least one run, in id order: the sidebar's list and the scope's
    /// cycle.
    fn places_with_runs(&self) -> Vec<(PlaceId, usize)> {
        self.heard
            .iter()
            .filter(|(_, heard)| !heard.runs.is_empty())
            .map(|(place, heard)| (*place, heard.runs.len()))
            .collect()
    }

    fn set_scope(&mut self, scope: Option<PlaceId>) {
        self.scope = scope;
        self.resolve_matrix();
    }

    /// All folders, then each place with runs, wrapping.
    fn cycle_scope(&mut self, by: isize) {
        let mut scopes: Vec<Option<PlaceId>> = vec![None];
        scopes.extend(self.places_with_runs().into_iter().map(|(t, _)| Some(t)));
        let at = scopes.iter().position(|s| *s == self.scope).unwrap_or(0);
        let next = matrix::wrap_index(at, by, scopes.len());
        self.set_scope(scopes.get(next).copied().flatten());
    }

    /// Open the opened cell's chosen rollout as a document, and ask for it.
    fn read_pick(&mut self) {
        let snap = self.snapshot();
        let Some(cell) = self
            .matrix
            .cell
            .as_ref()
            .and_then(|k| snap.index(k))
            .and_then(|(r, c)| snap.grid.cell(r, c))
        else {
            return;
        };
        let pick = self.matrix.pick.min(cell.members.len().saturating_sub(1));
        let Some(&i) = cell.members.get(pick) else {
            return;
        };
        let (Some(rollout), Some(run)) = (snap.rollouts.get(i), snap.origins.get(i)) else {
            return;
        };
        self.wants = Some((run.place, run.path.clone(), rollout.id.clone()));
        self.reading = Some(document::Reading::new(run.clone(), rollout.clone()));
    }

    /// The engine's answer to a document asked for; dropped when the reader has moved on.
    pub(crate) fn heard_document(
        &mut self,
        place: PlaceId,
        path: &str,
        rollout: &str,
        document: Option<seamlessh_runs::Document>,
        error: Option<String>,
    ) {
        if let Some(reading) = self.reading.as_mut().filter(|r| r.is(place, path, rollout)) {
            reading.heard(document, error);
        }
    }

    fn snapshot(&self) -> matrix::Snapshot {
        matrix::snapshot(&self.runs(), self.matrix.env.as_deref())
    }

    /// Keep the env and cell when they still exist, else the first of each.
    fn resolve_matrix(&mut self) {
        let snap = self.snapshot();
        let keep = self.matrix.cell.take().filter(|k| snap.contains(k));
        self.matrix.opened &= keep.is_some();
        self.matrix.cell = keep.or_else(|| snap.first_cell());
        self.matrix.env = snap.env;
    }

    fn step_cell(&mut self, drow: isize, dcol: isize) {
        let snap = self.snapshot();
        let (rows, cols) = (snap.grid.rows.len(), snap.grid.columns.len());
        if rows == 0 || cols == 0 {
            return;
        }
        let at = self
            .matrix
            .cell
            .as_ref()
            .and_then(|k| snap.index(k))
            .unwrap_or((0, 0));
        let (r, c) = (
            matrix::step_index(at.0, drow, rows),
            matrix::step_index(at.1, dcol, cols),
        );
        if let (Some(row), Some(column)) = (snap.grid.rows.get(r), snap.grid.columns.get(c)) {
            self.matrix.pick = 0;
            self.matrix.cell = Some(matrix::CellKey {
                row: row.clone(),
                column: column.clone(),
            });
        }
    }

    fn cycle_env(&mut self, by: isize) {
        let snap = self.snapshot();
        if snap.envs.is_empty() {
            return;
        }
        let at = snap
            .env
            .as_ref()
            .and_then(|e| snap.envs.iter().position(|x| x == e))
            .unwrap_or(0);
        let next = matrix::wrap_index(at, by, snap.envs.len());
        self.matrix.env = snap.envs.get(next).cloned();
        self.resolve_matrix();
    }

    fn set_env(&mut self, env: String) {
        self.matrix.env = Some(env);
        self.resolve_matrix();
    }

    /// Choose a cell; choosing the chosen one opens it.
    fn pick_cell(&mut self, key: matrix::CellKey) {
        if self.matrix.cell.as_ref() == Some(&key) {
            self.matrix.opened = true;
        } else {
            self.matrix.pick = 0;
        }
        self.matrix.cell = Some(key);
    }

    /// The chosen run, else the newest in scope.
    fn chosen_run(&self) -> Option<(PlaceId, &ResearchRun)> {
        let runs = self.runs();
        let found = self.chosen.as_ref().and_then(|k| {
            runs.iter()
                .find(|(t, r)| *t == k.place && r.path == k.path)
                .copied()
        });
        found.or_else(|| runs.first().copied())
    }

    fn baseline_run(&self) -> Option<(PlaceId, &ResearchRun)> {
        let key = self.baseline.as_ref()?;
        let heard = self.heard.get(&key.place)?;
        let run = heard.runs.iter().find(|r| r.path == key.path)?;
        Some((key.place, run))
    }

    /// Move the chosen run `by` rows, staying in the list.
    fn step_run(&mut self, by: isize) {
        let runs = self.runs();
        let at = self
            .chosen_run()
            .and_then(|(t, c)| runs.iter().position(|(rt, r)| *rt == t && r.path == c.path))
            .unwrap_or(0);
        let next = at
            .saturating_add_signed(by)
            .min(runs.len().saturating_sub(1));
        let key = runs.get(next).map(|(t, r)| RunKey::of(*t, r));
        if key.is_some() {
            self.chosen = key;
        }
    }

    /// Pin the chosen run as the baseline, or let it go when it is the baseline.
    fn toggle_baseline(&mut self) {
        let Some(key) = self.chosen_run().map(|(t, r)| RunKey::of(t, r)) else {
            return;
        };
        self.baseline = if self.baseline.as_ref() == Some(&key) {
            None
        } else {
            Some(key)
        };
    }

    /// A plain key on the screen. A document takes them first: ↑ and ↓ move between
    /// turns, ↩ and space show or fold one, ⌫ and escape go back. Both pages: `,` and
    /// `.` change the place in scope. Matrix: arrows move the cell, `[` and `]` change
    /// the env, ↩ opens the cell; once open, ↑ and ↓ choose its rollout instead, ↩
    /// reads it, and ⌫ and escape close the cell. Runs: ↑ and ↓ choose a run, `b` pins
    /// it as the baseline. Whether the key was used.
    fn key(&mut self, key: &str) -> bool {
        if let Some(reading) = &mut self.reading {
            match key {
                "up" => reading.step(-1),
                "down" => reading.step(1),
                "enter" | "space" => {
                    let at = reading.at;
                    reading.toggle(at);
                }
                "backspace" | "escape" => self.reading = None,
                _ => return false,
            }
            return true;
        }
        let opened = self.matrix.opened;
        match (self.page, key) {
            (_, "1") => self.page = Page::Matrix,
            (_, "2") => self.page = Page::Runs,
            (_, "3") => self.page = Page::Checks,
            (_, ",") => self.cycle_scope(-1),
            (_, ".") => self.cycle_scope(1),
            (Page::Matrix, "up") if opened => self.matrix.pick = self.matrix.pick.saturating_sub(1),
            (Page::Matrix, "down") if opened => {
                self.matrix.pick = self.matrix.pick.saturating_add(1)
            }
            (Page::Matrix, "enter") if opened => self.read_pick(),
            (Page::Matrix, "up") => self.step_cell(-1, 0),
            (Page::Matrix, "down") => self.step_cell(1, 0),
            (Page::Matrix, "left") => self.step_cell(0, -1),
            (Page::Matrix, "right") => self.step_cell(0, 1),
            (Page::Matrix, "[") => self.cycle_env(-1),
            (Page::Matrix, "]") => self.cycle_env(1),
            (Page::Matrix, "enter") => self.matrix.opened = true,
            (Page::Matrix, "backspace" | "escape") => self.matrix.opened = false,
            (Page::Runs, "up") => self.step_run(-1),
            (Page::Runs, "down") => self.step_run(1),
            (Page::Runs, "b") => self.toggle_baseline(),
            _ => return false,
        }
        true
    }
}

/// A run's name: what it calls itself, else its directory.
pub(crate) fn run_name(run: &ResearchRun) -> String {
    run.summary
        .facts
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| run.path.rsplit('/').next().unwrap_or(&run.path).to_owned())
}

/// A reward as the views print it: three places, or a dash when there is none.
pub(crate) fn reward(value: Option<f64>) -> String {
    value.map_or_else(|| "–".to_owned(), |v| format!("{v:.3}"))
}

/// A difference with its sign, and whether it is up, down or level.
pub(crate) fn delta(value: f64) -> (String, u32) {
    if value.abs() < 0.0005 {
        ("±0.000".to_owned(), DIM)
    } else if value > 0. {
        (format!("+{value:.3}"), ADDED)
    } else {
        (format!("−{:.3}", value.abs()), EMBER)
    }
}

fn count(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// A rollout's status as the locked design draws one: a dot's colour and its word.
fn outcome_status(outcome: Outcome) -> (u32, &'static str) {
    match outcome {
        Outcome::Completed => (WORKING, "Completed"),
        Outcome::Errored => (EMBER, "Errored"),
        Outcome::TimedOut => (style::WAITING, "Timed out"),
        Outcome::Unfinished => (MIST, "Unfinished"),
    }
}

/// A folder that may hold runs: a project's own folder or a chat's worktree.
struct Place {
    device_id: String,
    cwd: String,
    /// The worktree's branch or the folder's name.
    name: String,
    /// The project it belongs to, the sidebar's group.
    project: String,
}

/// The Research screen: its state, the folders it reads, and the engine calls.
pub struct ResearchView {
    state: Entity<AppState>,
    ui: ResearchUi,
    places: Vec<Place>,
    focus: FocusHandle,
    /// Take the keyboard on the next frame: the screen works by keys, and the
    /// focus it opened from (a composer) is no longer drawn.
    wants_focus: bool,
    _poll: Task<()>,
}

impl ResearchView {
    /// The shell calls this when the Research route comes back into view.
    pub fn on_visible(&mut self) {
        self.wants_focus = true;
    }

    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                if this.update(cx, |view, cx| view.refresh(cx)).is_err() {
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
        });
        // Dev/testing knob, beside the shell's: `ZERON_OPEN_ROUTE=research/runs` or
        // `research/checks` opens on that page, which synthetic input can't otherwise reach.
        let mut ui = ResearchUi::default();
        match std::env::var("ZERON_OPEN_ROUTE").as_deref() {
            Ok("research/runs") => ui.page = Page::Runs,
            Ok("research/checks") => ui.page = Page::Checks,
            _ => {}
        }
        Self {
            state,
            ui,
            places: Vec::new(),
            focus: cx.focus_handle(),
            wants_focus: true,
            _poll: poll,
        }
    }

    /// Every folder the workspace knows: each project's folder, and each chat's when
    /// it runs elsewhere (a worktree), under its project's name.
    fn found_places(state: &AppState) -> Vec<Place> {
        let mut found: Vec<Place> = Vec::new();
        let mut add = |place: Place| {
            if !found
                .iter()
                .any(|p| p.device_id == place.device_id && p.cwd == place.cwd)
            {
                found.push(place);
            }
        };
        for space in state.spaces_sorted() {
            let project = space_name(space);
            add(Place {
                device_id: space.device_id.clone(),
                cwd: space.path.clone(),
                name: project.clone(),
                project,
            });
        }
        for chat in state.chats.iter().filter(|c| !c.archived) {
            let Some(cwd) = chat.cwd.clone().filter(|c| !c.is_empty()) else {
                continue;
            };
            let project = state
                .space_for_chat(chat)
                .map_or_else(|| folder_name(&cwd), space_name);
            let name = chat.branch.clone().unwrap_or_else(|| folder_name(&cwd));
            add(Place {
                device_id: chat.device_id.clone(),
                cwd,
                name,
                project,
            });
        }
        found
    }

    /// The id for a folder, handing out a new one the first time it is seen.
    fn place_id(&mut self, place: Place) -> PlaceId {
        if let Some(at) = self
            .places
            .iter()
            .position(|p| p.device_id == place.device_id && p.cwd == place.cwd)
        {
            self.places[at] = place;
            return PlaceId(at as u64);
        }
        self.places.push(place);
        PlaceId(self.places.len() as u64 - 1)
    }

    /// Ask every folder again, unless an answer is on its way.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let (engine, found) = {
            let state = self.state.read(cx);
            (state.engine().cloned(), Self::found_places(state))
        };
        let Some(engine) = engine else {
            return;
        };
        for place in found {
            let (device, cwd) = (place.device_id.clone(), place.cwd.clone());
            let id = self.place_id(place);
            if !self.ui.asking.insert(id) {
                continue;
            }
            let engine = engine.clone();
            cx.spawn(async move |this, cx| {
                let reply = engine
                    .client()
                    .call(
                        methods::LIST_RESEARCH_RUNS,
                        serde_json::json!({ "cwd": cwd, "targetDeviceId": device }),
                    )
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|v| {
                        serde_json::from_value::<ResearchRuns>(v).map_err(|e| e.to_string())
                    });
                let _ = this.update(cx, |view, cx| {
                    match reply {
                        Ok(r) => view.ui.heard(id, r.runs, None),
                        Err(why) => view.ui.heard(id, Vec::new(), Some(why)),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn place_name(&self, place: PlaceId) -> String {
        self.places
            .get(place.0 as usize)
            .map_or_else(|| "a folder".to_owned(), |p| p.name.clone())
    }

    fn place_project(&self, place: PlaceId) -> String {
        self.places
            .get(place.0 as usize)
            .map_or_else(String::new, |p| p.project.clone())
    }

    fn toggle_baseline(&mut self) {
        self.ui.toggle_baseline();
    }

    /// Send the document request the keys or a click left.
    fn ask_document(&mut self, cx: &mut Context<Self>) {
        let Some((place, path, rollout)) = self.ui.wants.take() else {
            return;
        };
        let Some(engine) = self.state.read(cx).engine().cloned() else {
            return;
        };
        let Some(at) = self.places.get(place.0 as usize) else {
            return;
        };
        let params = serde_json::json!({
            "cwd": at.cwd,
            "run": path,
            "rollout": rollout,
            "targetDeviceId": at.device_id,
        });
        cx.spawn(async move |this, cx| {
            let reply = engine
                .client()
                .call(methods::GET_RESEARCH_DOCUMENT, params)
                .await
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    serde_json::from_value::<seamlessh_runs::Document>(v).map_err(|e| e.to_string())
                });
            let _ = this.update(cx, |view, cx| {
                let (doc, why) = match reply {
                    Ok(doc) => (Some(doc), None),
                    Err(why) => (None, Some(why)),
                };
                view.ui.heard_document(place, &path, &rollout, doc, why);
                cx.notify();
            });
        })
        .detach();
    }
}

fn folder_name(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned()
}

fn space_name(space: &zeron_proto::Space) -> String {
    space
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| folder_name(&space.path))
}

impl Focusable for ResearchView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ResearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.wants_focus) {
            window.focus(&self.focus, cx);
        }
        div()
            .id("research")
            .key_context("Research")
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| window.focus(&this.focus, cx)),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                if this.ui.key(event.keystroke.key.as_str()) {
                    this.ask_document(cx);
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .size_full()
            .flex()
            .gap(px(6.))
            .p(px(6.))
            // The window's buttons overlay the top left, as over Zeron's own
            // sidebar, so the list starts below them.
            .child(
                div()
                    .h_full()
                    .flex_none()
                    .pt(px(crate::theme::Theme::TITLEBAR_HEIGHT - 6.))
                    .child(self.research_sidebar(cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.research_panel(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamlessh_runs::{Axes, Framework, Rollout, RunFacts, RunSummary, TaskId, Tokens};

    fn run(path: &str, started: f64) -> ResearchRun {
        ResearchRun {
            path: path.to_owned(),
            live: false,
            summary: RunSummary {
                framework: Framework::Verifiers,
                facts: RunFacts::default(),
                tasks: 0,
                rollouts: 0,
                errored: 0,
                timed_out: 0,
                unfinished: 0,
                mean_reward: None,
                tokens: Tokens::default(),
                cost: None,
                started_at: Some(started),
            },
            tasks: Vec::new(),
            rollouts: Vec::new(),
        }
    }

    fn rollout(
        id: &str,
        task: &str,
        env: &str,
        harness: &str,
        outcome: Outcome,
        value: Option<f64>,
    ) -> Rollout {
        Rollout {
            id: id.into(),
            task: TaskId {
                key: task.into(),
                label: task.into(),
                checksum: None,
            },
            outcome,
            stop: None,
            error: None,
            rewards: vec![],
            reward: value,
            tokens: Tokens::default(),
            cost: None,
            duration_secs: None,
            started_at: None,
            traces: 1,
            axes: Axes {
                env: Some(env.to_owned()),
                harness: Some(harness.to_owned()),
                model: Some("qwen".to_owned()),
                runtime: None,
            },
        }
    }

    fn vrun(path: &str, started: f64, rollouts: Vec<Rollout>) -> ResearchRun {
        let mut run = run(path, started);
        run.summary.rollouts = rollouts.len() as u32;
        run.rollouts = rollouts;
        run
    }

    fn cell(row: &str, harness: &str) -> matrix::CellKey {
        matrix::CellKey {
            row: row.into(),
            column: vec![harness.into(), "qwen".into()],
        }
    }

    /// Thread 1 ran bash on t1 and t2; place 2 ran python on t1, and a Harbor job that
    /// stays out of the matrix.
    fn ui() -> ResearchUi {
        let mut ui = ResearchUi::default();
        ui.heard(
            PlaceId(1),
            vec![vrun(
                "outputs/bash",
                10.,
                vec![
                    rollout("1", "t1", "gsm8k", "bash", Outcome::Completed, Some(1.0)),
                    rollout("2", "t2", "gsm8k", "bash", Outcome::Completed, Some(0.5)),
                ],
            )],
            None,
        );
        let mut harbor = vrun(
            "jobs/h",
            30.,
            vec![rollout(
                "h",
                "t9",
                "gsm8k",
                "bash",
                Outcome::Completed,
                Some(1.0),
            )],
        );
        harbor.summary.framework = Framework::Harbor;
        ui.heard(
            PlaceId(2),
            vec![
                vrun(
                    "outputs/python",
                    20.,
                    vec![rollout(
                        "3",
                        "t1",
                        "gsm8k",
                        "python",
                        Outcome::Errored,
                        Some(0.9),
                    )],
                ),
                harbor,
            ],
            None,
        );
        ui
    }

    #[test]
    fn the_matrix_reads_across_threads_and_leaves_harbor_out() {
        let ui = ui();
        let snap = ui.snapshot();
        assert_eq!(snap.grid.rows, ["t1", "t2"]);
        assert_eq!(
            snap.grid.columns.len(),
            2,
            "bash from one place, python from another"
        );
        assert_eq!(snap.origins.len(), 3);
        // Thread 1 was heard first, and its cell stays chosen as place 2 arrives.
        assert_eq!(ui.matrix.cell, Some(cell("t1", "bash")));
    }

    #[test]
    fn all_tasks_means_each_column_over_its_rollouts_leaving_errors_out() {
        let snap = ui().snapshot();
        let totals = snap.column_totals();
        let bash = snap
            .grid
            .columns
            .iter()
            .position(|c| c[0] == "bash")
            .and_then(|i| totals.get(i).cloned());
        let python = snap
            .grid
            .columns
            .iter()
            .position(|c| c[0] == "python")
            .and_then(|i| totals.get(i).cloned());
        assert_eq!(
            bash,
            Some(matrix::Totals {
                mean: Some(0.75),
                rollouts: 2,
                errored: 0
            })
        );
        assert_eq!(
            python,
            Some(matrix::Totals {
                mean: None,
                rollouts: 1,
                errored: 1
            })
        );
    }

    #[test]
    fn cell_navigation_clamps_and_reaches_empty_cells() {
        let mut ui = ui();
        ui.step_cell(-1, -1);
        assert_eq!(ui.matrix.cell, Some(cell("t1", "python")));
        ui.step_cell(1, 0);
        // t2 has no python rollout; the cell can still be chosen.
        assert_eq!(ui.matrix.cell, Some(cell("t2", "python")));
        ui.step_cell(5, 5);
        assert_eq!(ui.matrix.cell, Some(cell("t2", "bash")));
    }

    #[test]
    fn scope_narrows_to_one_thread_and_cycles_back_to_all() {
        let mut ui = ui();
        ui.cycle_scope(1);
        assert_eq!(ui.scope, Some(PlaceId(1)));
        assert_eq!(ui.snapshot().grid.columns.len(), 1);
        // The python cell is gone from place 1's matrix, so the choice falls back.
        assert_eq!(ui.matrix.cell, Some(cell("t1", "bash")));
        ui.cycle_scope(1);
        assert_eq!(ui.scope, Some(PlaceId(2)));
        ui.cycle_scope(1);
        assert_eq!(ui.scope, None);
    }

    #[test]
    fn runs_list_newest_first_and_the_choice_survives_a_refresh() {
        let mut ui = ui();
        let paths: Vec<&str> = ui.runs().iter().map(|(_, r)| r.path.as_str()).collect();
        assert_eq!(paths, ["jobs/h", "outputs/python", "outputs/bash"]);
        ui.page = Page::Runs;
        assert!(ui.key("down"));
        assert!(ui.key("down"));
        assert!(ui.key("down"));
        assert_eq!(
            ui.chosen_run().map(|(_, r)| r.path.clone()),
            Some("outputs/bash".into())
        );
        ui.heard(PlaceId(2), Vec::new(), None);
        assert_eq!(
            ui.chosen_run().map(|(_, r)| r.path.clone()),
            Some("outputs/bash".into())
        );
    }

    #[test]
    fn the_baseline_pins_and_unpins_and_goes_with_its_run() {
        let mut ui = ui();
        ui.page = Page::Runs;
        assert!(ui.key("b"));
        assert_eq!(
            ui.baseline_run().map(|(_, r)| r.path.clone()),
            Some("jobs/h".into())
        );
        assert!(ui.key("b"));
        assert!(ui.baseline.is_none());
        ui.key("b");
        ui.heard(PlaceId(2), Vec::new(), None);
        assert!(ui.baseline.is_none(), "its run is gone");
    }

    #[test]
    fn enter_opens_a_cell_then_reads_its_chosen_rollout() {
        let mut ui = ui();
        assert!(ui.key("enter"));
        assert!(ui.matrix.opened);
        // t1 × bash holds one rollout; the pick clamps to it.
        ui.key("down");
        ui.key("down");
        assert!(ui.key("enter"));
        let reading = ui.reading.as_ref().unwrap();
        assert_eq!(reading.rollout.id, "1");
        assert_eq!(
            ui.wants,
            Some((PlaceId(1), "outputs/bash".into(), "1".into()))
        );
        // While reading, the document has the keys, and escape goes back to the cell.
        assert!(ui.key("down"));
        assert!(!ui.key("."), "the scope waits until the document is closed");
        assert!(ui.key("escape"));
        assert!(ui.reading.is_none());
        assert!(ui.matrix.opened);
        assert!(ui.key("escape"));
        assert!(!ui.matrix.opened);
        assert!(!ui.key("a"), "letters are not the screen's");
        ui.page = Page::Runs;
        assert!(!ui.key("left"));
    }

    #[test]
    fn a_document_answer_lands_only_on_the_rollout_being_read() {
        let mut ui = ui();
        ui.key("enter");
        ui.key("enter");
        ui.heard_document(PlaceId(2), "outputs/bash", "1", None, Some("other".into()));
        assert!(matches!(
            ui.reading.as_ref().unwrap().loaded,
            document::Loaded::Asking
        ));
        ui.heard_document(PlaceId(1), "outputs/bash", "1", None, Some("gone".into()));
        assert!(matches!(
            &ui.reading.as_ref().unwrap().loaded,
            document::Loaded::Failed(why) if why == "gone"
        ));
    }

    #[test]
    fn env_cycling_wraps() {
        let mut ui = ui();
        ui.heard(
            PlaceId(3),
            vec![vrun(
                "outputs/math",
                5.,
                vec![rollout(
                    "4",
                    "m1",
                    "math",
                    "bash",
                    Outcome::Completed,
                    Some(0.25),
                )],
            )],
            None,
        );
        let first = ui.matrix.env.clone();
        ui.key("]");
        assert_ne!(ui.matrix.env, first);
        ui.key("]");
        assert_eq!(ui.matrix.env, first);
        ui.key("[");
        assert_ne!(ui.matrix.env, first);
    }

    #[test]
    fn a_name_falls_back_to_the_directory_and_numbers_read_plainly() {
        let mut r = run("outputs/gsm8k--m--bash--0a1b2c3d", 0.);
        assert_eq!(run_name(&r), "gsm8k--m--bash--0a1b2c3d");
        r.summary.facts.name = Some("gsm8k".to_owned());
        assert_eq!(run_name(&r), "gsm8k");
        assert_eq!(reward(None), "–");
        assert_eq!(reward(Some(0.5)), "0.500");
        assert_eq!(delta(0.25).0, "+0.250");
        assert_eq!(delta(-0.25).0, "−0.250");
        assert_eq!(delta(0.0001).1, DIM);
        assert_eq!(count(12_345), "12.3k");
    }
}
