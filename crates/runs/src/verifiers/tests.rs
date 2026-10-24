//! Lines shaped as `write_episode` writes them (verifiers 0.4.0, `exclude_none=True`); not
//! from a real run, which is still open (`fixtures/README.md`). Expected values are worked
//! out by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::model::{Framework, NOT_RECORDED, axis_text, summarize, task_table};

/// An episode on task `key` with the given rewards as `(name, score, weight)`.
fn episode(id: &str, key: &str, ok: bool, rewards: &[(&str, f64, f64)]) -> String {
    let rewards: Vec<String> = rewards
        .iter()
        .map(|(n, s, w)| format!(r#""{n}":{{"score":{s},"weight":{w}}}"#))
        .collect();
    format!(
        r#"{{"id":"{id}","env":{{"id":"gsm8k"}},"task":{{"type":"t","key":"{key}","data":{{"prompt":[{{"role":"user","content":"What is 6 x 7?\nShow work."}}],"idx":0}}}},"ok":{ok},"is_timeout":false,"errors":[],"traces":[{{"version":1,"id":"tr-{id}","verifiers":{{"version":"0.4.0"}},"agent":{{"name":"agent","config":{{"model":"qwen3","harness":{{"id":"bash"}}}},"trainable":false}},"nodes":[{{"message":{{"role":"user","content":"What is 6 x 7?"}},"sampled":false,"token_ids":[1,2,3],"logprobs":[-0.1,-0.2]}},{{"parent":0,"message":{{"role":"assistant","content":"42"}},"sampled":true}}],"calls":[{{"usage":{{"prompt_tokens":100,"completion_tokens":7,"cached_input_tokens":30}}}}],"rewards":{{{}}},"metrics":{{"turns":1.0}},"ok":{ok},"is_timeout":false,"stop_condition":"agent_completed"}}],"num_input_tokens":130,"num_output_tokens":7}}"#,
        rewards.join(",")
    )
}

#[test]
fn the_reward_is_the_weighted_sum_the_file_never_stores() {
    let r = parse_episode_line(&episode(
        "e1",
        "k",
        true,
        &[("correct", 1.0, 0.75), ("format", 0.5, 0.5)],
    ))
    .unwrap();
    // 1.0 × 0.75 + 0.5 × 0.5
    assert_eq!(r.reward, Some(1.0));
    assert_eq!(
        r.rewards
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["correct", "format"]
    );
    assert_eq!(r.task.label, "What is 6 x 7?");
}

#[test]
fn ok_false_with_a_high_reward_is_an_error_and_stays_out_of_the_mean() {
    let text = [
        episode("e1", "k", true, &[("r", 0.5, 1.0)]),
        episode("e2", "k", false, &[("r", 1.0, 1.0)]),
    ]
    .join("\n")
        + "\n";
    let run = scan_text(&text, None);
    let failed = run.rollouts.iter().find(|r| r.id == "e2").unwrap();
    assert_eq!(
        (failed.outcome, failed.reward),
        (Outcome::Errored, Some(1.0))
    );
    let s = summarize(Framework::Verifiers, run.facts, &run.rollouts);
    assert_eq!((s.mean_reward, s.errored), (Some(0.5), 1));
}

#[test]
fn absent_keys_are_unknown_not_false_or_zero() {
    // `exclude_none` drops `ok`, `rewards` and the usage: nothing says it failed, and
    // nothing gives it a reward of 0.
    let r =
        parse_episode_line(r#"{"id":"e","task":{"key":"k"},"traces":[{"version":1}]}"#).unwrap();
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(r.reward, None);
    assert_eq!(r.tokens.total(), None);
}

#[test]
fn verifiers_input_tokens_add_the_cache_back() {
    let r = parse_episode_line(&episode("e1", "k", true, &[])).unwrap();
    // `prompt_tokens` 100 excludes 30 cache reads.
    assert_eq!(
        (r.tokens.input, r.tokens.cached, r.tokens.output),
        (Some(130), Some(30), Some(7))
    );
}

#[test]
fn a_torn_last_line_is_counted_not_read_and_bad_lines_are_skipped() {
    let full = episode("e1", "k", true, &[("r", 1.0, 1.0)]);
    let torn = &episode("e2", "k", true, &[])[..40];
    let text = format!("{full}\nnot json\n\n{torn}");
    let run = scan_text(&text, None);
    assert_eq!(run.rollouts.len(), 1);
    assert!(run.facts.torn_tail);
    assert_eq!(run.facts.skipped, 1);
    // Once the write finishes, the next reading has both.
    let whole = format!("{full}\n{}\n", episode("e2", "k", true, &[]));
    assert_eq!(scan_text(&whole, None).rollouts.len(), 2);
}

#[test]
fn a_file_rewritten_by_resume_reads_as_what_it_holds_now() {
    // Before: e1 done, e2 errored. `--resume` drops e2 and reruns its rollout as e3.
    let before = format!(
        "{}\n{}\n",
        episode("e1", "k", true, &[("r", 1.0, 1.0)]),
        episode("e2", "k", false, &[])
    );
    let after = format!(
        "{}\n{}\n",
        episode("e1", "k", true, &[("r", 1.0, 1.0)]),
        episode("e3", "k", true, &[("r", 0.0, 1.0)])
    );
    assert_eq!(scan_text(&before, None).rollouts.len(), 2);
    let run = scan_text(&after, None);
    let ids: Vec<_> = run.rollouts.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["e1", "e3"]);
    let rows = task_table(&run.rollouts);
    assert_eq!((rows[0].mean, rows[0].errored), (Some(0.5), 0));
}

#[test]
fn the_resolved_config_names_the_run_and_a_newer_schema_is_noted() {
    let config = parse_resolved_config(
        r#"{"run":{"name":"gsm8k--qwen3--bash--0a1b2c3d"},"model":"qwen3","num_rollouts":3,"env":{"id":"","taskset":{"id":"gsm8k"}}}"#,
    );
    let newer = episode("e1", "k", true, &[]).replace(r#""version":1,"#, r#""version":2,"#);
    let run = scan_text(&format!("{newer}\n"), Some(&config));
    assert_eq!(
        run.facts.name.as_deref(),
        Some("gsm8k--qwen3--bash--0a1b2c3d")
    );
    assert_eq!(run.facts.env.as_deref(), Some("gsm8k"));
    assert_eq!(run.facts.rollouts_per_task, Some(3));
    assert_eq!(run.facts.harnesses, ["bash"]);
    assert_eq!(run.facts.notes.len(), 1);
    // A file with no config still names what the episodes say.
    let bare = scan_text(&format!("{}\n", episode("e1", "k", true, &[])), None);
    assert_eq!(bare.facts.env.as_deref(), Some("gsm8k"));
    assert_eq!(bare.facts.framework_version.as_deref(), Some("0.4.0"));
}

#[test]
fn agent_runtime_type_is_the_runtime_axis() {
    let line = episode("e1", "k", true, &[]).replace(
        r#""trainable":false"#,
        r#""runtime":{"type":"modal","borrowed":false,"unknown":true},"trainable":false"#,
    );
    let r = parse_episode_line(&line).unwrap();
    assert_eq!(r.axes.runtime.as_deref(), Some("modal"));
    assert_eq!(r.axes.env.as_deref(), Some("gsm8k"));
    assert_eq!(r.axes.harness.as_deref(), Some("bash"));
    assert_eq!(r.axes.model.as_deref(), Some("qwen3"));
}

#[test]
fn missing_runtime_is_not_recorded() {
    let r = parse_episode_line(&episode("e1", "k", true, &[])).unwrap();
    assert_eq!(r.axes.runtime, None);
    assert_eq!(axis_text(r.axes.runtime.as_deref()), NOT_RECORDED);

    let empty = episode("e2", "k", true, &[]).replace(
        r#""trainable":false"#,
        r#""runtime":{"type":""},"trainable":false"#,
    );
    let r = parse_episode_line(&empty).unwrap();
    assert_eq!(r.axes.runtime, None);
}

#[test]
fn a_task_with_no_key_falls_back_to_hash() {
    let r = parse_episode_line(r#"{"id":"e","task":{"hash":"abc123"},"traces":[{"version":1}]}"#)
        .unwrap();
    assert_eq!(r.task.key, "abc123");
}

#[test]
fn the_document_follows_the_message_graph_and_skips_token_streams() {
    let doc = document(&episode("e1", "k", true, &[("r", 1.0, 1.0)]), None).unwrap();
    let turns: Vec<_> = doc
        .turns
        .iter()
        .map(|t| (t.role, t.text.as_str()))
        .collect();
    assert_eq!(
        turns,
        [(Role::User, "What is 6 x 7?"), (Role::Assistant, "42")]
    );
    assert_eq!(doc.metrics, [("turns".to_owned(), 1.0)]);
    assert_eq!(doc.other_branches, 0);
}
