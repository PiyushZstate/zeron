//! The normalized run model: what the views draw, whichever framework wrote the run.
//!
//! Rules that hold for both frameworks:
//! - An absent reward is not zero. A rollout without one has `reward: None` and stays out of
//!   every mean.
//! - An errored rollout keeps whatever reward the file holds, so it can be shown, but it never
//!   counts in a mean. It is counted as an error.
//! - `Tokens::input` includes cache reads for both frameworks (verifiers stores
//!   `prompt_tokens` without them; Harbor's `n_input_tokens` already has them).

use serde::{Deserialize, Serialize};

/// Longest error message kept on a summary; the document holds the full text.
pub const MESSAGE_LIMIT: usize = 400;

/// Shown when an axis was not written on the rollout.
pub const NOT_RECORDED: &str = "not recorded";

/// `None` and empty strings render as [`NOT_RECORDED`].
pub fn axis_text(value: Option<&str>) -> &str {
    match value {
        Some(s) if !s.is_empty() => s,
        _ => NOT_RECORDED,
    }
}

/// Four of the five matrix axes. Task is [`Rollout::task`] `.key` (verifiers `task.key`,
/// else `task.hash`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Axes {
    pub env: Option<String>,
    pub harness: Option<String>,
    pub model: Option<String>,
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Framework {
    Verifiers,
    Harbor,
}

/// How a rollout ended. Only `Completed` and `TimedOut` rollouts with a reward count in means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// Finished without an error. Says nothing about whether the reward was high.
    Completed,
    /// verifiers `ok` was false, or Harbor recorded an exception.
    Errored,
    /// The environment's own deadline expired (verifiers `is_timeout`).
    TimedOut,
    /// Harbor trial with no `result.json` yet.
    Unfinished,
}

impl Outcome {
    pub fn is_error(self) -> bool {
        self == Outcome::Errored
    }
}

/// Task identity inside one framework. `key` is what comparison matches on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId {
    /// verifiers: `task.key`, else `task.hash`. Harbor: the task name.
    pub key: String,
    /// Short human text: the prompt's first line, or the task name.
    pub label: String,
    /// Harbor `task_checksum`. A different checksum under the same key means the task changed.
    pub checksum: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RewardPart {
    pub name: String,
    pub score: f64,
    /// verifiers weight. Harbor rewards have none.
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    /// Includes `cached`.
    pub input: Option<u64>,
    pub cached: Option<u64>,
    pub output: Option<u64>,
}

impl Tokens {
    pub fn add(&mut self, other: &Tokens) {
        self.input = sum_opt(self.input, other.input);
        self.cached = sum_opt(self.cached, other.cached);
        self.output = sum_opt(self.output, other.output);
    }

    pub fn total(&self) -> Option<u64> {
        sum_opt(self.input, self.output)
    }
}

fn sum_opt(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).saturating_add(b.unwrap_or(0))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rollout {
    /// verifiers episode id, or Harbor trial name.
    pub id: String,
    pub task: TaskId,
    pub outcome: Outcome,
    /// verifiers `stop_condition`.
    pub stop: Option<String>,
    /// The last error, newest last in the file.
    pub error: Option<ErrorInfo>,
    pub rewards: Vec<RewardPart>,
    /// verifiers: `sum(score * weight)`, `None` with no rewards. Harbor: the reward named
    /// `reward` or the only reward, else `None`; Harbor defines no sum.
    pub reward: Option<f64>,
    pub tokens: Tokens,
    pub cost: Option<f64>,
    pub duration_secs: Option<f64>,
    /// Epoch seconds.
    pub started_at: Option<f64>,
    /// Traces in a verifiers episode; 1 for Harbor.
    pub traces: u32,
    /// Env, harness, model, runtime. Task lives on [`Self::task`].
    pub axes: Axes,
    /// verifiers `metrics` of the main trace, in file order: what a run measures beside
    /// its reward, such as a kernel's runtime. Appended last for the wire's order.
    #[serde(default)]
    pub metrics: Vec<(String, f64)>,
}

impl Rollout {
    /// Whether this rollout's reward enters means.
    pub fn counts_in_mean(&self) -> bool {
        matches!(self.outcome, Outcome::Completed | Outcome::TimedOut) && self.reward.is_some()
    }
}

