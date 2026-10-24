//! What a set of rollouts adds up to, read the way verifiers' own `vf-eval` dashboard
//! reads a run (docs/research-view.md, "How a verifiers run goes"): each named reward's
//! mean raw score over the rollouts that did not error, and how every rollout ended,
//! by stop condition or by error type. A cell of the matrix and a run both use it.

use seamlessh_runs::{Outcome, Rollout};

/// One named reward over a set of rollouts.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RewardMean {
    pub name: String,
    /// The mean raw score, not score × weight, as verifiers breaks a reward down.
    pub mean: f64,
    /// The weight the rollouts gave it; `None` when unstated or when they disagree.
    pub weight: Option<f64>,
}

/// How some rollouts ended: a word and how many ended that way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Ending {
    pub word: String,
    pub count: u32,
    pub error: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Tally {
    pub rewards: Vec<RewardMean>,
    /// Most common first; errors after the endings that were not errors.
    pub endings: Vec<Ending>,
    /// The mean duration over rollouts that recorded one.
    pub mean_secs: Option<f64>,
}

pub(super) fn tally<'a>(rollouts: impl IntoIterator<Item = &'a Rollout>) -> Tally {
    let rollouts: Vec<&Rollout> = rollouts.into_iter().collect();
    // Errored rollouts carry no clean scores, so they stay out of the breakdown as they
    // stay out of the mean; a clean rollout without a reward its task names reads as 0.
    let clean: Vec<&Rollout> = rollouts
        .iter()
        .copied()
        .filter(|r| !r.outcome.is_error() && r.outcome != Outcome::Unfinished)
        .collect();
    let mut rewards: Vec<RewardMean> = Vec::new();
    for r in &clean {
        for part in &r.rewards {
            if !rewards.iter().any(|m| m.name == part.name) {
                rewards.push(RewardMean {
                    name: part.name.clone(),
                    mean: 0.,
                    weight: part.weight,
                });
            }
        }
    }
    for mean in &mut rewards {
        let mut sum = 0.;
        for r in &clean {
            if let Some(part) = r.rewards.iter().find(|p| p.name == mean.name) {
                sum += part.score;
                if part.weight != mean.weight {
                    mean.weight = None;
                }
            }
        }
        // why: `rewards` is only filled from `clean`, so it is not empty here.
        mean.mean = sum / clean.len() as f64;
    }

    let mut endings: Vec<Ending> = Vec::new();
    for r in &rollouts {
        let (word, error) = ending(r);
        match endings
            .iter_mut()
            .find(|e| e.word == word && e.error == error)
        {
            Some(e) => e.count += 1,
            None => endings.push(Ending {
                word,
                count: 1,
                error,
            }),
        }
    }
    // Stable, so equal counts keep the order they were first seen in.
    endings.sort_by(|a, b| a.error.cmp(&b.error).then(b.count.cmp(&a.count)));

    let secs: Vec<f64> = rollouts.iter().filter_map(|r| r.duration_secs).collect();
    let mean_secs = (!secs.is_empty()).then(|| secs.iter().sum::<f64>() / secs.len() as f64);
    Tally {
        rewards,
        endings,
        mean_secs,
    }
}

/// How one rollout ended, in a word: its error's type, else its stop condition.
pub(super) fn ending(r: &Rollout) -> (String, bool) {
    match r.outcome {
        Outcome::Errored => (
            r.error
                .as_ref()
                .map(|e| e.kind.clone())
                .filter(|k| !k.is_empty())
                .unwrap_or_else(|| "error".to_owned()),
            true,
        ),
        Outcome::TimedOut => ("timed out".to_owned(), false),
        Outcome::Unfinished => ("unfinished".to_owned(), false),
        Outcome::Completed => (
            r.stop
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "completed".to_owned()),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seamlessh_runs::{Axes, ErrorInfo, RewardPart, TaskId, Tokens};

    fn rollout(outcome: Outcome, stop: Option<&str>, parts: &[(&str, f64)]) -> Rollout {
        Rollout {
            id: "r".to_owned(),
            task: TaskId {
                key: "t".to_owned(),
                label: "t".to_owned(),
                checksum: None,
            },
            outcome,
            stop: stop.map(str::to_owned),
            error: outcome.is_error().then(|| ErrorInfo {
                kind: "ModelError".to_owned(),
                message: String::new(),
            }),
            rewards: parts
                .iter()
                .map(|(name, score)| RewardPart {
                    name: (*name).to_owned(),
                    score: *score,
                    weight: Some(1.),
                })
                .collect(),
            reward: Some(parts.iter().map(|(_, s)| s).sum()),
            tokens: Tokens::default(),
            cost: None,
            duration_secs: Some(10.),
            started_at: None,
            traces: 1,
            axes: Axes::default(),
        }
    }

    #[test]
    fn rewards_break_down_over_clean_rollouts_with_a_missing_part_as_zero() {
        let rs = [
            rollout(
                Outcome::Completed,
                Some("agent_completed"),
                &[("exact", 1.), ("format", 1.)],
            ),
            rollout(Outcome::Completed, Some("max_turns"), &[("exact", 0.)]),
            rollout(Outcome::Errored, None, &[("exact", 1.)]),
        ];
        let t = tally(&rs);
        assert_eq!(t.rewards.len(), 2);
        assert_eq!(
            (t.rewards[0].name.as_str(), t.rewards[0].mean),
            ("exact", 0.5)
        );
        assert_eq!(
            (t.rewards[1].name.as_str(), t.rewards[1].mean),
            ("format", 0.5)
        );
        assert_eq!(t.mean_secs, Some(10.));
    }

    #[test]
    fn endings_count_stop_conditions_then_error_types() {
        let rs = [
            rollout(Outcome::Errored, None, &[]),
            rollout(Outcome::Completed, Some("max_turns"), &[]),
            rollout(Outcome::Completed, Some("agent_completed"), &[]),
            rollout(Outcome::Completed, Some("agent_completed"), &[]),
        ];
        let words: Vec<(String, u32, bool)> = tally(&rs)
            .endings
            .into_iter()
            .map(|e| (e.word, e.count, e.error))
            .collect();
        assert_eq!(
            words,
            vec![
                ("agent_completed".to_owned(), 2, false),
                ("max_turns".to_owned(), 1, false),
                ("ModelError".to_owned(), 1, true),
            ]
        );
    }

    #[test]
    fn nothing_clean_means_no_breakdown() {
        let t = tally(&[rollout(Outcome::Errored, None, &[("exact", 1.)])]);
        assert!(t.rewards.is_empty());
        assert_eq!(t.endings.len(), 1);
    }
}
