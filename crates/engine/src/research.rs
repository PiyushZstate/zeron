//! Eval runs in a folder, read for the Research view (Seamlessh's
//! docs/research-view.md). Ported from Seamlessh's host, where it read one
//! thread's worktree; here it reads whichever folder a chat runs in.
//!
//! Read-only: nothing here writes in a run's directory. verifiers runs are found as
//! `outputs/*/traces.jsonl` and Harbor jobs as `jobs/*/` holding `result.json` or
//! `config.json`. `seamlessh-runs` does the parsing; this module only finds and reads files.
//! A `traces.jsonl` is read line by line, so memory follows the episode count, not the
//! file's size, and every call reads afresh: `vf-eval --resume` rewrites the whole file.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Component, Path};
use std::time::{Duration, SystemTime};

use seamlessh_runs::{
    Document, Framework, Rollout, RunSummary, TaskRow, harbor, summarize, task_table, verifiers,
};
use serde::{Deserialize, Serialize};

/// One run as the engine read it: where it is, whether it is still being written, its
/// totals, its tasks and every rollout. Documents are not sent here; a run's file can
/// be gigabytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchRun {
    /// The run's directory, relative to the folder it was read from.
    pub path: String,
    /// Its files changed in the last 30 seconds. No format marks a run's end.
    pub live: bool,
    pub summary: RunSummary,
    pub tasks: Vec<TaskRow>,
    pub rollouts: Vec<Rollout>,
}

/// `ListResearchRuns`'s reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResearchRuns {
    pub runs: Vec<ResearchRun>,
}

/// A run whose files changed this recently is shown as live.
const LIVE_FOR: Duration = Duration::from_secs(30);

/// The document of one rollout of `run` (a [`ResearchRun::path`]) in `worktree`.
pub fn document(worktree: &Path, run: &str, rollout: &str) -> Result<Document, String> {
    let path = Path::new(run);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || !(run.starts_with("outputs/") || run.starts_with("jobs/"))
    {
        return Err("run path is not a run directory".to_owned());
    }
    if rollout.is_empty()
        || rollout.contains('/')
        || rollout.contains('\\')
        || rollout.contains("..")
    {
        return Err("rollout is not a rollout id".to_owned());
    }
    if run.starts_with("outputs/") {
        verifiers_document(worktree, run, rollout)
    } else {
        harbor_document(worktree, run, rollout)
    }
}

/// Every run under `worktree`, newest first.
pub fn runs(worktree: &Path, now: SystemTime) -> Vec<ResearchRun> {
    let mut found: Vec<(SystemTime, ResearchRun)> = Vec::new();
    for dir in children(&worktree.join("outputs")) {
        if let Some(run) = verifiers_run(worktree, &dir, now) {
            found.push(run);
        }
    }
    for dir in children(&worktree.join("jobs")) {
        if let Some(run) = harbor_job(worktree, &dir, now) {
            found.push(run);
        }
    }
    found.sort_by_key(|(changed, _)| std::cmp::Reverse(*changed));
    found.into_iter().map(|(_, run)| run).collect()
}

/// Subdirectories, in name order; none when `dir` is missing or unreadable.
fn children(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn is_live(changed: SystemTime, now: SystemTime) -> bool {
    now.duration_since(changed).is_ok_and(|age| age <= LIVE_FOR) || changed > now
}

fn relative(worktree: &Path, dir: &Path) -> String {
    dir.strip_prefix(worktree)
        .unwrap_or(dir)
        .to_string_lossy()
        .into_owned()
}

fn verifiers_run(
    worktree: &Path,
    dir: &Path,
    now: SystemTime,
) -> Option<(SystemTime, ResearchRun)> {
    let traces = dir.join(verifiers::TRACES_FILE);
    let file = File::open(&traces).ok()?;
    let changed = modified(&traces).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut scan = verifiers::TraceScan::new();
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                let terminated = line.last() == Some(&b'\n');
                let body = if terminated {
                    &line[..line.len() - 1]
                } else {
                    &line[..]
                };
                scan.push(body, terminated);
            }
            // A file swapped mid-read by `--resume`: show what was read; the next ask
            // reads the new file.
            Err(_) => break,
        }
    }
    let config = fs::read_to_string(dir.join("configs/resolved/eval.json"))
        .ok()
        .map(|text| verifiers::parse_resolved_config(&text));
    let parsed = scan.finish(config.as_ref());
    let tasks = task_table(&parsed.rollouts);
    let summary = summarize(Framework::Verifiers, parsed.facts, &parsed.rollouts);
    Some((
        changed,
        ResearchRun {
            path: relative(worktree, dir),
            live: is_live(changed, now),
            summary,
            tasks,
            rollouts: parsed.rollouts,
        },
    ))
}

