//! Task qualification: the reference solution must pass every time, and doing
//! nothing must score zero (docs: research/2026-10-25-research-users.md in Seamlessh).
//! Harbor runs these as the `oracle` and `nop` agents.

use serde::{Deserialize, Serialize};

use crate::model::{Rollout, TaskId};

/// One fixed agent's rollouts on one task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Side {
    pub rollouts: u32,
    /// Oracle: scored full reward. No-op: scored above zero.
    pub passed: u32,
    pub errored: u32,
}

/// What the two fixed agents say about a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// The oracle always passed and the no-op never did.
    Sound,
    /// The oracle missed full reward or errored at least once: the task or its
    /// solution is broken.
    OracleFails,
    /// The no-op scored above zero: the grader passes untouched work.
    NopPasses,
    /// Both of the above.
    Both,
    /// No fault so far, but one of the two agents has not run: half the check.
    Unchecked,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskCheck {
    pub task: TaskId,
    pub oracle: Option<Side>,
    pub nop: Option<Side>,
    pub verdict: Verdict,
}

/// Rewards are floats written by a verifier; an exact compare would miss `0.9999999999`.
const EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, PartialEq)]
enum Fixed {
    Oracle,
    Nop,
}

/// Harbor writes the agent as `<name>` or `<name> <version>`.
fn fixed_agent(rollout: &Rollout) -> Option<Fixed> {
    let word = rollout.axes.harness.as_deref()?.split_whitespace().next()?;
    match word.to_ascii_lowercase().as_str() {
        "oracle" => Some(Fixed::Oracle),
        "nop" | "noop" | "no-op" => Some(Fixed::Nop),
        _ => None,
    }
}

fn tally(side: &mut Option<Side>, rollout: &Rollout, passes: impl Fn(f64) -> bool) {
    let side = side.get_or_insert_with(Side::default);
    side.rollouts += 1;
    side.errored += u32::from(rollout.outcome.is_error());
    side.passed += u32::from(rollout.reward.is_some_and(passes));
}

/// One check per task that an oracle or no-op agent ran, first-seen order.
pub fn qualify(rollouts: &[Rollout]) -> Vec<TaskCheck> {
    let mut checks: Vec<TaskCheck> = Vec::new();
    for rollout in rollouts {
        let Some(agent) = fixed_agent(rollout) else {
            continue;
        };
        let at = match checks.iter().position(|c| c.task.key == rollout.task.key) {
            Some(at) => at,
            None => {
                checks.push(TaskCheck {
                    task: rollout.task.clone(),
                    oracle: None,
                    nop: None,
                    verdict: Verdict::Unchecked,
                });
                checks.len() - 1
            }
        };
        match agent {
            Fixed::Oracle => tally(&mut checks[at].oracle, rollout, |r| r >= 1.0 - EPSILON),
            Fixed::Nop => tally(&mut checks[at].nop, rollout, |r| r > EPSILON),
        }
    }
    for check in &mut checks {
        check.verdict = verdict(check.oracle.as_ref(), check.nop.as_ref());
    }
    checks
}

