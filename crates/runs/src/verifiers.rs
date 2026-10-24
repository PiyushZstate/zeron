//! verifiers v0.4.0 `traces.jsonl`: one `Episode` per line (docs/research-view.md, "Formats read").
//!
//! The summary parser declares only the keys the list and comparison views need. serde skips
//! every other key, including each node's `token_ids`, `mask`, `logprobs` and the like, without
//! allocating, so a line costs about its summary and not its size. The document is built on
//! demand from one line, and again leaves the token streams undeclared.
//!
//! `write_episode` dumps with `exclude_none=True`, so a key whose value was `None` is absent;
//! every declared key is optional and absent never reads as `false` or zero.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{
    Axes, Document, ErrorInfo, MESSAGE_LIMIT, Outcome, RewardPart, Role, Rollout, RunFacts, TaskId,
    Tokens, ToolCall, Turn, truncate,
};

/// The trace schema version this reader knows.
pub const KNOWN_TRACE_VERSION: u32 = 1;

/// The file a verifiers run is recognized by.
pub const TRACES_FILE: &str = "traces.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineError {
    /// Not JSON, not an object, or a key with an unusable type.
    Invalid(String),
    /// No episode in this line has anything to show.
    Empty,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineError::Invalid(why) => write!(f, "unreadable line: {why}"),
            LineError::Empty => f.write_str("empty line"),
        }
    }
}

impl std::error::Error for LineError {}

/// A mapping read in file order. `rewards` order is the order the env declared them.
struct Ordered<T>(Vec<(String, T)>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Ordered<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Ordered<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry::<String, T>()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(V(PhantomData))
    }
}

/// The keys of an object and nothing else; values are skipped.
struct Keys(Vec<String>);

impl<'de> Deserialize<'de> for Keys {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Keys;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut keys = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    map.next_value::<IgnoredAny>()?;
                    keys.push(key);
                }
                Ok(Keys(keys))
            }
        }
        d.deserialize_map(V)
    }
}