fn verifiers_document(worktree: &Path, run: &str, rollout: &str) -> Result<Document, String> {
    let file =
        File::open(worktree.join(run).join(verifiers::TRACES_FILE)).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {
                let body = match line.as_slice() {
                    [rest @ .., b'\n'] => rest,
                    bytes => bytes,
                };
                let Ok(text) = std::str::from_utf8(body) else {
                    continue;
                };
                // A traces.jsonl can be gigabytes; only parse lines that could name this episode.
                if !text.contains(rollout) {
                    continue;
                }
                if episode_id(text).as_deref() != Some(rollout) {
                    continue;
                }
                return verifiers::document(text, None).map_err(|e| e.to_string());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!("no rollout {rollout} in this run"))
}

#[derive(Deserialize)]
struct EpisodeId {
    id: Option<String>,
}

fn episode_id(line: &str) -> Option<String> {
    serde_json::from_str::<EpisodeId>(line)
        .ok()?
        .id
        .filter(|id| !id.is_empty())
}

fn harbor_document(worktree: &Path, run: &str, rollout: &str) -> Result<Document, String> {
    let trial = worktree.join(run).join(rollout);
    if !trial.is_dir() {
        return Err(format!("no rollout {rollout} in this run"));
    }
    let trajectory = trial.join(harbor::TRAJECTORY_FILE);
    let mut doc = if trajectory.is_file() {
        harbor::document(&fs::read_to_string(&trajectory).map_err(|e| e.to_string())?)?
    } else {
        harbor::log_document(&harbor_logs(&trial))
    };
    if let Ok(json) = fs::read_to_string(trial.join(harbor::RESULT_FILE))
        && let Ok(summary) = harbor::parse_trial_result(&json)
    {
        doc.attach(&summary);
    }
    Ok(doc)
}

fn harbor_logs(trial: &Path) -> Vec<(String, String)> {
    let mut logs: Vec<_> = fs::read_dir(trial.join("agent"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((name, e.path()))
        })
        .collect();
    logs.sort_by(|a, b| a.0.cmp(&b.0));
    logs.into_iter()
        .filter(|(name, _)| name != "trajectory.json")
        .filter_map(|(name, path)| Some((name, fs::read_to_string(path).ok()?)))
        .collect()
}

