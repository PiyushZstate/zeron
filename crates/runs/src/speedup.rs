//! How much faster one run is than a baseline, task by task: the measure kernel work
//! is judged by (research/2026-10-25-research-users.md in Seamlessh). A kernel counts
//! only when it is correct, so only rollouts that scored above zero are timed, and the
//! tasks combine by their geometric mean, as KernelBench and METR report it: a 2×
//! gain and a 2× loss cancel rather than average to 1.25×.

use serde::{Deserialize, Serialize};

use crate::model::{Rollout, TaskId};

/// Timing metrics, lower is faster, in the order a run is searched for one.
const TIME_METRICS: [&str; 10] = [
    "runtime_ms",
    "runtime_us",
    "runtime",
    "kernel_time_ms",
    "kernel_time",
    "latency_ms",
    "latency",
    "time_ms",
    "elapsed_ms",
    "duration_ms",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpeedup {
    pub task: TaskId,
    /// Mean of the metric over the baseline's correct rollouts.
    pub base: f64,
    /// Mean of the metric over the run's correct rollouts.
    pub run: f64,
    /// `base / run`: above 1 is faster.
    pub ratio: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Speedup {
    /// The metric both runs were timed by.
    pub metric: String,
    /// Tasks both runs solved correctly, in the run's order.
    pub tasks: Vec<TaskSpeedup>,
    /// Geometric mean of the ratios.
    pub geomean: f64,
    pub faster: u32,
    pub slower: u32,
    /// Tasks the baseline solved and the run did not: a speedup there means nothing,
    /// so they are counted apart rather than timed.
    pub lost: u32,
}

/// `None` when the two runs share no timing metric or no task both solved correctly.
pub fn speedup(base: &[Rollout], run: &[Rollout]) -> Option<Speedup> {
    let metric = TIME_METRICS
        .iter()
        .find(|m| has_metric(base, m) && has_metric(run, m))?;
    let base_times = times(base, metric);
    let run_times = times(run, metric);
    let mut tasks = Vec::new();
    let mut lost = 0;
    for (task, b) in &base_times {
        match run_times.iter().find(|(t, _)| t.key == task.key) {
            Some((_, r)) if *r > 0.0 && *b > 0.0 => tasks.push(TaskSpeedup {
                task: task.clone(),
                base: *b,
                run: *r,
                ratio: b / r,
            }),
            Some(_) => {}
            None if solved_somewhere(run, &task.key) => {}
            None => lost += 1,
        }
    }
    if tasks.is_empty() {
        return None;
    }
    let logs: f64 = tasks.iter().map(|t| t.ratio.ln()).sum();
    let geomean = (logs / tasks.len() as f64).exp();
    Some(Speedup {
        metric: (*metric).to_owned(),
        faster: tasks.iter().filter(|t| t.ratio > 1.0).count() as u32,
        slower: tasks.iter().filter(|t| t.ratio < 1.0).count() as u32,
        geomean,
        tasks,
        lost,
    })
}

fn metric_of(rollout: &Rollout, name: &str) -> Option<f64> {
    rollout
        .metrics
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| *v)
        .filter(|v| v.is_finite())
}

fn correct(rollout: &Rollout) -> bool {
    rollout.counts_in_mean() && rollout.reward.is_some_and(|r| r > 0.0)
}

fn has_metric(rollouts: &[Rollout], name: &str) -> bool {
    rollouts.iter().any(|r| metric_of(r, name).is_some())
}

/// A task the run attempted at all, so its absence from the timed tasks is a
/// missing metric rather than a lost task.
fn solved_somewhere(rollouts: &[Rollout], key: &str) -> bool {
    rollouts.iter().any(|r| r.task.key == key && correct(r))
}

/// Each task's mean time over its correct rollouts, first-seen order.
fn times(rollouts: &[Rollout], metric: &str) -> Vec<(TaskId, f64)> {
    let mut sums: Vec<(TaskId, f64, u32)> = Vec::new();
    for r in rollouts.iter().filter(|r| correct(r)) {
        let Some(v) = metric_of(r, metric) else {
            continue;
        };
        match sums.iter_mut().find(|(t, _, _)| t.key == r.task.key) {
            Some((_, sum, n)) => {
                *sum += v;
                *n += 1;
            }
            None => sums.push((r.task.clone(), v, 1)),
        }
    }
    sums.into_iter()
        .map(|(t, sum, n)| (t, sum / f64::from(n)))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::model::{Axes, Outcome, Tokens};

    fn kernel(task: &str, reward: f64, ms: Option<f64>) -> Rollout {
        Rollout {
            id: format!("{task}-{reward}"),
            task: TaskId {
                key: task.into(),
                label: task.into(),
                checksum: None,
            },
            outcome: Outcome::Completed,
            stop: None,
            error: None,
            rewards: vec![],
            reward: Some(reward),
            tokens: Tokens::default(),
            cost: None,
            duration_secs: None,
            started_at: None,
            traces: 1,
            axes: Axes::default(),
            metrics: ms
                .map(|v| vec![("runtime_ms".to_owned(), v)])
                .unwrap_or_default(),
        }
    }

    #[test]
    fn a_gain_and_an_equal_loss_cancel_in_the_geometric_mean() {
        let base = [kernel("a", 1.0, Some(10.0)), kernel("b", 1.0, Some(10.0))];
        let run = [kernel("a", 1.0, Some(5.0)), kernel("b", 1.0, Some(20.0))];
        let s = speedup(&base, &run).unwrap();
        assert_eq!(s.metric, "runtime_ms");
        assert!((s.geomean - 1.0).abs() < 1e-12);
        assert_eq!((s.faster, s.slower, s.lost), (1, 1, 0));
        assert!((s.tasks[0].ratio - 2.0).abs() < 1e-12);
    }

    #[test]
    fn only_correct_rollouts_are_timed() {
        // The run's fast attempt at `a` was wrong; its correct one took as long as the base.
        let base = [kernel("a", 1.0, Some(10.0))];
        let run = [kernel("a", 0.0, Some(1.0)), kernel("a", 1.0, Some(10.0))];
        let s = speedup(&base, &run).unwrap();
        assert!((s.geomean - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_task_the_run_lost_is_counted_not_timed() {
        let base = [kernel("a", 1.0, Some(10.0)), kernel("b", 1.0, Some(10.0))];
        let run = [kernel("a", 1.0, Some(5.0)), kernel("b", 0.0, Some(1.0))];
        let s = speedup(&base, &run).unwrap();
        assert_eq!(s.tasks.len(), 1);
        assert_eq!(s.lost, 1);
        assert!((s.geomean - 2.0).abs() < 1e-12);
    }

    #[test]
    fn rollouts_are_averaged_within_a_task() {
        let base = [kernel("a", 1.0, Some(10.0))];
        let run = [kernel("a", 1.0, Some(4.0)), kernel("a", 1.0, Some(6.0))];
        assert!((speedup(&base, &run).unwrap().geomean - 2.0).abs() < 1e-12);
    }

    #[test]
    fn no_shared_metric_or_no_shared_solved_task_gives_none() {
        let timed = [kernel("a", 1.0, Some(10.0))];
        let untimed = [kernel("a", 1.0, None)];
        assert!(speedup(&timed, &untimed).is_none());
        let other = [kernel("z", 1.0, Some(10.0))];
        assert!(speedup(&timed, &other).is_none());
    }
}