#[derive(Deserialize)]
struct EpisodeLine {
    id: Option<String>,
    env: Option<EnvLine>,
    task: Option<TaskLine>,
    run: Option<RunLine>,
    ok: Option<bool>,
    is_timeout: Option<bool>,
    #[serde(default)]
    errors: Vec<ErrorLine>,
    #[serde(default)]
    traces: Vec<TraceLine>,
    num_input_tokens: Option<u64>,
    num_output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct EnvLine {
    id: Option<String>,
}

#[derive(Deserialize)]
struct RunLine {
    name: Option<String>,
}

#[derive(Deserialize)]
struct TaskLine {
    key: Option<String>,
    hash: Option<String>,
    data: Option<TaskData>,
}

#[derive(Deserialize)]
struct TaskData {
    idx: Option<u64>,
    prompt: Option<Prompt>,
}

/// `TaskData.prompt` is a string or a list of messages; anything else is ignored.
#[derive(Deserialize)]
#[serde(untagged)]
enum Prompt {
    Text(String),
    Messages(Vec<PromptMessage>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct PromptMessage {
    content: Option<Content>,
}

#[derive(Deserialize)]
struct ErrorLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct TraceLine {
    version: Option<u32>,
    id: Option<String>,
    verifiers: Option<VersionLine>,
    task: Option<TaskLine>,
    agent: Option<AgentLine>,
    rewards: Option<Ordered<RewardLine>>,
    #[serde(default)]
    calls: Vec<CallLine>,
    ok: Option<bool>,
    is_timeout: Option<bool>,
    stop_condition: Option<String>,
    #[serde(default)]
    errors: Vec<ErrorLine>,
    timing: Option<TimingLine>,
}

#[derive(Deserialize)]
struct VersionLine {
    version: Option<String>,
}

#[derive(Deserialize)]
struct AgentLine {
    name: Option<String>,
    config: Option<AgentConfigLine>,
    runtime: Option<RuntimeLine>,
}

#[derive(Deserialize)]
struct RuntimeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct AgentConfigLine {
    model: Option<String>,
    harness: Option<HarnessLine>,
}

#[derive(Deserialize)]
struct HarnessLine {
    id: Option<String>,
}

#[derive(Deserialize)]
struct RewardLine {
    score: Option<f64>,
    weight: Option<f64>,
}

#[derive(Deserialize)]
struct CallLine {
    usage: Option<UsageLine>,
    error: Option<ErrorLine>,
}

#[derive(Deserialize)]
struct UsageLine {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct TimingLine {
    start: Option<f64>,
    boot: Option<Phase>,
    setup: Option<Phase>,
    agent: Option<Phase>,
    finalize: Option<Phase>,
    scoring: Option<Phase>,
}

#[derive(Deserialize)]
struct Phase {
    start: Option<f64>,
    end: Option<f64>,
}

impl Phase {
    /// Phases that did not run are written as 0 to 0.
    fn seconds(&self) -> Option<f64> {
        let (start, end) = (self.start?, self.end?);
        (start > 0.0 && end > start).then_some(end - start)
    }
}

impl TimingLine {
    fn phases(&self) -> [(&'static str, &Option<Phase>); 5] {
        [
            ("boot", &self.boot),
            ("setup", &self.setup),
            ("agent", &self.agent),
            ("finalize", &self.finalize),
            ("scoring", &self.scoring),
        ]
    }

    fn duration(&self) -> Option<f64> {
        let start = self.start.filter(|s| *s > 0.0)?;
        let end = self
            .phases()
            .into_iter()
            .filter_map(|(_, p)| p.as_ref()?.end)
            .fold(0.0, f64::max);
        (end > start).then_some(end - start)
    }
}

/// What a line says about its run, beside the rollout.
#[derive(Debug, Default)]
struct LineFacts {
    run_name: Option<String>,
    env: Option<String>,
    model: Option<String>,
    harness: Option<String>,
    runtime: Option<String>,
    version: Option<String>,
    newer_schema: Option<u32>,
}

/// The rollout a line describes. A line is one episode; its headline rewards come from the
/// trace of the agent named `agent`, else the first trace (spec open question: multi-trace
/// episodes).
pub fn parse_episode_line(line: &str) -> Result<Rollout, LineError> {
    parse_line(line).map(|(rollout, _)| rollout)
}

fn parse_line(line: &str) -> Result<(Rollout, LineFacts), LineError> {
    if line.trim().is_empty() {
        return Err(LineError::Empty);
    }
    let episode: EpisodeLine =
        serde_json::from_str(line).map_err(|e| LineError::Invalid(e.to_string()))?;
    let main = episode
        .traces
        .iter()
        .find(|t| t.agent.as_ref().and_then(|a| a.name.as_deref()) == Some("agent"))
        .or_else(|| episode.traces.first());

    let task_line = episode
        .task
        .as_ref()
        .or_else(|| main.and_then(|t| t.task.as_ref()));
    let task = task_id(task_line);

    let timed_out = episode
        .is_timeout
        .or_else(|| main.and_then(|t| t.is_timeout))
        .unwrap_or(false);
    let ok = episode
        .ok
        .or_else(|| main.and_then(|t| t.ok))
        .unwrap_or_else(|| episode.errors.is_empty() && main.is_none_or(|t| t.errors.is_empty()));
    let outcome = if timed_out {
        Outcome::TimedOut
    } else if ok {
        Outcome::Completed
    } else {
        Outcome::Errored
    };

    let error = episode
        .errors
        .last()
        .or_else(|| main.and_then(|t| t.errors.last()))
        .or_else(|| {
            main.and_then(|t| t.calls.iter().rev().find_map(|c| c.error.as_ref()))
                .filter(|_| !ok)
        })
        .map(|e| ErrorInfo {
            kind: e.kind.clone().unwrap_or_default(),
            message: truncate(e.message.as_deref().unwrap_or_default(), MESSAGE_LIMIT),
        });

    let rewards: Vec<RewardPart> = main
        .and_then(|t| t.rewards.as_ref())
        .map(|r| {
            r.0.iter()
                .filter_map(|(name, line)| {
                    Some(RewardPart {
                        name: name.clone(),
                        score: line.score?,
                        weight: line.weight,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let reward = weighted_sum(&rewards);

    let (tokens, cost) = usage(&episode);

    let config = main.and_then(|t| t.agent.as_ref()?.config.as_ref());
    let keep = |s: Option<String>| s.filter(|s| !s.is_empty());
    let facts = LineFacts {
        run_name: episode.run.as_ref().and_then(|r| r.name.clone()),
        env: episode
            .env
            .as_ref()
            .and_then(|e| e.id.clone())
            .filter(|id| !id.is_empty()),
        model: keep(config.and_then(|c| c.model.clone())),
        harness: keep(config.and_then(|c| c.harness.as_ref()?.id.clone())),
        runtime: keep(main.and_then(|t| t.agent.as_ref()?.runtime.as_ref()?.kind.clone())),
        version: main.and_then(|t| t.verifiers.as_ref()?.version.clone()),
        newer_schema: episode
            .traces
            .iter()
            .filter_map(|t| t.version)
            .find(|v| *v > KNOWN_TRACE_VERSION),
    };
    let rollout = Rollout {
        id: episode
            .id
            .clone()
            .or_else(|| main.and_then(|t| t.id.clone()))
            .unwrap_or_default(),
        task,
        outcome,
        stop: main.and_then(|t| t.stop_condition.clone()),
        error,
        rewards,
        reward,
        tokens,
        cost,
        duration_secs: main.and_then(|t| t.timing.as_ref()?.duration()),
        started_at: main
            .and_then(|t| t.timing.as_ref()?.start)
            .filter(|s| *s > 0.0),
        traces: episode.traces.len() as u32,
        axes: Axes {
            env: facts.env.clone(),
            harness: facts.harness.clone(),
            model: facts.model.clone(),
            runtime: facts.runtime.clone(),
        },
    };
    Ok((rollout, facts))
}

/// `sum(score * weight)`. `Trace.reward` is a property, so the file never stores it. A reward
/// with no weight key weighs 1, as `record_reward` defaults it.
fn weighted_sum(rewards: &[RewardPart]) -> Option<f64> {
    (!rewards.is_empty()).then(|| {
        rewards
            .iter()
            .map(|r| r.score * r.weight.unwrap_or(1.0))
            .sum()
    })
}

fn task_id(task: Option<&TaskLine>) -> TaskId {
    let key = task
        .and_then(|t| t.key.clone().or_else(|| t.hash.clone()))
        .or_else(|| task.and_then(|t| Some(format!("idx-{}", t.data.as_ref()?.idx?))))
        .unwrap_or_else(|| "unknown".to_owned());
    let prompt = task
        .and_then(|t| t.data.as_ref()?.prompt.as_ref())
        .and_then(|p| match p {
            Prompt::Text(text) => Some(text.clone()),
            Prompt::Messages(messages) => messages.iter().rev().find_map(|m| {
                m.content
                    .as_ref()
                    .map(Content::text)
                    .filter(|t| !t.is_empty())
            }),
            Prompt::Other(_) => None,
        });
    let label = prompt
        .and_then(|p| {
            p.lines()
                .find(|l| !l.trim().is_empty())
                .map(|l| truncate(l.trim(), 80))
        })
        .unwrap_or_else(|| truncate(&key, 12));
    TaskId {
        key,
        label,
        checksum: None,
    }
}

/// Token totals over every call of every trace. `prompt_tokens` excludes cache reads, so input
/// is `prompt_tokens + cached_input_tokens`. When no call carries usage the episode's computed
/// totals stand in (they are the same sums, written by the framework).
fn usage(episode: &EpisodeLine) -> (Tokens, Option<f64>) {
    let mut tokens = Tokens::default();
    let mut cost: Option<f64> = None;
    let mut seen = false;
    for usage in episode
        .traces
        .iter()
        .flat_map(|t| &t.calls)
        .filter_map(|c| c.usage.as_ref())
    {
        seen = true;
        tokens.add(&Tokens {
            input: Some(
                usage
                    .prompt_tokens
                    .unwrap_or(0)
                    .saturating_add(usage.cached_input_tokens.unwrap_or(0)),
            ),
            cached: usage.cached_input_tokens,
            output: usage.completion_tokens,
        });
        cost = usage.cost.map(|c| c + cost.unwrap_or(0.0)).or(cost);
    }
    if !seen {
        tokens.input = episode.num_input_tokens;
        tokens.output = episode.num_output_tokens;
    }
    (tokens, cost)
}

/// What `configs/resolved/eval.json` states. The file may be missing; every field then falls
/// back to the episodes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedConfig {
    pub name: Option<String>,
    pub env: Option<String>,
    pub model: Option<String>,
    pub num_rollouts: Option<u32>,
}

pub fn parse_resolved_config(json: &str) -> ResolvedConfig {
    #[derive(Deserialize)]
    struct File {
        run: Option<RunLine>,
        model: Option<String>,
        num_rollouts: Option<u32>,
        env: Option<EnvFile>,
    }
    #[derive(Deserialize)]
    struct EnvFile {
        id: Option<String>,
        taskset: Option<TasksetFile>,
    }
    #[derive(Deserialize)]
    struct TasksetFile {
        id: Option<String>,
    }
    let Ok(file) = serde_json::from_str::<File>(json) else {
        return ResolvedConfig::default();
    };
    let non_empty = |s: Option<String>| s.filter(|s| !s.is_empty());
    let env = file
        .env
        .map(|e| (non_empty(e.id), e.taskset.and_then(|t| non_empty(t.id))));
    ResolvedConfig {
        name: file.run.and_then(|r| non_empty(r.name)),
        env: env.and_then(|(id, taskset)| id.or(taskset)),
        model: non_empty(file.model),
        num_rollouts: file.num_rollouts,
    }
}

/// What one pushed line did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pushed {
    Added,
    /// Same episode id seen again; the later line replaced the earlier rollout.
    Replaced,
    Skipped,
    /// A final line with no newline: a write in progress.
    Torn,
    Blank,
}

/// One pass over a `traces.jsonl`. Feed it the file's lines in order, with whether each ended
/// in a newline, then call [`TraceScan::finish`]. It holds one summary per episode and nothing
/// of the file, so memory follows the episode count, not the file size.
///
/// A scan describes one reading of the file. It never carries state across reads: `--resume`
/// rewrites the whole file (errored episodes come back under new ids), so a reader scans again
/// from the top and replaces what it showed.
#[derive(Default)]
pub struct TraceScan {
    rollouts: Vec<Rollout>,
    skipped: u32,
    torn_tail: bool,
    facts: RunFacts,
    first_started: Option<f64>,
}

impl TraceScan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: &[u8], terminated: bool) -> Pushed {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Pushed::Blank;
        }
        if !terminated {
            self.torn_tail = true;
            return Pushed::Torn;
        }
        let parsed = std::str::from_utf8(line)
            .map_err(|e| LineError::Invalid(e.to_string()))
            .and_then(parse_line);
        let (mut rollout, facts) = match parsed {
            Ok(parsed) => parsed,
            Err(_) => {
                self.skipped += 1;
                return Pushed::Skipped;
            }
        };
        self.absorb(facts);
        if rollout.id.is_empty() {
            rollout.id = format!("line-{}", self.rollouts.len() + 1 + self.skipped as usize);
        }
        if rollout.started_at.is_some() {
            self.first_started = match (self.first_started, rollout.started_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        match self.rollouts.iter().position(|r| r.id == rollout.id) {
            Some(at) => {
                self.rollouts[at] = rollout;
                Pushed::Replaced
            }
            None => {
                self.rollouts.push(rollout);
                Pushed::Added
            }
        }
    }

    fn absorb(&mut self, facts: LineFacts) {
        let f = &mut self.facts;
        f.name = f.name.take().or(facts.run_name);
        f.env = f.env.take().or(facts.env);
        f.framework_version = f.framework_version.take().or(facts.version);
        for (list, item) in [
            (&mut f.models, facts.model),
            (&mut f.harnesses, facts.harness),
        ] {
            if let Some(item) = item.filter(|i| !list.contains(i)) {
                list.push(item);
            }
        }
        if let Some(v) = facts.newer_schema {
            let note = format!("trace schema version {v} is newer than {KNOWN_TRACE_VERSION}");
            if !f.notes.contains(&note) {
                f.notes.push(note);
            }
        }
    }

    /// The facts and rollouts of this reading. `config` is `configs/resolved/eval.json`, when it
    /// exists; it wins over what the episodes say.
    pub fn finish(self, config: Option<&ResolvedConfig>) -> ParsedRun {
        let mut facts = self.facts;
        facts.skipped = self.skipped;
        facts.torn_tail = self.torn_tail;
        if let Some(c) = config {
            facts.name = c.name.clone().or(facts.name);
            facts.env = c.env.clone().or(facts.env);
            if let Some(model) = &c.model {
                facts.models.retain(|m| m != model);
                facts.models.insert(0, model.clone());
            }
            facts.rollouts_per_task = c.num_rollouts;
        }
        ParsedRun {
            facts,
            rollouts: self.rollouts,
        }
    }
}

pub struct ParsedRun {
    pub facts: RunFacts,
    pub rollouts: Vec<Rollout>,
}

/// Scan a whole file held as text; the host's streaming reader calls [`TraceScan::push`] itself.
pub fn scan_text(text: &str, config: Option<&ResolvedConfig>) -> ParsedRun {
    let mut scan = TraceScan::new();
    let mut rest = text.as_bytes();
    while !rest.is_empty() {
        let (line, terminated, next) = match rest.iter().position(|b| *b == b'\n') {
            Some(at) => (&rest[..at], true, &rest[at + 1..]),
            None => (rest, false, &rest[..0]),
        };
        scan.push(line, terminated);
        rest = next;
    }
    scan.finish(config)
}

// ---- document -------------------------------------------------------------------------------

#[derive(Deserialize)]
struct DocEpisode {
    #[serde(default)]
    errors: Vec<ErrorLine>,
    #[serde(default)]
    traces: Vec<DocTrace>,
}

#[derive(Deserialize)]
struct DocTrace {
    version: Option<u32>,
    agent: Option<AgentLine>,
    #[serde(default)]
    nodes: Vec<Node>,
    rewards: Option<Ordered<RewardLine>>,
    metrics: Option<Ordered<f64>>,
    #[serde(default)]
    errors: Vec<ErrorLine>,
    timing: Option<TimingLine>,
}

#[derive(Deserialize)]
struct Node {
    parent: Option<usize>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    role: Option<String>,
    content: Option<Content>,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ToolCallLine>>,
    tool_call_id: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
struct ToolCallLine {
    id: Option<String>,
    name: Option<String>,
    arguments: Option<Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Content {
    Text(String),
    Parts(Vec<Part>),
    Other(IgnoredAny),
}

#[derive(Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: Option<String>,
    text: Option<String>,
}

impl Content {
    fn text(&self) -> String {
        match self {
            Content::Text(text) => text.clone(),
            Content::Parts(parts) => parts
                .iter()
                .map(|p| match (p.kind.as_deref(), &p.text) {
                    (_, Some(text)) => text.clone(),
                    (Some("image_url"), None) => "[image]".to_owned(),
                    _ => String::new(),
                })
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            Content::Other(_) => String::new(),
        }
    }
}

/// One line as a document: the nodes along the last branch as turns, then rewards, metrics,
/// timing and errors. `trace` picks a trace of the episode; `None` picks as the summary does.
/// Token ids and logprobs are not declared, so they are skipped.
pub fn document(line: &str, trace: Option<usize>) -> Result<Document, LineError> {
    if line.trim().is_empty() {
        return Err(LineError::Empty);
    }
    let episode: DocEpisode =
        serde_json::from_str(line).map_err(|e| LineError::Invalid(e.to_string()))?;
    let at = trace
        .filter(|t| *t < episode.traces.len())
        .or_else(|| {
            episode
                .traces
                .iter()
                .position(|t| t.agent.as_ref().and_then(|a| a.name.as_deref()) == Some("agent"))
        })
        .unwrap_or(0);
    let mut doc = Document::default();
    for e in &episode.errors {
        doc.errors.push(error_info(e));
    }
    let Some(trace) = episode.traces.get(at) else {
        return Ok(doc);
    };

    let (path, others) = last_branch(&trace.nodes);
    doc.other_branches = others;
    doc.turns = path
        .into_iter()
        .filter_map(|i| turn(trace.nodes[i].message.as_ref()?))
        .collect();
    doc.rewards = trace
        .rewards
        .as_ref()
        .map(|r| {
            r.0.iter()
                .filter_map(|(n, l)| {
                    Some(RewardPart {
                        name: n.clone(),
                        score: l.score?,
                        weight: l.weight,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    doc.metrics = trace
        .metrics
        .as_ref()
        .map(|m| m.0.clone())
        .unwrap_or_default();
    if let Some(timing) = &trace.timing {
        doc.timing = timing
            .phases()
            .into_iter()
            .filter_map(|(name, p)| Some((name.to_owned(), p.as_ref()?.seconds()?)))
            .collect();
    }
    doc.errors.extend(trace.errors.iter().map(error_info));
    if trace.version.is_some_and(|v| v > KNOWN_TRACE_VERSION) {
        doc.notice = Some(format!(
            "newer schema: trace version {} is above {KNOWN_TRACE_VERSION}",
            trace.version.unwrap_or_default()
        ));
        doc.raw_keys = raw_keys(line, at);
    }
    Ok(doc)
}

fn error_info(e: &ErrorLine) -> ErrorInfo {
    ErrorInfo {
        kind: e.kind.clone().unwrap_or_default(),
        message: e.message.clone().unwrap_or_default(),
    }
}

fn raw_keys(line: &str, at: usize) -> Vec<String> {
    #[derive(Deserialize)]
    struct Episode {
        #[serde(default)]
        traces: Vec<Keys>,
    }
    serde_json::from_str::<Episode>(line)
        .ok()
        .and_then(|e| e.traces.into_iter().nth(at))
        .map(|k| k.0)
        .unwrap_or_default()
}

/// Indices from the root to the last node, and how many other leaves the graph has. Parents
/// are followed with a step bound, so a malformed cycle ends instead of looping.
fn last_branch(nodes: &[Node]) -> (Vec<usize>, u32) {
    let Some(last) = nodes.len().checked_sub(1) else {
        return (Vec::new(), 0);
    };
    let mut has_child = vec![false; nodes.len()];
    for parent in nodes
        .iter()
        .filter_map(|n| n.parent)
        .filter(|p| *p < nodes.len())
    {
        has_child[parent] = true;
    }
    let leaves = has_child.iter().filter(|c| !**c).count();
    let mut path = vec![last];
    let mut at = last;
    while let Some(parent) = nodes[at].parent.filter(|p| *p < nodes.len()) {
        if path.len() > nodes.len() {
            break;
        }
        path.push(parent);
        at = parent;
    }
    path.reverse();
    (path, leaves.saturating_sub(1) as u32)
}

fn turn(message: &Message) -> Option<Turn> {
    let role = match message.role.as_deref()? {
        "system" => Role::System,
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        _ => return None,
    };
    Some(Turn {
        role,
        text: message
            .content
            .as_ref()
            .map(Content::text)
            .unwrap_or_default(),
        reasoning: message.reasoning_content.clone().filter(|r| !r.is_empty()),
        tool_calls: message
            .tool_calls
            .iter()
            .flatten()
            .map(|c| ToolCall {
                id: c.id.clone().unwrap_or_default(),
                name: c.name.clone().unwrap_or_default(),
                arguments: match &c.arguments {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => String::new(),
                },
            })
            .collect(),
        tool_call_id: message.tool_call_id.clone(),
        name: message.name.clone(),
    })
}

#[cfg(test)]
mod tests;
