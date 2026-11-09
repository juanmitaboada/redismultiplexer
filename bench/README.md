# Bench harness

Black-box, matrix-driven throughput bench for redismultiplexer. The program
under test is given as a command line; the harness renders its configuration,
loads the source queues, starts it, measures until the targets are drained,
stops it with SIGINT and records one CSV row per run. Comparing two builds
(for example a release against a branch) is a matter of running both and
calling `compare`.

Requirements: Python 3.11 or newer (standard library only, `tomllib`), a Redis
6.2 or newer server reachable from the host. No redis-cli needed.

## Commands

```
python3 bench/rmbench.py list [--tag quick|full|smoke] [--scenario GLOB]
python3 bench/rmbench.py run --cmd 'target/release/redismultiplexer {config}' \
    --label rust-1.0.7 --redis redis://127.0.0.1:6390 --results DIR \
    [--tag T ...] [--scenario GLOB ...] [--repeat N] [--scale F] \
    [--renderer CMD] [--config-ext .yaml] \
    [--timeout 120] [--settle 2] [--max-bytes BYTES] [--keep-keys]
python3 bench/rmbench.py clean --redis URL
python3 bench/rmbench.py compare A.csv B.csv [--label-a X] [--label-b Y] [--scenario GLOB ...]
python3 -m unittest discover -s bench -p 'test_*.py'
```

- `--cmd` must contain `{config}`; it is split with shell rules and run
  without a shell, in its own session (SIGINT goes to its process group). The
  program must stay in the foreground.
- `--redis` defaults to `$RM_BENCH_REDIS_URL`, then `redis://127.0.0.1:6379`.
- `run` exits 1 if any run ended in `error` (renderer failure, program not
  starting or exiting early, non-zero exit after SIGINT), 2 on harness errors.
  Timeouts and missing packages are results, not errors.
- `clean` deletes every `rmbench:*` key with SCAN and UNLINK, never KEYS.

## Scenarios

`scenarios.toml` holds the matrix. Every `[[scenario]]` inherits `[defaults]`;
a list value is a matrix axis and expands to the cartesian product. Runs are
named `<name>.<axis><value>...` with the axis abbreviations `p` payload,
`c` children, `m` mode, `n` clients, `o` ordering, `f` filter, `l` limits,
`k` count, `t` transport. A scenario with `instances` starts one process per
instance at the same time (mixed scenario) and reports each instance plus an
aggregate row.

| Field | Values |
|---|---|
| transport | `list` (pub/sub and streams later) |
| count | packages per source, multiple of 10 |
| payload | bytes per package, at least 64 |
| children | worker threads |
| mode | `replicant`, `spreader` |
| clients | number of targets |
| ordering | `true`: regex `"ts": *(?P<ts>\d+),`, buffer 1 s, limit 64 |
| filter | `off`; `root`: `"k": [0-4],` on the first 64 bytes (half pass); `client`: same regex on target 0 with replacement `"K": $1,` (replicant only) |
| limits | `off`; `countdown`: checklimit 1000, timelimit 3600; `time`: timelimit 1, checklimit 1000000; soft and hard limits are never reached |

Tags: `smoke` (CI), `quick` (routine, about 1.5 min with 1.0.7 on 2 vCPU),
`full` (whole matrix, about 10-12 min). `--scale F` multiplies every count
(rounded to a multiple of 30) for when runs get too short to measure.

Package format (ASCII, exactly `payload` bytes, deterministic):
`{"ts": <seq>, "k": <seq mod 10>, "seq": <seq>, "p": "aaa..."}`.

## Renderer contract

For every instance the harness writes `iN.spec.json` and runs
`<renderer> <spec.json> <out_config>`; the program is then started with
`{config}` replaced by `<out_config>`. Exit 0 = written, 3 = not supported
(the run is recorded as `n/a`), anything else = error. `render_rust.py` is the
renderer for redismultiplexer and the default. Spec:

```json
{
  "scenario": "q_base.c2", "run_id": "...", "instance": 0,
  "transport": "list",
  "redis": {"host": "127.0.0.1", "port": 6390, "password": "", "ssl": false},
  "source": {"key": "rmbench:q_base.c2:i0:src"},
  "children": 2, "mode": "replicant",
  "pid_file": "/.../i0.pid", "status_file": "/.../i0.status",
  "filter": null,
  "ordering": null,
  "clients": [
    {"name": "t0", "key": "rmbench:q_base.c2:i0:t0", "filter": null, "limits": null}
  ]
}
```

`filter` is null or `{regex, until, limit, replace}`; `ordering` is null or
`{regex, buffer_time, limit}`; `limits` is null or `{timelimit, checklimit,
softlimit, hardlimit}`.

## Metrics (results.csv)

One row per run and repetition (plus an aggregate row for mixed runs).

| Column | Meaning |
|---|---|
| msg_s | source packages consumed / drain_s |
| mib_s | payload bytes delivered / drain_s |
| drain_s | from process start to the last change in any watched list |
| stop_s | from SIGINT to process exit |
| wall_s | drain_s + stop_s |
| expected_total, delivered_total, missing | expected packages in the targets, packages found there after exit, difference |
| per_target | packages per target, `;` separated |
| status_file_drop | `total_drop` from the status file if the program wrote one; redismultiplexer writes it every 10 s with interval totals, so it is informative only |
| cpu_user_s, cpu_sys_s | from `wait4` of the started process (includes children it waited for) |
| max_rss_kib | peak VmHWM of the started process, sampled every 100 ms from /proc |
| harness_cpu_s | CPU used by the harness itself while the program was draining |

A run ends when the targets hold the expected count and the source is empty,
or when the source is empty and nothing has changed for `--settle` seconds
(plus the ordering buffer time), or at `--timeout` (`status = timeout`).
Packages delivered after that point still count in `delivered_total`.

Per-run files go to `DIR/logs/<run_id>/`: spec, rendered config, stdout,
stderr. Each invocation also writes `DIR/report-<label>-<time>.md` with the
median of the repetitions per scenario.