fn harbor_job(worktree: &Path, dir: &Path, now: SystemTime) -> Option<(SystemTime, ResearchRun)> {
    let result_path = dir.join(harbor::RESULT_FILE);
    let job_result = fs::read_to_string(&result_path).ok();
    if job_result.is_none() && !dir.join(harbor::CONFIG_FILE).is_file() {
        return None;
    }
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let mut scan = harbor::JobScan::new(&name);
    if let Some(json) = &job_result {
        scan.job_result(json);
    }
    if let Ok(lock) = fs::read_to_string(dir.join(harbor::LOCK_FILE)) {
        scan.lock(&lock);
    }
    let mut changed = modified(&result_path).unwrap_or(SystemTime::UNIX_EPOCH);
    for trial in children(dir) {
        let result = trial.join(harbor::RESULT_FILE);
        let config = trial.join(harbor::CONFIG_FILE);
        if !result.is_file() && !config.is_file() {
            continue;
        }
        let trial_name = trial.file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(trial_name) = trial_name else {
            continue;
        };
        changed = changed.max(modified(&trial).unwrap_or(SystemTime::UNIX_EPOCH));
        scan.trial(
            &trial_name,
            fs::read_to_string(&result).ok().as_deref(),
            fs::read_to_string(&config).ok().as_deref(),
        );
    }
    let parsed = scan.finish();
    let tasks = task_table(&parsed.rollouts);
    let summary = summarize(Framework::Harbor, parsed.facts, &parsed.rollouts);
    let live = summary.facts.ended != Some(true) && is_live(changed, now);
    Some((
        changed,
        ResearchRun {
            path: relative(worktree, dir),
            live,
            summary,
            tasks,
            rollouts: parsed.rollouts,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    const EPISODE_OK: &str = r#"{"id":"e1","task":{"key":"t1","data":{"prompt":"Add 2 and 2"}},"ok":true,"is_timeout":false,"errors":[],"traces":[{"version":1,"agent":{"name":"agent","config":{"model":"m"}},"rewards":{"correct":{"score":1.0,"weight":0.5},"format":{"score":0.5,"weight":1.0}},"ok":true}]}"#;
    const EPISODE_FAILED: &str = r#"{"id":"e2","task":{"key":"t1"},"ok":false,"errors":[{"type":"Boom","message":"sandbox died"}],"traces":[{"version":1,"rewards":{"correct":{"score":1.0,"weight":1.0}},"ok":false}]}"#;

    #[test]
    fn finds_both_formats_and_reads_a_torn_tail_as_in_progress() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // An errored rollout with reward 1.0 stays out of the mean; the torn last line is
        // a write in progress, not a third rollout.
        write(
            &root.join("outputs/run-a/traces.jsonl"),
            &format!("{EPISODE_OK}\n{EPISODE_FAILED}\n{{\"id\":\"e3\",\"ta"),
        );
        write(&root.join("outputs/not-a-run/notes.txt"), "x");
        write(
            &root.join("jobs/2026-10-22__10-00-00/result.json"),
            r#"{"id":"0","started_at":"2026-10-22T10:00:00","finished_at":"2026-10-22T10:01:00","n_total_trials":2,"stats":{"n_errored_trials":1}}"#,
        );
        write(
            &root.join("jobs/2026-10-22__10-00-00/write-file__abc1234/result.json"),
            r#"{"task_name":"write-file","trial_name":"write-file__abc1234","agent_info":{"name":"oracle","version":"1"},"agent_result":{"n_input_tokens":100,"n_cache_tokens":40,"n_output_tokens":5},"verifier_result":{"rewards":{"reward":1,"style":0.5}},"started_at":"2026-10-22T10:00:01","finished_at":"2026-10-22T10:00:11"}"#,
        );
        write(
            &root.join("jobs/2026-10-22__10-00-00/write-file__def5678/config.json"),
            r#"{"task":{"path":"tasks/write-file"}}"#,
        );

        let runs = runs(root, SystemTime::now());
        assert_eq!(runs.len(), 2);
        let v = runs.iter().find(|r| r.path == "outputs/run-a").unwrap();
        assert_eq!(v.summary.rollouts, 2);
        assert_eq!(v.summary.errored, 1);
        assert!(v.summary.facts.torn_tail);
        // 1.0 × 0.5 + 0.5 × 1.0, from the first rollout alone.
        assert_eq!(v.summary.mean_reward, Some(1.0));
        assert!(v.live);
        assert_eq!(v.rollouts.len(), 2);
        assert_eq!(v.rollouts[0].id, "e1");

        let h = runs.iter().find(|r| r.path.starts_with("jobs/")).unwrap();
        assert_eq!((h.summary.rollouts, h.summary.unfinished), (2, 1));
        assert_eq!(h.summary.mean_reward, Some(1.0));
        assert_eq!(h.rollouts.len(), 2);
        // Harbor's input count already includes the cache.
        assert_eq!(h.summary.tokens.input, Some(100));
        assert_eq!(h.tasks.len(), 1);
        // The job says it finished, so it is not live however fresh its files are.
        assert!(!h.live);
    }

    #[test]
    fn a_worktree_without_runs_has_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(runs(dir.path(), SystemTime::now()).is_empty());
    }

    #[test]
    fn loads_a_verifiers_episode_as_a_document() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("outputs/run-a/traces.jsonl"),
            &format!("{EPISODE_OK}\n{EPISODE_FAILED}\n"),
        );
        let doc = document(root, "outputs/run-a", "e1").unwrap();
        assert_eq!(
            doc.rewards
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>(),
            ["correct", "format"]
        );
    }

    #[test]
    fn a_missing_verifiers_rollout_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("outputs/run-a/traces.jsonl"),
            &format!("{EPISODE_OK}\n"),
        );
        assert_eq!(
            document(root, "outputs/run-a", "e3").unwrap_err(),
            "no rollout e3 in this run"
        );
    }

    #[test]
    fn loads_a_harbor_trial_with_a_trajectory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let trial = root.join("jobs/2026-10-22__10-00-00/write-file__abc1234");
        write(
            &trial.join("agent/trajectory.json"),
            r#"{"schema_version":"ATIF-v1.8","steps":[{"step_id":1,"source":"user","message":"Fix the test"}]}"#,
        );
        write(
            &trial.join("result.json"),
            r#"{"task_name":"write-file","trial_name":"write-file__abc1234","verifier_result":{"rewards":{"reward":1,"style":0.5}}}"#,
        );
        let doc = document(root, "jobs/2026-10-22__10-00-00", "write-file__abc1234").unwrap();
        assert_eq!(doc.turns[0].text, "Fix the test");
        assert_eq!(
            doc.rewards
                .iter()
                .map(|r| (r.name.as_str(), r.score))
                .collect::<Vec<_>>(),
            [("reward", 1.0), ("style", 0.5)]
        );
    }

    #[test]
    fn refuses_a_run_or_rollout_that_leaves_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for run in ["../x", "/etc", "outputs/../../x"] {
            assert!(document(root, run, "e1").is_err(), "{run}");
        }
        assert!(document(root, "outputs/run-a", "e1/x").is_err());
        assert!(
            document(root, "jobs/j", "").is_err(),
            "an empty id is the job itself"
        );
    }
}