/// Facts a run states about itself; the rest comes from the rollouts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunFacts {
    pub name: Option<String>,
    /// verifiers env id, or Harbor dataset (`source`).
    pub env: Option<String>,
    pub models: Vec<String>,
    /// verifiers harness ids, or Harbor agent names with versions.
    pub harnesses: Vec<String>,
    pub framework_version: Option<String>,
    /// verifiers `num_rollouts` from the resolved config.
    pub rollouts_per_task: Option<u32>,
    /// Harbor: `finished_at` is present. verifiers has no end marker, so `None`.
    pub ended: Option<bool>,
    /// Job-level counts Harbor reports, to check against the trial files.
    pub reported: Option<ReportedCounts>,
    /// Lines or trials that could not be read.
    pub skipped: u32,
    pub torn_tail: bool,
    /// Schema notes such as "newer schema".
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedCounts {
    pub total: Option<u32>,
    pub errored: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub framework: Framework,
    pub facts: RunFacts,
    pub tasks: u32,
    pub rollouts: u32,
    pub errored: u32,
    pub timed_out: u32,
    pub unfinished: u32,
    /// Mean over rollouts that count; `None` when none do.
    pub mean_reward: Option<f64>,
    pub tokens: Tokens,
    pub cost: Option<f64>,
    pub started_at: Option<f64>,
}

pub fn summarize(framework: Framework, facts: RunFacts, rollouts: &[Rollout]) -> RunSummary {
    let mut tokens = Tokens::default();
    let mut cost: Option<f64> = None;
    let mut started_at: Option<f64> = None;
    let (mut errored, mut timed_out, mut unfinished) = (0, 0, 0);
    let mut tasks = std::collections::HashSet::new();
    for r in rollouts {
        tasks.insert(&r.task.key);
        tokens.add(&r.tokens);
        cost = r.cost.map(|c| c + cost.unwrap_or(0.0)).or(cost);
        started_at = match (started_at, r.started_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        match r.outcome {
            Outcome::Errored => errored += 1,
            Outcome::TimedOut => timed_out += 1,
            Outcome::Unfinished => unfinished += 1,
            Outcome::Completed => {}
        }
    }
    RunSummary {
        framework,
        facts,
        tasks: tasks.len() as u32,
        rollouts: rollouts.len() as u32,
        errored,
        timed_out,
        unfinished,
        mean_reward: mean(
            rollouts
                .iter()
                .filter(|r| r.counts_in_mean())
                .filter_map(|r| r.reward),
        ),
        tokens,
        cost,
        started_at,
    }
}

pub(crate) fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let (sum, n) = values.fold((0.0, 0u32), |(s, n), v| (s + v, n + 1));
    (n > 0).then(|| sum / f64::from(n))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RolloutCell {
    pub id: String,
    pub outcome: Outcome,
    pub reward: Option<f64>,
}

/// One task of a run: every rollout, the mean and the spread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRow {
    pub task: TaskId,
    pub rollouts: Vec<RolloutCell>,
    /// Over rollouts that count, so errored ones are not zeros.
    pub mean: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub counted: u32,
    pub errored: u32,
}

