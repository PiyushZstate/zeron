//! Harbor 0.24.0 job and trial directories (docs/research-view.md, "Formats read").
//!
//! The host lists a job directory and hands each file's text here; this module never touches
//! the file system. A trial is finished when its `result.json` exists. Trial `result.json` is
//! written with `model_dump_json`, so absent values appear as `null`, and every declared key is
//! optional here. Harbor's datetimes may carry no zone; they read as UTC.

use serde::Deserialize;
use serde_json::Value;

use crate::model::{
    Axes, Document, ErrorInfo, MESSAGE_LIMIT, Outcome, ReportedCounts, RewardPart, Role, Rollout,
    RunFacts, TaskId, Tokens, ToolCall, Turn, truncate,
};
use crate::time::parse_iso;
use crate::verifiers::ParsedRun;

/// File names inside a job or trial directory.
pub const RESULT_FILE: &str = "result.json";
pub const CONFIG_FILE: &str = "config.json";
pub const LOCK_FILE: &str = "lock.json";
pub const TRAJECTORY_FILE: &str = "agent/trajectory.json";

#[derive(Deserialize)]
struct TrialResultFile {
    task_name: Option<String>,
    trial_name: Option<String>,
    source: Option<String>,
    task_checksum: Option<String>,
    agent_info: Option<AgentInfo>,
    agent_result: Option<AgentContext>,
    verifier_result: Option<VerifierResult>,
    exception_info: Option<ExceptionInfo>,
    started_at: Option<String>,
    finished_at: Option<String>,
    step_results: Option<Vec<StepResult>>,
}

#[derive(Deserialize)]
struct AgentInfo {
    name: Option<String>,
    version: Option<String>,
    model_info: Option<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: Option<String>,
    provider: Option<String>,
}