fn verdict(oracle: Option<&Side>, nop: Option<&Side>) -> Verdict {
    // A fault needs only its own side: an oracle that failed has failed whether or
    // not the no-op has run yet.
    // An errored oracle rollout cannot count as a pass even when it kept a reward.
    let oracle_fails = oracle.is_some_and(|o| o.errored > 0 || o.passed < o.rollouts);
    let nop_passes = nop.is_some_and(|n| n.passed > 0);
    match (oracle_fails, nop_passes) {
        (true, true) => Verdict::Both,
        (true, false) => Verdict::OracleFails,
        (false, true) => Verdict::NopPasses,
        (false, false) if oracle.is_some() && nop.is_some() => Verdict::Sound,
        (false, false) => Verdict::Unchecked,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::model::{Axes, Outcome, Tokens};

    fn rollout(task: &str, agent: Option<&str>, outcome: Outcome, reward: Option<f64>) -> Rollout {
        Rollout {
            id: format!("{task}__{}", agent.unwrap_or("none")),
            task: TaskId {
                key: task.into(),
                label: task.into(),
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
            axes: Axes {
                harness: agent.map(str::to_owned),
                ..Axes::default()
            },
            metrics: Vec::new(),
        }
    }

    fn done(task: &str, agent: &str, reward: f64) -> Rollout {
        rollout(task, Some(agent), Outcome::Completed, Some(reward))
    }

    fn only(rollouts: &[Rollout]) -> TaskCheck {
        let mut checks = qualify(rollouts);
        assert_eq!(checks.len(), 1);
        checks.remove(0)
    }

    #[test]
    fn a_passing_oracle_and_a_zero_nop_is_sound() {
        let check = only(&[
            done("t", "oracle", 1.0),
            done("t", "oracle", 1.0),
            done("t", "nop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::Sound);
        let oracle = check.oracle.unwrap();
        assert_eq!((oracle.rollouts, oracle.passed, oracle.errored), (2, 2, 0));
        assert_eq!(check.nop.unwrap().passed, 0);
    }

    #[test]
    fn an_oracle_below_full_reward_fails() {
        let check = only(&[
            done("t", "oracle", 1.0),
            done("t", "oracle", 0.5),
            done("t", "nop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::OracleFails);
        assert_eq!(check.oracle.unwrap().passed, 1);
    }

    #[test]
    fn an_oracle_without_a_reward_fails() {
        let check = only(&[
            rollout("t", Some("oracle"), Outcome::Unfinished, None),
            done("t", "nop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::OracleFails);
    }

    #[test]
    fn an_errored_oracle_fails_even_with_a_full_reward() {
        let check = only(&[
            rollout("t", Some("oracle"), Outcome::Errored, Some(1.0)),
            done("t", "oracle", 1.0),
            done("t", "nop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::OracleFails);
        let oracle = check.oracle.unwrap();
        assert_eq!((oracle.passed, oracle.errored), (2, 1));
    }

    #[test]
    fn a_nop_above_zero_passes_untouched_work() {
        let check = only(&[done("t", "oracle", 1.0), done("t", "nop", 0.25)]);
        assert_eq!(check.verdict, Verdict::NopPasses);
        assert_eq!(check.nop.unwrap().passed, 1);
    }

    #[test]
    fn both_faults_are_reported_together() {
        let check = only(&[done("t", "oracle", 0.0), done("t", "nop", 1.0)]);
        assert_eq!(check.verdict, Verdict::Both);
    }

    #[test]
    fn an_errored_nop_does_not_pass_and_is_counted() {
        let check = only(&[
            done("t", "oracle", 1.0),
            rollout("t", Some("nop"), Outcome::Errored, None),
            done("t", "nop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::Sound);
        let nop = check.nop.unwrap();
        assert_eq!((nop.rollouts, nop.passed, nop.errored), (2, 0, 1));
    }

    #[test]
    fn one_side_or_none_is_unchecked() {
        assert_eq!(
            only(&[done("t", "oracle", 1.0)]).verdict,
            Verdict::Unchecked
        );
        // A side at fault is a fault before the other side runs.
        assert_eq!(
            only(&[done("t", "oracle", 0.0)]).verdict,
            Verdict::OracleFails
        );
        assert_eq!(only(&[done("t", "nop", 1.0)]).verdict, Verdict::NopPasses);
        let nop_only = only(&[done("t", "nop", 0.0)]);
        assert_eq!(nop_only.verdict, Verdict::Unchecked);
        assert!(nop_only.oracle.is_none());
        assert!(qualify(&[done("t", "claude-code 2.1", 1.0)]).is_empty());
    }

    #[test]
    fn agent_names_ignore_version_and_case() {
        let check = only(&[
            done("t", "Oracle 1.0.0", 1.0),
            done("t", "NOP 1.0.0", 0.0),
            done("t", "no-op", 0.0),
            done("t", "noop", 0.0),
        ]);
        assert_eq!(check.verdict, Verdict::Sound);
        assert_eq!(check.oracle.unwrap().rollouts, 1);
        assert_eq!(check.nop.unwrap().rollouts, 3);
    }

    #[test]
    fn other_agents_and_unnamed_rollouts_are_ignored() {
        let check = only(&[
            done("t", "oracle", 1.0),
            done("t", "nop", 0.0),
            done("t", "oracle-like", 1.0),
            done("t", "claude-code", 0.0),
            rollout("t", None, Outcome::Completed, Some(1.0)),
            done("other", "claude-code", 1.0),
        ]);
        assert_eq!(check.verdict, Verdict::Sound);
        assert_eq!(check.oracle.unwrap().rollouts, 1);
        assert_eq!(check.nop.unwrap().rollouts, 1);
    }

    #[test]
    fn tolerance_applies_to_both_thresholds() {
        let check = only(&[done("t", "oracle", 1.0 - 1e-12), done("t", "nop", 1e-12)]);
        assert_eq!(check.verdict, Verdict::Sound);
        let slack = only(&[done("t", "oracle", 1.0 - 1e-6), done("t", "nop", 1e-6)]);
        assert_eq!(slack.verdict, Verdict::Both);
    }

    #[test]
    fn tasks_come_out_in_first_seen_order() {
        let checks = qualify(&[
            done("b", "nop", 0.0),
            done("a", "oracle", 1.0),
            done("b", "oracle", 1.0),
            done("a", "nop", 0.0),
            done("c", "claude-code", 1.0),
        ]);
        let keys: Vec<_> = checks.iter().map(|c| c.task.key.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
    }
}