/// Tasks in first-seen order.
pub fn task_table(rollouts: &[Rollout]) -> Vec<TaskRow> {
    let mut rows: Vec<TaskRow> = Vec::new();
    for r in rollouts {
        let at = match rows.iter().position(|row| row.task.key == r.task.key) {
            Some(at) => at,
            None => {
                rows.push(TaskRow {
                    task: r.task.clone(),
                    rollouts: Vec::new(),
                    mean: None,
                    min: None,
                    max: None,
                    counted: 0,
                    errored: 0,
                });
                rows.len() - 1
            }
        };
        rows[at].rollouts.push(RolloutCell {
            id: r.id.clone(),
            outcome: r.outcome,
            reward: r.reward,
        });
    }
    for row in &mut rows {
        let counted: Vec<f64> = row
            .rollouts
            .iter()
            .filter(|c| matches!(c.outcome, Outcome::Completed | Outcome::TimedOut))
            .filter_map(|c| c.reward)
            .collect();
        row.counted = counted.len() as u32;
        row.errored = row.rollouts.iter().filter(|c| c.outcome.is_error()).count() as u32;
        row.min = counted.iter().copied().reduce(f64::min);
        row.max = counted.iter().copied().reduce(f64::max);
        row.mean = mean(counted.iter().copied());
    }
    rows
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompareRow {
    pub task: TaskId,
    pub a: Option<TaskRow>,
    pub b: Option<TaskRow>,
    /// `b.mean - a.mean`, when both runs have a mean for the task.
    pub delta: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub rows: Vec<CompareRow>,
    pub only_a: u32,
    pub only_b: u32,
    /// False when the runs hold different tasks; the header must say so.
    pub same_tasks: bool,
}

/// Match two runs of one framework by task key. Rows keep `a`'s order, then tasks only in `b`.
pub fn compare(a: &[TaskRow], b: &[TaskRow]) -> Comparison {
    let mut rows = Vec::new();
    for row_a in a {
        let row_b = b.iter().find(|r| r.task.key == row_a.task.key);
        rows.push(compare_row(row_a.task.clone(), Some(row_a), row_b));
    }
    for row_b in b {
        if !a.iter().any(|r| r.task.key == row_b.task.key) {
            rows.push(compare_row(row_b.task.clone(), None, Some(row_b)));
        }
    }
    let only_a = rows.iter().filter(|r| r.b.is_none()).count() as u32;
    let only_b = rows.iter().filter(|r| r.a.is_none()).count() as u32;
    Comparison {
        rows,
        only_a,
        only_b,
        same_tasks: only_a == 0 && only_b == 0,
    }
}

fn compare_row(task: TaskId, a: Option<&TaskRow>, b: Option<&TaskRow>) -> CompareRow {
    let delta = match (a.and_then(|r| r.mean), b.and_then(|r| r.mean)) {
        (Some(x), Some(y)) => Some(y - x),
        _ => None,
    };
    CompareRow {
        task,
        a: a.cloned(),
        b: b.cloned(),
        delta,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
    /// Plain log text, for Harbor trials with no trajectory.
    Log,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The raw string the model emitted (verifiers), or the arguments as JSON text (ATIF).
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Turn {
    pub role: Role,
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// For `Role::Tool`: the call this answers.
    pub tool_call_id: Option<String>,
    /// For `Role::Log`: the file's name.
    pub name: Option<String>,
}

/// A rollout as a readable document. Token ids and logprobs are never carried.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub turns: Vec<Turn>,
    pub rewards: Vec<RewardPart>,
    pub metrics: Vec<(String, f64)>,
    /// Phase name and seconds.
    pub timing: Vec<(String, f64)>,
    pub errors: Vec<ErrorInfo>,
    /// Other branches than the one shown (compaction, subagents).
    pub other_branches: u32,
    /// "newer schema" and similar; the document is best effort when set.
    pub notice: Option<String>,
    /// Top-level keys of a trace whose schema is newer than known.
    pub raw_keys: Vec<String>,
}

impl Document {
    /// Fill the footer from a rollout summary, for formats whose document file has no rewards.
    pub fn attach(&mut self, rollout: &Rollout) {
        self.rewards = rollout.rewards.clone();
        if let Some(e) = &rollout.error {
            self.errors.push(e.clone());
        }
    }
}

pub(crate) fn truncate(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rollout(id: &str, key: &str, outcome: Outcome, reward: Option<f64>) -> Rollout {
        Rollout {
            id: id.into(),
            task: TaskId {
                key: key.into(),
                label: key.into(),
                checksum: None,
            },
            outcome,
            stop: None,
            error: None,
            rewards: vec![],
            reward,
            tokens: Tokens::default(),
            cost: None,
            duration_secs: None,
            started_at: None,
            traces: 1,
            axes: Axes::default(),
            metrics: Vec::new(),
        }
    }

    #[test]
    fn errored_rollouts_are_counted_but_not_averaged() {
        // 1.0, 0.0 and an errored 0.9: the mean is 0.5, not 0.633 and not 0.333.
        let rows = task_table(&[
            rollout("a", "t", Outcome::Completed, Some(1.0)),
            rollout("b", "t", Outcome::Completed, Some(0.0)),
            rollout("c", "t", Outcome::Errored, Some(0.9)),
        ]);
        assert_eq!(rows[0].mean, Some(0.5));
        assert_eq!((rows[0].counted, rows[0].errored), (2, 1));
        assert_eq!((rows[0].min, rows[0].max), (Some(0.0), Some(1.0)));
    }

    #[test]
    fn a_task_with_only_errors_has_no_mean() {
        let rows = task_table(&[rollout("a", "t", Outcome::Errored, None)]);
        assert_eq!(rows[0].mean, None);
    }

    #[test]
    fn missing_reward_is_not_zero() {
        let all = [
            rollout("a", "t", Outcome::Completed, Some(1.0)),
            rollout("b", "t", Outcome::Completed, None),
        ];
        let s = summarize(Framework::Verifiers, RunFacts::default(), &all);
        assert_eq!(s.mean_reward, Some(1.0));
    }

    #[test]
    fn comparison_keeps_unmatched_tasks_and_says_so() {
        let a = task_table(&[
            rollout("a1", "x", Outcome::Completed, Some(0.25)),
            rollout("a2", "y", Outcome::Completed, Some(1.0)),
        ]);
        let b = task_table(&[
            rollout("b1", "x", Outcome::Completed, Some(0.75)),
            rollout("b2", "z", Outcome::Completed, Some(0.0)),
        ]);
        let c = compare(&a, &b);
        assert!(!c.same_tasks);
        assert_eq!((c.only_a, c.only_b), (1, 1));
        assert_eq!(c.rows.len(), 3);
        assert_eq!(c.rows[0].delta, Some(0.5));
        assert_eq!(c.rows[1].delta, None);
    }

    #[test]
    fn tokens_add_keeps_unknown_apart_from_zero() {
        let mut t = Tokens::default();
        t.add(&Tokens::default());
        assert_eq!(t.total(), None);
        t.add(&Tokens {
            input: Some(5),
            cached: None,
            output: Some(2),
        });
        assert_eq!((t.total(), t.cached), (Some(7), None));
    }
}