#[derive(Deserialize)]
struct AgentContext {
    n_input_tokens: Option<u64>,
    n_cache_tokens: Option<u64>,
    n_output_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

#[derive(Deserialize)]
struct VerifierResult {
    rewards: Option<serde_json::Map<String, Value>>,
}

#[derive(Deserialize)]
struct ExceptionInfo {
    exception_type: Option<String>,
    exception_message: Option<String>,
}

#[derive(Deserialize)]
struct StepResult {
    agent_result: Option<AgentContext>,
}

/// A finished trial. Its `result.json` text, as written by Harbor.
pub fn parse_trial_result(json: &str) -> Result<Rollout, String> {
    parse_trial(json).map(|(rollout, _)| rollout)
}

struct TrialFacts {
    source: Option<String>,
    model: Option<String>,
    harness: Option<String>,
}

fn parse_trial(json: &str) -> Result<(Rollout, TrialFacts), String> {
    let file: TrialResultFile = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let name = file.trial_name.clone().unwrap_or_default();
    let task_name = file
        .task_name
        .clone()
        .unwrap_or_else(|| task_from_trial_name(&name));

    let rewards: Vec<RewardPart> = file
        .verifier_result
        .as_ref()
        .and_then(|v| v.rewards.as_ref())
        .map(|map| {
            map.iter()
                .filter_map(|(n, v)| {
                    Some(RewardPart {
                        name: n.clone(),
                        score: v.as_f64()?,
                        weight: None,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let reward = headline(&rewards);

    // `compute_token_cost_totals`: the trial's own context, else the sum of its steps'.
    // `n_input_tokens` already includes cache reads.
    let contexts: Vec<&AgentContext> = match (&file.agent_result, &file.step_results) {
        (Some(c), _) => vec![c],
        (None, Some(steps)) => steps
            .iter()
            .filter_map(|s| s.agent_result.as_ref())
            .collect(),
        (None, None) => Vec::new(),
    };
    let mut tokens = Tokens::default();
    let mut cost: Option<f64> = None;
    for c in contexts {
        tokens.add(&Tokens {
            input: c.n_input_tokens,
            cached: c.n_cache_tokens,
            output: c.n_output_tokens,
        });
        cost = c.cost_usd.map(|x| x + cost.unwrap_or(0.0)).or(cost);
    }

    let started = file.started_at.as_deref().and_then(parse_iso);
    let finished = file.finished_at.as_deref().and_then(parse_iso);
    let error = file.exception_info.as_ref().map(|e| ErrorInfo {
        kind: e.exception_type.clone().unwrap_or_default(),
        message: truncate(
            e.exception_message.as_deref().unwrap_or_default(),
            MESSAGE_LIMIT,
        ),
    });
    let agent = file.agent_info.as_ref();
    let facts = TrialFacts {
        source: file.source.clone(),
        model: agent.and_then(|a| a.model_info.as_ref()).and_then(|m| {
            let name = m.name.clone()?;
            Some(match &m.provider {
                Some(p) => format!("{p}/{name}"),
                None => name,
            })
        }),
        harness: agent.and_then(|a| {
            let name = a.name.clone()?;
            Some(match &a.version {
                Some(v) if !v.is_empty() => format!("{name} {v}"),
                _ => name,
            })
        }),
    };
    let keep = |s: Option<String>| s.filter(|s| !s.is_empty());
    let rollout = Rollout {
        id: name,
        task: TaskId {
            key: task_name.clone(),
            label: task_name,
            checksum: file.task_checksum,
        },
        outcome: if error.is_some() {
            Outcome::Errored
        } else {
            Outcome::Completed
        },
        stop: None,
        error,
        rewards,
        reward,
        tokens,
        cost,
        duration_secs: match (started, finished) {
            (Some(s), Some(f)) if f >= s => Some(f - s),
            _ => None,
        },
        started_at: started,
        traces: 1,
        axes: Axes {
            env: keep(facts.source.clone()),
            harness: keep(facts.harness.clone()),
            model: keep(facts.model.clone()),
            runtime: None,
        },
    };
    Ok((rollout, facts))
}

/// Harbor defines no sum over rewards: the one named `reward`, else the only one.
fn headline(rewards: &[RewardPart]) -> Option<f64> {
    rewards
        .iter()
        .find(|r| r.name == "reward")
        .or_else(|| (rewards.len() == 1).then(|| &rewards[0]))
        .map(|r| r.score)
}

/// Trial names are `<task_name[:32]>__<7 characters>`.
fn task_from_trial_name(trial: &str) -> String {
    trial
        .rsplit_once("__")
        .map_or(trial, |(task, _)| task)
        .to_owned()
}

/// A trial directory with no `result.json`: still running, or abandoned. `config` is its
/// `config.json` when present, for the task name.
pub fn unfinished_trial(trial_name: &str, config: Option<&str>) -> Rollout {
    #[derive(Deserialize)]
    struct Config {
        task: Option<Task>,
    }
    #[derive(Deserialize)]
    struct Task {
        name: Option<String>,
        path: Option<String>,
    }
    let task = config
        .and_then(|c| serde_json::from_str::<Config>(c).ok())
        .and_then(|c| c.task)
        .and_then(|t| {
            t.name.or_else(|| {
                t.path.and_then(|p| {
                    p.trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .map(str::to_owned)
                })
            })
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| task_from_trial_name(trial_name));
    Rollout {
        id: trial_name.to_owned(),
        task: TaskId {
            key: task.clone(),
            label: task,
            checksum: None,
        },
        outcome: Outcome::Unfinished,
        stop: None,
        error: None,
        rewards: Vec::new(),
        reward: None,
        tokens: Tokens::default(),
        cost: None,
        duration_secs: None,
        started_at: None,
        traces: 1,
        axes: Axes::default(),
    }
}

/// One reading of a job directory. Feed it the job's files and each trial, then finish.
pub struct JobScan {
    facts: RunFacts,
    rollouts: Vec<Rollout>,
}

impl JobScan {
    /// `job_name` is the directory's name, which Harbor uses as the job name.
    pub fn new(job_name: &str) -> Self {
        let facts = RunFacts {
            name: Some(job_name.to_owned()),
            ..RunFacts::default()
        };
        Self {
            facts,
            rollouts: Vec::new(),
        }
    }

    /// The job-level `result.json`. It is rewritten while the job runs; an unreadable copy
    /// is noted and otherwise ignored, since the trial files hold the rollouts.
    pub fn job_result(&mut self, json: &str) {
        #[derive(Deserialize)]
        struct JobResult {
            finished_at: Option<String>,
            n_total_trials: Option<u32>,
            stats: Option<Stats>,
        }
        #[derive(Deserialize)]
        struct Stats {
            n_errored_trials: Option<u32>,
            // Older files; `JobStats` migrates them the same way.
            n_errors: Option<u32>,
        }
        let Ok(job) = serde_json::from_str::<JobResult>(json) else {
            self.facts
                .notes
                .push("job result.json could not be read".to_owned());
            return;
        };
        self.facts.ended = Some(job.finished_at.is_some());
        self.facts.reported = Some(ReportedCounts {
            total: job.n_total_trials,
            errored: job.stats.and_then(|s| s.n_errored_trials.or(s.n_errors)),
        });
    }

    /// The job-level `lock.json`, for the Harbor version that ran it.
    pub fn lock(&mut self, json: &str) {
        #[derive(Deserialize)]
        struct Lock {
            harbor: Option<Info>,
        }
        #[derive(Deserialize)]
        struct Info {
            version: Option<String>,
        }
        self.facts.framework_version = serde_json::from_str::<Lock>(json)
            .ok()
            .and_then(|l| l.harbor?.version)
            .or(self.facts.framework_version.take());
    }

    /// One trial directory: its `result.json` if it exists, else its `config.json`.
    pub fn trial(&mut self, trial_name: &str, result: Option<&str>, config: Option<&str>) {
        let Some(result) = result else {
            self.rollouts.push(unfinished_trial(trial_name, config));
            return;
        };
        match parse_trial(result) {
            Ok((mut rollout, facts)) => {
                if rollout.id.is_empty() {
                    rollout.id = trial_name.to_owned();
                }
                let f = &mut self.facts;
                f.env = f.env.take().or(facts.source);
                for (list, item) in [
                    (&mut f.models, facts.model),
                    (&mut f.harnesses, facts.harness),
                ] {
                    if let Some(item) = item.filter(|i| !list.contains(i)) {
                        list.push(item);
                    }
                }
                self.rollouts.push(rollout);
            }
            Err(_) => self.facts.skipped += 1,
        }
    }

    pub fn finish(mut self) -> ParsedRun {
        // Trial directory listing order is arbitrary; show trials as they started.
        self.rollouts.sort_by(|a, b| {
            a.started_at
                .unwrap_or(f64::INFINITY)
                .total_cmp(&b.started_at.unwrap_or(f64::INFINITY))
                .then_with(|| a.id.cmp(&b.id))
        });
        // An unfinished trial knows its task only by path or trial name (`write-file`);
        // a finished one has the full name (`org/write-file`). Same task, one row.
        let known: Vec<TaskId> = self
            .rollouts
            .iter()
            .filter(|r| r.outcome != Outcome::Unfinished)
            .map(|r| r.task.clone())
            .collect();
        for r in self
            .rollouts
            .iter_mut()
            .filter(|r| r.outcome == Outcome::Unfinished)
        {
            let short = r.task.key.clone();
            if let Some(full) = known
                .iter()
                .find(|t| t.key == short || t.key.ends_with(&format!("/{short}")))
            {
                r.task = TaskId {
                    checksum: None,
                    ..full.clone()
                };
            }
        }
        if self.facts.ended.is_none() {
            self.facts.ended = Some(false);
        }
        ParsedRun {
            facts: self.facts,
            rollouts: self.rollouts,
        }
    }
}

// ---- document -------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Trajectory {
    schema_version: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
    #[serde(default)]
    subagent_trajectories: Vec<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct Step {
    source: Option<String>,
    message: Option<StepContent>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<AtifToolCall>>,
    observation: Option<Observation>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StepContent {
    Text(String),
    Parts(Vec<ContentPart>),
    Other(serde::de::IgnoredAny),
}

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

impl StepContent {
    fn text(&self) -> String {
        match self {
            StepContent::Text(t) => t.clone(),
            StepContent::Parts(parts) => parts
                .iter()
                .map(|p| match (p.kind.as_deref(), &p.text) {
                    (_, Some(t)) => t.clone(),
                    (Some(kind), None) => format!("[{kind}]"),
                    (None, None) => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            StepContent::Other(_) => String::new(),
        }
    }
}

#[derive(Deserialize)]
struct AtifToolCall {
    tool_call_id: Option<String>,
    function_name: Option<String>,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
struct Observation {
    #[serde(default)]
    results: Vec<ObservationResult>,
}

#[derive(Deserialize)]
struct ObservationResult {
    source_call_id: Option<String>,
    content: Option<StepContent>,
}

/// `agent/trajectory.json` (ATIF) as a document. A step's observation results follow it as
/// tool turns. Schema versions other than `ATIF-v1.x` are read best effort with a notice.
pub fn document(trajectory: &str) -> Result<Document, String> {
    let file: Trajectory = serde_json::from_str(trajectory).map_err(|e| e.to_string())?;
    let mut doc = Document::default();
    match file.schema_version.as_deref() {
        Some(v) if v.starts_with("ATIF-v1.") => {}
        other => {
            doc.notice = Some(format!(
                "unsupported trajectory schema {}",
                other.unwrap_or("(none)")
            ))
        }
    }
    doc.other_branches = file.subagent_trajectories.len() as u32;
    for step in &file.steps {
        let role = match step.source.as_deref() {
            Some("system") => Role::System,
            Some("user") => Role::User,
            _ => Role::Assistant,
        };
        doc.turns.push(Turn {
            role,
            text: step
                .message
                .as_ref()
                .map(StepContent::text)
                .unwrap_or_default(),
            reasoning: step.reasoning_content.clone().filter(|r| !r.is_empty()),
            tool_calls: step
                .tool_calls
                .iter()
                .flatten()
                .map(|c| ToolCall {
                    id: c.tool_call_id.clone().unwrap_or_default(),
                    name: c.function_name.clone().unwrap_or_default(),
                    arguments: c
                        .arguments
                        .as_ref()
                        .map(Value::to_string)
                        .unwrap_or_default(),
                })
                .collect(),
            tool_call_id: None,
            name: None,
        });
        for result in step.observation.iter().flat_map(|o| &o.results) {
            doc.turns.push(Turn {
                role: Role::Tool,
                text: result
                    .content
                    .as_ref()
                    .map(StepContent::text)
                    .unwrap_or_default(),
                reasoning: None,
                tool_calls: Vec::new(),
                tool_call_id: result.source_call_id.clone(),
                name: None,
            });
        }
    }
    Ok(doc)
}

/// Agents that write no trajectory leave logs; each file becomes one plain-text turn.
pub fn log_document(files: &[(String, String)]) -> Document {
    Document {
        turns: files
            .iter()
            .map(|(name, text)| Turn {
                role: Role::Log,
                text: text.clone(),
                reasoning: None,
                tool_calls: Vec::new(),
                tool_call_id: None,
                name: Some(name.clone()),
            })
            .collect(),
        ..Document::default()
    }
}

#[cfg(test)]
mod tests;
