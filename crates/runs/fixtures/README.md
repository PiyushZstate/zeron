# Fixtures

## harbor-0.24.0-oracle

A real Harbor 0.24.0 job, run on 2026-10-22 on macOS with local Docker:

```sh
harbor run -p htasks -a oracle -k 2 -o hjobs --job-name oracle-small -n 3
```

`htasks` held three tasks made with `harbor tasks init`: `write-file` (the oracle
passes; the verifier writes `{"reward": 1, "style": 0.5}` to `reward.json`),
`wrong-answer` (the oracle writes the wrong answer; `reward.txt` is 0) and
`broken-env` (its Dockerfile runs `exit 3`, so both trials raise `RuntimeError`).

Edits: each `exception_traceback` is replaced with `(trimmed)`, local paths are
rewritten to `/work` and `/home`, and the job `result.json`'s `trial_results`
(full copies of the trial files) is emptied for size. Only `result.json`,
`config.json` and `lock.json` are kept. No model was called, so token counts are
absent.

A verifiers fixture from a real run is still open: `vf-eval` needs a model
endpoint, and the bundled Harbor taskset refuses tasks that build a Dockerfile.
