//! Against a real Harbor 0.24.0 job (`fixtures/README.md`); expected values are worked
//! out by hand from the fixture's files, not from this parser.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::Framework;
use crate::model::{Axes, summarize, task_table};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/harbor-0.24.0-oracle/",
            $path
        ))
    };
}

const TRIALS: [(&str, &str, &str); 6] = [
    (
        "broken-env__3eaHFQz",
        fixture!("broken-env__3eaHFQz/result.json"),
        fixture!("broken-env__3eaHFQz/config.json"),
    ),
    (
        "broken-env__57KRXab",
        fixture!("broken-env__57KRXab/result.json"),
        fixture!("broken-env__57KRXab/config.json"),
    ),
    (
        "write-file__6fBnNtH",
        fixture!("write-file__6fBnNtH/result.json"),
        fixture!("write-file__6fBnNtH/config.json"),
    ),
    (
        "write-file__Gm5rL9f",
        fixture!("write-file__Gm5rL9f/result.json"),
        fixture!("write-file__Gm5rL9f/config.json"),
    ),
    (
        "wrong-answer__DWmyxBy",
        fixture!("wrong-answer__DWmyxBy/result.json"),
        fixture!("wrong-answer__DWmyxBy/config.json"),
    ),
    (
        "wrong-answer__uFiDH5F",
        fixture!("wrong-answer__uFiDH5F/result.json"),
        fixture!("wrong-answer__uFiDH5F/config.json"),
    ),
];

fn job(skip_results: &[&str]) -> ParsedRun {
    let mut scan = JobScan::new("oracle-small");
    scan.job_result(fixture!("result.json"));
    scan.lock(fixture!("lock.json"));
    for (name, result, config) in TRIALS {
        let result = (!skip_results.contains(&name)).then_some(result);
        scan.trial(name, result, Some(config));
    }
    scan.finish()
}

#[test]
fn the_real_job_reads_as_six_trials_over_three_tasks() {
    let run = job(&[]);
    let s = summarize(Framework::Harbor, run.facts.clone(), &run.rollouts);
    assert_eq!((s.rollouts, s.tasks, s.errored, s.unfinished), (6, 3, 2, 0));
    // write-file 1, 1 and wrong-answer 0, 0; the two errored trials have no reward.
    assert_eq!(s.mean_reward, Some(0.5));
    assert_eq!(s.facts.framework_version.as_deref(), Some("0.24.0"));
    assert_eq!(s.facts.harnesses, ["oracle 1.0.0"]);
    assert_eq!(s.facts.ended, Some(true));
    // The job's own count agrees with the trial files.
    assert_eq!(s.facts.reported.as_ref().and_then(|r| r.errored), Some(2));
    // No model was called: unknown, not zero.
    assert_eq!(s.tokens.total(), None);
    // Trials come out in the order they started, not the directory's.
    assert_eq!(run.rollouts[0].id, "broken-env__3eaHFQz");
    assert_eq!(run.rollouts[1].id, "write-file__Gm5rL9f");
}

#[test]
fn rewards_keep_their_names_and_the_headline_is_the_one_called_reward() {
    let run = job(&[]);
    let write = run
        .rollouts
        .iter()
        .find(|r| r.id == "write-file__6fBnNtH")
        .unwrap();
    assert_eq!(write.reward, Some(1.0));
    let names: Vec<_> = write
        .rewards
        .iter()
        .map(|r| (r.name.as_str(), r.score))
        .collect();
    assert!(names.contains(&("style", 0.5)) && names.contains(&("reward", 1.0)));
    let rows = task_table(&run.rollouts);
    let mean = |key: &str| rows.iter().find(|r| r.task.key == key).and_then(|r| r.mean);
    assert_eq!(mean("sl/write-file").or(mean("write-file")), Some(1.0));
    assert_eq!(mean("sl/wrong-answer").or(mean("wrong-answer")), Some(0.0));
    assert_eq!(mean("sl/broken-env").or(mean("broken-env")), None);
}

#[test]
fn a_trial_without_result_json_is_unfinished_and_keeps_its_task() {
    let run = job(&["write-file__6fBnNtH"]);
    let unfinished = run
        .rollouts
        .iter()
        .find(|r| r.id == "write-file__6fBnNtH")
        .unwrap();
    assert_eq!(unfinished.outcome, Outcome::Unfinished);
    let finished = run
        .rollouts
        .iter()
        .find(|r| r.id == "write-file__Gm5rL9f")
        .unwrap();
    // The same task, whether the name came from config.json or result.json.
    assert_eq!(unfinished.task.key, finished.task.key);
    // Nothing but the agent is known of an unfinished trial, and the job's lock names that.
    assert_eq!(
        unfinished.axes,
        Axes {
            harness: Some("oracle".into()),
            ..Axes::default()
        }
    );
}

#[test]
fn trial_axes_come_from_source_agent_and_model() {
    let run = job(&[]);
    let write = run
        .rollouts
        .iter()
        .find(|r| r.id == "write-file__6fBnNtH")
        .unwrap();
    assert_eq!(write.axes.env.as_deref(), Some("htasks"));
    assert_eq!(write.axes.harness.as_deref(), Some("oracle 1.0.0"));
    assert_eq!(write.axes.model, None);
    assert_eq!(write.axes.runtime, None);
}

#[test]
fn harbor_input_tokens_already_include_the_cache() {
    let rollout = parse_trial_result(
        r#"{"trial_name":"t__abcdefg","task_name":"t","agent_result":{"n_input_tokens":100,"n_cache_tokens":40,"n_output_tokens":5,"cost_usd":0.25},"step_results":null}"#,
    )
    .unwrap();
    assert_eq!(rollout.tokens.input, Some(100));
    assert_eq!(rollout.tokens.total(), Some(105));
    // A multi-step trial sums its steps instead.
    let steps = parse_trial_result(
        r#"{"trial_name":"t__abcdefg","agent_result":null,"step_results":[{"agent_result":{"n_input_tokens":10,"n_output_tokens":1}},{"agent_result":{"n_input_tokens":5,"n_output_tokens":2}}]}"#,
    )
    .unwrap();
    assert_eq!(
        (steps.tokens.input, steps.tokens.output),
        (Some(15), Some(3))
    );
    assert_eq!(steps.task.key, "t");
}

#[test]
fn an_exception_with_a_reward_is_still_an_error() {
    let r = parse_trial_result(
        r#"{"trial_name":"x__1234567","task_name":"x","verifier_result":{"rewards":{"reward":1}},"exception_info":{"exception_type":"AgentTimeoutError","exception_message":"too slow"}}"#,
    )
    .unwrap();
    assert_eq!(r.outcome, Outcome::Errored);
    assert_eq!(r.reward, Some(1.0));
    assert!(!r.counts_in_mean());
}

#[test]
fn an_atif_trajectory_reads_as_turns_with_tool_results() {
    let doc = document(
        r#"{"schema_version":"ATIF-v1.8","agent":{"name":"a","version":"1"},"steps":[
            {"step_id":1,"source":"user","message":"Fix the test"},
            {"step_id":2,"source":"agent","message":[{"type":"text","text":"Running it"}],
             "tool_calls":[{"tool_call_id":"c1","function_name":"bash","arguments":{"cmd":"pytest"}}],
             "observation":{"results":[{"source_call_id":"c1","content":"1 passed"}]}}]}"#,
    )
    .unwrap();
    assert_eq!(doc.notice, None);
    let roles: Vec<_> = doc.turns.iter().map(|t| t.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool]);
    assert_eq!(doc.turns[1].tool_calls[0].arguments, r#"{"cmd":"pytest"}"#);
    assert_eq!(doc.turns[2].tool_call_id.as_deref(), Some("c1"));
    let other = document(r#"{"schema_version":"ATIF-v2.0","steps":[]}"#).unwrap();
    assert!(other.notice.is_some());
}

#[test]
fn a_result_without_agent_info_names_its_agent_from_the_echoed_config() {
    let r = parse_trial_result(
        r#"{"trial_name":"t__abcdefg","task_name":"t","agent_info":null,"config":{"agent":{"name":"nop"}}}"#,
    )
    .unwrap();
    assert_eq!(r.axes.harness.as_deref(), Some("nop"));
    // agent_info is the better source: it carries the version.
    let both = parse_trial_result(
        r#"{"trial_name":"t__abcdefg","task_name":"t","agent_info":{"name":"oracle","version":"1.0.0"},"config":{"agent":{"name":"other"}}}"#,
    )
    .unwrap();
    assert_eq!(both.axes.harness.as_deref(), Some("oracle 1.0.0"));
}

#[test]
fn an_unfinished_trial_reads_its_agent_from_its_config() {
    let r = unfinished_trial(
        "t__abcdefg",
        Some(r#"{"task":{"path":"tasks/t"},"agent":{"name":"nop"}}"#),
    );
    assert_eq!(r.axes.harness.as_deref(), Some("nop"));
    let none = unfinished_trial("t__abcdefg", Some(r#"{"task":{"path":"tasks/t"}}"#));
    assert_eq!(none.axes.harness, None);
}

#[test]
fn the_lock_names_the_agent_of_trials_that_do_not() {
    let lock = r#"{"harbor":{"version":"0.24.0"},"trials":[{"agent":{"name":"oracle"}},{"agent":{"name":"oracle"}}]}"#;
    let scan = |lock: &str| {
        let mut scan = JobScan::new("j");
        scan.lock(lock);
        scan.trial("t__abcdefg", None, Some(r#"{"task":{"path":"tasks/t"}}"#));
        scan.trial(
            "t__hijklmn",
            Some(r#"{"trial_name":"t__hijklmn","task_name":"t"}"#),
            None,
        );
        scan.finish()
    };
    let run = scan(lock);
    assert!(
        run.rollouts
            .iter()
            .all(|r| r.axes.harness.as_deref() == Some("oracle"))
    );
    assert_eq!(run.facts.framework_version.as_deref(), Some("0.24.0"));
    // Two agents in one job: a trial cannot be matched to either, so it stays unnamed.
    let mixed = scan(r#"{"trials":[{"agent":{"name":"oracle"}},{"agent":{"name":"nop"}}]}"#);
    assert!(mixed.rollouts.iter().all(|r| r.axes.harness.is_none()));
}

#[test]
fn the_real_oracle_job_names_its_agent_on_every_trial() {
    let run = job(&["write-file__6fBnNtH", "broken-env__3eaHFQz"]);
    assert!(run.rollouts.iter().all(|r| {
        r.axes
            .harness
            .as_deref()
            .is_some_and(|h| h.starts_with("oracle"))
    }));
}
