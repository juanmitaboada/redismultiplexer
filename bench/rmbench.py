#!/usr/bin/env python3
"""Black-box, matrix-driven throughput bench for redismultiplexer.

The program under test is a command line containing "{config}"; a renderer
turns each run's spec JSON into its configuration. Python 3.11+
standard library only. See bench/README.md.

Subcommands: run, list, clean, compare.
"""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import fnmatch
import itertools
import json
import math
import os
import resource
import shlex
import signal
import statistics
import subprocess
import sys
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from resp import Redis, RedisError, parse_url  # noqa: E402

HERE = Path(__file__).resolve().parent
DEFAULT_SCENARIOS = HERE / "scenarios.toml"
DEFAULT_RENDERER = f"{shlex.quote(sys.executable)} {shlex.quote(str(HERE / 'render_rust.py'))}"
DEFAULT_REDIS = os.environ.get("RM_BENCH_REDIS_URL", "redis://127.0.0.1:6379")
KEY_PREFIX = "rmbench"
POLL_S = 0.010
EXIT_WAIT_S = 30.0
LOAD_CHUNK_BYTES = 8 << 20
RENDER_UNSUPPORTED = 3
EXIT_MISSING = 3  # `run`: no run failed, but some repetition did not deliver exactly what was expected

AXES = ("transport", "count", "payload", "children", "mode", "clients", "ordering", "filter", "limits")
ABBREV = {
    "transport": "t",
    "count": "k",
    "payload": "p",
    "children": "c",
    "mode": "m",
    "clients": "n",
    "ordering": "o",
    "filter": "f",
    "limits": "l",
}

ORDERING = {"regex": '"ts": *(?P<ts>\\d+),', "buffer_time": 1, "limit": 64}
ROOT_FILTER = {"regex": '"k": [0-4],', "until": None, "limit": 64, "replace": None}
CLIENT_FILTER = {"regex": '"k": ([0-4]),', "until": None, "limit": 64, "replace": '"K": $1,'}

CSV_FIELDS = [
    "ts_utc", "label", "run_id", "scenario", "tags", "transport", "count",
    "payload", "children", "mode", "clients", "ordering", "filter", "limits", "repeat_idx",
    "status", "reason", "msg_s", "mib_s", "drain_s", "stop_s", "wall_s", "expected_total",
    "delivered_total", "missing", "per_target", "status_file_drop", "cpu_user_s", "cpu_sys_s",
    "max_rss_kib", "harness_cpu_s", "redis_version", "host_cpus", "duration_s",
]


class BenchError(Exception):
    pass


# ---------------------------------------------------------------- matrix


@dataclass
class Run:
    name: str
    tags: list[str]
    instances: list[dict[str, Any]]
    mixed: bool = False
    duration: float = 0.0  # > 0: time-boxed run of this many seconds; 0: drain the source


def _fmt(v: Any) -> str:
    if isinstance(v, bool):
        return "on" if v else "off"
    return str(v)


def expand(doc: dict[str, Any], scale: float = 1.0) -> list[Run]:
    """Expand scenario entries into runs (cartesian product of list axes)."""
    defaults = dict(doc.get("defaults", {}))
    runs: list[Run] = []
    seen: set[str] = set()
    for sc in doc.get("scenario", []):
        if "name" not in sc:
            raise BenchError("scenario without name")
        base = {**defaults, **{k: v for k, v in sc.items() if k not in ("name", "tags", "instances")}}
        duration = float(base.get("duration", 0))
        if duration < 0:
            raise BenchError(f"{sc['name']}: duration must be 0 or positive")
        axes = [k for k in AXES if isinstance(base.get(k), list)]
        for combo in itertools.product(*(base[k] for k in axes)):
            params = dict(base)
            params.update(zip(axes, combo))
            name = sc["name"] + "".join(f".{ABBREV[k]}{_fmt(v)}" for k, v in zip(axes, combo))
            if name in seen:
                raise BenchError(f"duplicate run name {name}")
            seen.add(name)
            if "instances" in sc:
                insts = [_scaled({**params, **inst}, scale) for inst in sc["instances"]]
                runs.append(Run(name, list(sc.get("tags", [])), insts, mixed=True, duration=duration))
            else:
                runs.append(Run(name, list(sc.get("tags", [])), [_scaled(params, scale)], duration=duration))
    return runs


def _scaled(params: dict[str, Any], scale: float) -> dict[str, Any]:
    p = {k: params[k] for k in AXES if k in params}
    # Multiple of 30: exact 50% filter split (k = seq mod 10) and 3-way spreading.
    # A scale of 1 (the default) keeps the scenario's count as written (MQF-10: no float equality).
    if math.isclose(scale, 1.0):
        p["count"] = int(p["count"])
    else:
        p["count"] = max(30, int(round(int(p["count"]) * scale / 30.0)) * 30)
    return p


def matching(n: int) -> int:
    """Packages with k in 0-4 among the first n (k = seq mod 10): the filter regexes match these."""
    return (n // 10) * 5 + min(n % 10, 5)


def expected_per_target(inst: dict[str, Any], n: int | None = None) -> list[int] | None:
    """Expected count per target (replicant) for the first n packages (default: all), or None
    when only the total is known."""
    n = inst["count"] if n is None else n
    passed = matching(n) if inst["filter"] == "root" else n
    if inst["mode"] == "spreader":
        return None
    out = []
    for i in range(inst["clients"]):
        out.append(matching(passed) if (inst["filter"] == "client" and i == 0) else passed)
    return out


def expected_total(inst: dict[str, Any], n: int | None = None) -> int:
    n = inst["count"] if n is None else n
    per = expected_per_target(inst, n)
    if per is not None:
        return sum(per)
    return matching(n) if inst["filter"] == "root" else n


def validate(run: Run, max_bytes: int) -> None:
    for inst in run.instances:
        if inst["transport"] != "list":
            raise BenchError(f"{run.name}: transport {inst['transport']!r} not supported yet (only 'list')")
        if inst["mode"] not in ("replicant", "spreader"):
            raise BenchError(f"{run.name}: unknown mode {inst['mode']!r}")
        if inst["filter"] not in ("off", "root", "client"):
            raise BenchError(f"{run.name}: unknown filter {inst['filter']!r}")
        if inst["limits"] not in ("off", "countdown", "time"):
            raise BenchError(f"{run.name}: unknown limits {inst['limits']!r}")
        if inst["filter"] == "client" and inst["mode"] != "replicant":
            raise BenchError(f"{run.name}: client filter scenarios must be replicant")
        if inst["count"] % 10:
            raise BenchError(f"{run.name}: count must be a multiple of 10")
        if len(payload(inst["count"] - 1, inst["payload"])) != inst["payload"]:
            raise BenchError(f"{run.name}: payload {inst['payload']} too small for the package header")
    need = sum(i["count"] * i["payload"] * (1 + i["clients"]) for i in run.instances)
    if need > max_bytes:
        raise BenchError(f"{run.name}: needs about {need >> 20} MiB in Redis, over --max-bytes")


def select(runs: list[Run], tags: list[str], patterns: list[str]) -> list[Run]:
    out = []
    for r in runs:
        if tags and not set(tags) & set(r.tags):
            continue
        if patterns and not any(fnmatch.fnmatchcase(r.name, p) for p in patterns):
            continue
        out.append(r)
    return out


def load_scenarios(path: Path, scale: float) -> list[Run]:
    with open(path, "rb") as f:
        return expand(tomllib.load(f), scale)


# ---------------------------------------------------------------- payloads and specs


def payload(seq: int, size: int) -> bytes:
    """Deterministic ASCII package of exactly `size` bytes (shorter if size is too small)."""
    head = f'{{"ts": {seq}, "k": {seq % 10}, "seq": {seq}, "p": "'
    pad = size - len(head) - 2
    return (head + "a" * max(pad, 0) + '"}').encode()


def keys_for(run: Run, idx: int, inst: dict[str, Any]) -> tuple[str, list[str]]:
    base = f"{KEY_PREFIX}:{run.name}:i{idx}"
    return f"{base}:src", [f"{base}:t{j}" for j in range(inst["clients"])]


def limits_for(inst: dict[str, Any]) -> dict[str, int] | None:
    # Soft and hard limits are never reached in a run.
    soft, hard = 3 * inst["count"], 4 * inst["count"]
    if inst["limits"] == "countdown":
        # Rust needs all four limits; a timelimit no run reaches makes it a pure countdown.
        return {"timelimit": 3600, "checklimit": 1000, "softlimit": soft, "hardlimit": hard}
    if inst["limits"] == "time":
        return {"timelimit": 1, "checklimit": 1000000, "softlimit": soft, "hardlimit": hard}
    return None


def make_spec(run: Run, idx: int, inst: dict[str, Any], redis_url: str, run_dir: Path, run_id: str) -> dict[str, Any]:
    info = parse_url(redis_url)
    src, targets = keys_for(run, idx, inst)
    clients = []
    for j, key in enumerate(targets):
        clients.append({
            "name": f"t{j}",
            "key": key,
            "filter": CLIENT_FILTER if (inst["filter"] == "client" and j == 0) else None,
            "limits": limits_for(inst),
        })
    return {
        "scenario": run.name,
        "run_id": run_id,
        "instance": idx,
        "transport": inst["transport"],
        "redis": {"host": info["host"], "port": info["port"], "password": info["password"], "ssl": info["ssl"]},
        "source": {"key": src},
        "children": inst["children"],
        "mode": inst["mode"],
        "pid_file": str(run_dir / f"i{idx}.pid"),
        "status_file": str(run_dir / f"i{idx}.status"),
        "filter": ROOT_FILTER if inst["filter"] == "root" else None,
        "ordering": ORDERING if inst["ordering"] else None,
        "clients": clients,
    }


# ---------------------------------------------------------------- execution


@dataclass
class Proc:
    idx: int
    inst: dict[str, Any]
    src: str
    targets: list[str]
    popen: subprocess.Popen[bytes] | None = None
    t0: float = 0.0
    t_change: float = 0.0
    last: tuple[int, ...] = ()
    done: bool = False
    status: str = "ok"
    reason: str = ""
    rusage: Any = None
    exit_code: int | None = None
    stop_s: float = 0.0
    counts: list[int] = field(default_factory=list)
    src_left: int = 0
    status_drop: str = ""
    cfg: Path | None = None
    hwm_kib: int = 0
    t_end: float = 0.0  # end of the measured window: last change (drain) or the time box
    win: tuple[int, ...] = ()  # list lengths (source, targets...) at t_end
    boxed: bool = False  # the time box closed the run (not a drain that came first)


def load_source(r: Redis, key: str, count: int, size: int) -> None:
    cmds: list[list[Any]] = []
    batch: list[Any] = ["RPUSH", key]
    pending = 0
    for seq in range(count):
        batch.append(payload(seq, size))
        pending += size
        if len(batch) >= 1002 or pending >= LOAD_CHUNK_BYTES:
            cmds.append(batch)
            batch = ["RPUSH", key]
            if pending >= LOAD_CHUNK_BYTES:
                _check(r.pipeline(cmds))
                cmds, pending = [], 0
    if len(batch) > 2:
        cmds.append(batch)
    if cmds:
        _check(r.pipeline(cmds))


def _check(replies: list[Any]) -> None:
    for rep in replies:
        if isinstance(rep, RedisError):
            raise rep


def sample_hwm(p: Proc) -> None:
    """Peak RSS from /proc/<pid>/status VmHWM.

    wait4's ru_maxrss is not used: Linux carries the parent's peak RSS across
    fork and exec, so it would report the harness's memory, not the program's.
    """
    if p.popen is None or p.rusage is not None:
        return
    try:
        with open(f"/proc/{p.popen.pid}/status") as f:
            for line in f:
                if line.startswith("VmHWM:"):
                    p.hwm_kib = max(p.hwm_kib, int(line.split()[1]))
                    break
    except OSError:
        pass


def reap(p: Proc, block: bool) -> bool:
    assert p.popen is not None
    if p.rusage is not None:
        return True
    pid, st, ru = os.wait4(p.popen.pid, 0 if block else os.WNOHANG)
    if pid == 0:
        return False
    p.rusage = ru
    p.exit_code = os.waitstatus_to_exitcode(st)
    p.popen.returncode = p.exit_code
    return True


def stop(p: Proc) -> None:
    sample_hwm(p)
    if p.popen is None or reap(p, False):
        return
    t = time.monotonic()
    try:
        os.killpg(p.popen.pid, signal.SIGINT)
    except ProcessLookupError:
        pass
    while time.monotonic() - t < EXIT_WAIT_S:
        sample_hwm(p)
        if reap(p, False):
            break
        time.sleep(0.02)
    else:
        os.killpg(p.popen.pid, signal.SIGKILL)
        reap(p, True)
        p.status, p.reason = "error", f"no exit {EXIT_WAIT_S:.0f} s after SIGINT, killed"
    p.stop_s = time.monotonic() - t


def run_once(run: Run, rep: int, args: argparse.Namespace, r: Redis, env: dict[str, Any]) -> list[dict[str, Any]]:
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_id = f"{args.label}-{stamp}-{run.name}-r{rep}"
    run_dir = Path(args.results) / "logs" / run_id
    run_dir.mkdir(parents=True, exist_ok=True)
    procs: list[Proc] = []
    for idx, inst in enumerate(run.instances):
        src, targets = keys_for(run, idx, inst)
        procs.append(Proc(idx, inst, src, targets))
    all_keys = [k for p in procs for k in [p.src, *p.targets]]

    def rows(status: str, reason: str = "") -> list[dict[str, Any]]:
        for p in procs:
            p.status, p.reason = status, reason
        return build_rows(run, rep, run_id, procs, args, env, 0.0)

    # Render every instance first: an unsupported scenario starts nothing.
    for p in procs:
        spec = make_spec(run, p.idx, p.inst, args.redis, run_dir, run_id)
        spec_path = run_dir / f"i{p.idx}.spec.json"
        spec_path.write_text(json.dumps(spec, indent=2) + "\n")
        cfg = run_dir / f"i{p.idx}.config{args.config_ext}"
        res = subprocess.run([*shlex.split(args.renderer), str(spec_path), str(cfg)], capture_output=True, text=True)
        if res.returncode == RENDER_UNSUPPORTED:
            return rows("n/a", f"renderer: {res.stderr.strip() or 'unsupported'}")
        if res.returncode != 0:
            return rows("error", f"renderer exit {res.returncode}: {res.stderr.strip()}")
        p.cfg = cfg

    r.pipeline([["DEL", k] for k in all_keys])
    for p in procs:
        load_source(r, p.src, p.inst["count"], p.inst["payload"])

    duration = run_duration(run, args)
    settle = args.settle
    if any(p.inst["ordering"] for p in procs):
        settle += ORDERING["buffer_time"] + 1
    h0 = resource.getrusage(resource.RUSAGE_SELF)
    for p in procs:
        cmd = [a.replace("{config}", str(p.cfg)) for a in shlex.split(args.cmd)]
        out = open(run_dir / f"i{p.idx}.stdout.log", "wb")
        err = open(run_dir / f"i{p.idx}.stderr.log", "wb")
        try:
            p.popen = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=out, stderr=err,
                                       start_new_session=True)
        except OSError as e:
            p.status, p.reason, p.done = "error", f"cannot start: {e}", True
        finally:
            out.close()
            err.close()
        p.t0 = p.t_change = time.monotonic()
        p.last = tuple([p.inst["count"]] + [0] * len(p.targets))

    next_reap = 0.0
    while not all(p.done for p in procs):
        time.sleep(POLL_S)
        live = [p for p in procs if not p.done]
        replies = r.pipeline([["LLEN", k] for p in live for k in [p.src, *p.targets]])
        now = time.monotonic()
        pos = 0
        for p in live:
            n = 1 + len(p.targets)
            vals = tuple(int(v) for v in replies[pos:pos + n])
            pos += n
            if vals != p.last:
                p.last, p.t_change = vals, now
            src_left, delivered = vals[0], sum(vals[1:])
            if duration and now - p.t0 >= duration:
                p.done, p.t_end, p.win, p.boxed = True, now, vals, True
            elif src_left == 0 and delivered >= expected_total(p.inst):
                p.done = True
            elif src_left == 0 and now - p.t_change >= settle:
                p.done = True
            elif now - p.t0 > args.timeout:
                p.done, p.status, p.reason = True, "timeout", f"not drained after {args.timeout:.0f} s"
        if now >= next_reap:
            next_reap = now + 0.1
            for p in live:
                sample_hwm(p)
                if not p.done and p.popen is not None and reap(p, False):
                    p.done, p.status = True, "error"
                    p.reason = f"exited early with code {p.exit_code}"
    h1 = resource.getrusage(resource.RUSAGE_SELF)
    harness_cpu = (h1.ru_utime - h0.ru_utime) + (h1.ru_stime - h0.ru_stime)
    for p in procs:
        if not p.win:
            # Drained, settled, timed out or failed before the time box: the window ends at the last change.
            p.t_end, p.win = p.t_change, p.last

    for p in procs:
        stop(p)
        p.src_left, p.counts = p.last[0], list(p.last[1:])
        if p.exit_code not in (0, None) and p.status == "ok":
            p.status, p.reason = "error", f"exit code {p.exit_code} after SIGINT"
        status_file = run_dir / f"i{p.idx}.status"
        if status_file.exists():
            try:
                p.status_drop = str(json.loads(status_file.read_text()).get("total_drop", ""))
            except (ValueError, OSError):
                p.status_drop = "unreadable"
    final = r.pipeline([["LLEN", k] for p in procs for k in [p.src, *p.targets]])
    pos = 0
    for p in procs:
        n = 1 + len(p.targets)
        vals = [int(v) for v in final[pos:pos + n]]
        pos += n
        if sum(vals[1:]) != sum(p.counts):
            # Packages delivered during shutdown count as delivered, not as throughput.
            p.counts = vals[1:]
        p.src_left = vals[0]
    if not args.keep_keys:
        r.pipeline([["UNLINK", k] for k in all_keys])
    return build_rows(run, rep, run_id, procs, args, env, harness_cpu)


def run_duration(run: Run, args: argparse.Namespace) -> float:
    """Time box in seconds (0 = drain): --duration overrides the scenario's value."""
    d = getattr(args, "duration", None)
    return run.duration if d is None else d


def _window(p: Proc, timed: bool) -> tuple[float, int, int]:
    """Measured span, packages consumed and packages delivered in it.

    Drain runs keep their 1.0 definition: consumed counts the source as left after exit.
    Time-boxed runs count what the lists held when the time box closed; packages still in
    flight then are delivered during the stop and count as delivered, not as throughput.
    """
    span = p.t_end - p.t0
    if timed:
        return span, p.inst["count"] - p.win[0], sum(p.win[1:])
    return span, p.inst["count"] - p.src_left, sum(p.counts)


def build_rows(run: Run, rep: int, run_id: str, procs: list[Proc], args: argparse.Namespace,
               env: dict[str, Any], harness_cpu: float) -> list[dict[str, Any]]:
    ts = dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    duration = run_duration(run, args)
    timed = duration > 0
    out = []
    for p in procs:
        inst = p.inst
        # A source that drained before the box closed is a drain run, not a time-boxed one (MQF-2)
        boxed = timed and p.boxed
        if timed and not boxed and p.status == "ok":
            p.reason = "source drained before the time box"
        drain, consumed, delivered_w = _window(p, boxed) if p.popen is not None else (0.0, 0, 0)
        delivered = sum(p.counts)
        # A time-boxed run leaves packages in the source: only those taken from it are expected.
        if boxed and p.popen is not None:
            exp = expected_total(inst, inst["count"] - p.src_left)
        else:
            exp = expected_total(inst)
        row = {
            "ts_utc": ts, "label": args.label, "run_id": run_id,
            "scenario": f"{run.name}.i{p.idx}" if run.mixed else run.name,
            "tags": " ".join(run.tags),
            "transport": inst["transport"], "count": inst["count"], "payload": inst["payload"],
            "children": inst["children"], "mode": inst["mode"], "clients": inst["clients"],
            "ordering": _fmt(inst["ordering"]), "filter": inst["filter"], "limits": inst["limits"],
            "repeat_idx": rep, "status": p.status, "reason": p.reason,
            "msg_s": f"{consumed / drain:.1f}" if drain > 0 and p.status != "n/a" else "",
            "mib_s": f"{delivered_w * inst['payload'] / drain / 1048576:.3f}" if drain > 0 and p.status != "n/a" else "",
            "drain_s": f"{drain:.3f}" if p.popen is not None else "",
            "stop_s": f"{p.stop_s:.3f}" if p.popen is not None else "",
            "wall_s": f"{drain + p.stop_s:.3f}" if p.popen is not None else "",
            "expected_total": exp,
            "delivered_total": delivered if p.popen is not None else "",
            "missing": exp - delivered if p.popen is not None else "",
            "per_target": ";".join(str(c) for c in p.counts),
            "status_file_drop": p.status_drop,
            "cpu_user_s": f"{p.rusage.ru_utime:.3f}" if p.rusage else "",
            "cpu_sys_s": f"{p.rusage.ru_stime:.3f}" if p.rusage else "",
            "max_rss_kib": p.hwm_kib or "",
            "harness_cpu_s": f"{harness_cpu:.3f}" if p.popen is not None else "",
            "redis_version": env["redis_version"], "host_cpus": env["host_cpus"],
            "duration_s": f"{duration:g}" if boxed else "",
        }
        out.append(row)
    if run.mixed and any(p.popen is not None for p in procs):
        started = [p for p in procs if p.popen is not None]
        span = max(p.t_end for p in started) - min(p.t0 for p in started)
        windows = [_window(p, timed and p.boxed) for p in started]
        consumed = sum(w[1] for w in windows)
        delivered_bytes = sum(w[2] * p.inst["payload"] for w, p in zip(windows, started))
        status = "ok" if all(p.status == "ok" for p in procs) else next(p.status for p in procs if p.status != "ok")
        agg = dict(out[0])
        agg.update({
            "scenario": run.name, "status": status, "reason": "aggregate of all instances",
            "count": sum(p.inst["count"] for p in procs), "children": "", "mode": "mixed",
            "clients": sum(p.inst["clients"] for p in procs), "ordering": "", "filter": "", "limits": "",
            "payload": "", "msg_s": f"{consumed / span:.1f}" if span > 0 else "",
            "mib_s": f"{delivered_bytes / span / 1048576:.3f}" if span > 0 else "",
            "drain_s": f"{span:.3f}", "stop_s": f"{max(p.stop_s for p in started):.3f}",
            "wall_s": f"{span + max(p.stop_s for p in started):.3f}",
            "expected_total": sum(int(o["expected_total"]) for o in out),
            "delivered_total": sum(int(o["delivered_total"] or 0) for o in out),
            "missing": sum(int(o["missing"] or 0) for o in out), "per_target": "",
            "status_file_drop": "",
            "duration_s": f"{duration:g}" if timed and all(p.boxed for p in started) else "",
            "cpu_user_s": f"{sum(p.rusage.ru_utime for p in started if p.rusage):.3f}",
            "cpu_sys_s": f"{sum(p.rusage.ru_stime for p in started if p.rusage):.3f}",
            "max_rss_kib": max((p.hwm_kib for p in started if p.hwm_kib), default=""),
        })
        out.append(agg)
    return out


# ---------------------------------------------------------------- reports


def append_csv(path: Path, rows: list[dict[str, Any]]) -> None:
    new = not path.exists()
    if not new:
        with open(path, newline="") as f:
            header = next(csv.reader(f), [])
        if header != CSV_FIELDS:
            raise BenchError(f"{path} has different columns (older harness?); use a new --results directory")
    with open(path, "a", newline="") as f:
        w = csv.DictWriter(f, fieldnames=CSV_FIELDS)
        if new:
            w.writeheader()
        w.writerows(rows)


def _median(vals: list[str]) -> float | None:
    nums = [float(v) for v in vals if v not in ("", None)]
    return statistics.median(nums) if nums else None


def _max_missing(rows: list[dict[str, Any]]) -> float | None:
    """Worst `missing` across repetitions: a median would hide one lossy repetition (M2R-4)."""
    nums = [float(r["missing"]) for r in rows if r.get("missing") not in ("", None)]
    return max(nums, key=abs) if nums else None


def lossy(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Rows whose targets did not receive exactly the expected packages."""
    return [r for r in rows if r.get("missing") not in ("", None) and int(r["missing"]) != 0]


def _num(v: float | None, fmt: str = "{:.0f}") -> str:
    return "n/a" if v is None else fmt.format(v)


def markdown_report(rows: list[dict[str, Any]], label: str) -> str:
    groups: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        groups.setdefault(row["scenario"], []).append(row)
    lines = [
        f"## {label}",
        "",
        "| scenario | status | msg/s | MiB/s | drain s | stop s | missing (worst) | cpu user s | cpu sys s | max RSS MiB |",
        "|---|---|---|---|---|---|---|---|---|---|",
    ]
    for name, rs in groups.items():
        statuses = sorted({r["status"] for r in rs})
        ok = [r for r in rs if r["status"] in ("ok", "timeout")]
        rss = _median([str(r["max_rss_kib"]) for r in ok])
        lines.append(
            f"| {name} | {','.join(statuses)} "
            f"| {_num(_median([r['msg_s'] for r in ok]))} | {_num(_median([r['mib_s'] for r in ok]), '{:.2f}')} "
            f"| {_num(_median([r['drain_s'] for r in ok]), '{:.2f}')} | {_num(_median([r['stop_s'] for r in ok]), '{:.2f}')} "
            f"| {_num(_max_missing(rs))} "
            f"| {_num(_median([r['cpu_user_s'] for r in ok]), '{:.2f}')} | {_num(_median([r['cpu_sys_s'] for r in ok]), '{:.2f}')} "
            f"| {_num(None if rss is None else rss / 1024, '{:.1f}')} |"
        )
    return "\n".join(lines) + "\n"


def read_results(path: Path, label: str | None) -> tuple[str, dict[str, list[dict[str, str]]]]:
    with open(path, newline="") as f:
        rows = list(csv.DictReader(f))
    labels = sorted({r["label"] for r in rows})
    if label is None:
        if len(labels) > 1:
            raise BenchError(f"{path} holds several labels {labels}; choose one with --label-a/--label-b")
        label = labels[0] if labels else path.stem
    groups: dict[str, list[dict[str, str]]] = {}
    for r in rows:
        if r["label"] == label:
            groups.setdefault(r["scenario"], []).append(r)
    return label, groups


def compare_table(a_label: str, a: dict[str, list[dict[str, str]]], b_label: str,
                  b: dict[str, list[dict[str, str]]]) -> str:
    lines = [
        f"| scenario | {a_label} msg/s | {b_label} msg/s | ratio {b_label}/{a_label} | {a_label} missing (worst) | {b_label} missing (worst) |",
        "|---|---|---|---|---|---|",
    ]
    for name in list(dict.fromkeys([*a, *b])):
        def stat(rows: list[dict[str, str]] | None, col: str) -> float | None:
            if not rows:
                return None
            return _median([r[col] for r in rows if r["status"] in ("ok", "timeout")])
        ma, mb = stat(a.get(name), "msg_s"), stat(b.get(name), "msg_s")
        ratio = f"{mb / ma:.2f}" if ma and mb is not None else "n/a"
        lines.append(
            f"| {name} | {_num(ma)} | {_num(mb)} | {ratio} "
            f"| {_num(_max_missing(a.get(name, [])))} | {_num(_max_missing(b.get(name, [])))} |"
        )
    return "\n".join(lines) + "\n"


def count_mismatches(a: dict[str, list[dict[str, str]]], b: dict[str, list[dict[str, str]]]) -> list[str]:
    """Scenarios present in both files whose package counts or time boxes differ (M2F-2)."""
    out = []
    for name in a:
        if name not in b:
            continue
        for col in ("count", "duration_s"):
            va = sorted({r.get(col, "") for r in a[name]})
            vb = sorted({r.get(col, "") for r in b[name]})
            if va != vb:
                out.append(f"{name}: {col} {','.join(va) or '-'} vs {','.join(vb) or '-'}")
    return out


# ---------------------------------------------------------------- commands


def cmd_list(args: argparse.Namespace) -> int:
    runs = select(load_scenarios(Path(args.scenarios), args.scale), args.tag, args.scenario)
    for run in runs:
        desc = "; ".join(
            f"k{i['count']} p{i['payload']} c{i['children']} {i['mode']} n{i['clients']} "
            f"o{_fmt(i['ordering'])} f{i['filter']} l{i['limits']}" for i in run.instances
        )
        box = f" (time box {run.duration:g} s)" if run.duration else ""
        print(f"{run.name:28} {','.join(run.tags):8} {desc}{box}")
    print(f"{len(runs)} runs")
    return 0


def cmd_clean(args: argparse.Namespace) -> int:
    removed = 0
    with Redis(args.redis) as r:
        cursor = "0"
        while True:
            cursor_b, keys = r.execute("SCAN", cursor, "MATCH", f"{KEY_PREFIX}:*", "COUNT", 1000)
            cursor = cursor_b.decode()
            if keys:
                removed += int(r.execute("UNLINK", *keys))
            if cursor == "0":
                break
    print(f"removed {removed} keys matching {KEY_PREFIX}:*")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    if "{config}" not in args.cmd:
        raise BenchError("--cmd must contain {config}")
    runs = select(load_scenarios(Path(args.scenarios), args.scale), args.tag, args.scenario)
    if not runs:
        raise BenchError("no scenario selected")
    if args.duration is not None and args.duration < 0:
        raise BenchError("--duration must be 0 or positive")
    for run in runs:
        validate(run, args.max_bytes)
        if run_duration(run, args) >= args.timeout:
            raise BenchError(f"{run.name}: duration must be shorter than --timeout")
    results = Path(args.results)
    results.mkdir(parents=True, exist_ok=True)
    csv_path = results / "results.csv"
    with Redis(args.redis) as r:
        info = r.execute("INFO", "server").decode()
        version = next((ln.split(":", 1)[1].strip() for ln in info.splitlines() if ln.startswith("redis_version:")), "")
        env = {"redis_version": version, "host_cpus": os.cpu_count()}
        all_rows: list[dict[str, Any]] = []
        for run in runs:
            for rep in range(args.repeat):
                rows = run_once(run, rep, args, r, env)
                append_csv(csv_path, rows)
                all_rows.extend(rows)
                for row in rows:
                    print(f"{row['scenario']:32} r{rep} {row['status']:7} msg/s={row['msg_s'] or '-':>9} "
                          f"missing={row['missing']!s:>6} {row['reason']}", flush=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    md_path = results / f"report-{args.label}-{stamp}.md"
    md_path.write_text(markdown_report(all_rows, args.label))
    print(f"\nCSV: {csv_path}\nreport: {md_path}\n")
    print(md_path.read_text())
    for row in lossy(all_rows):
        print(f"MISSING: {row['scenario']} r{row['repeat_idx']} missing={row['missing']}", file=sys.stderr)
    return run_exit_code(all_rows)


def run_exit_code(rows: list[dict[str, Any]]) -> int:
    """1 if any run ended in error, EXIT_MISSING if any repetition lost or duplicated packages."""
    if any(r["status"] == "error" for r in rows):
        return 1
    return EXIT_MISSING if lossy(rows) else 0


def cmd_compare(args: argparse.Namespace) -> int:
    la, a = read_results(Path(args.a), args.label_a)
    lb, b = read_results(Path(args.b), args.label_b)
    if args.scenario:
        keep = lambda name: any(fnmatch.fnmatchcase(name, pat) for pat in args.scenario)  # noqa: E731
        a = {k: v for k, v in a.items() if keep(k)}
        b = {k: v for k, v in b.items() if keep(k)}
    if la == lb:
        la, lb = f"A:{la}", f"B:{lb}"
    for warning in count_mismatches(a, b):
        print(f"warning: not comparable, {warning}", file=sys.stderr)
    print(compare_table(la, a, lb, b), end="")
    return 0


def parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)

    def common(p: argparse.ArgumentParser) -> None:
        p.add_argument("--scenarios", default=str(DEFAULT_SCENARIOS), help="scenario file (TOML)")
        p.add_argument("--tag", action="append", default=[], help="select runs with this tag (repeatable)")
        p.add_argument("--scenario", action="append", default=[], help="select runs by name or glob (repeatable)")
        p.add_argument("--scale", type=float, default=1.0, help="multiply every package count")

    p = sub.add_parser("run", help="run the selected scenarios")
    common(p)
    p.add_argument("--cmd", required=True, help="program under test; {config} is replaced by the config path")
    p.add_argument("--renderer", default=DEFAULT_RENDERER, help="command: <renderer> <spec.json> <out_config>")
    p.add_argument("--config-ext", default=".yaml", help="extension of the rendered config file")
    p.add_argument("--label", required=True, help="implementation label written to every row")
    p.add_argument("--redis", default=DEFAULT_REDIS, help="Redis URL (default $RM_BENCH_REDIS_URL or local 6379)")
    p.add_argument("--results", required=True, help="results directory")
    p.add_argument("--repeat", type=int, default=1)
    p.add_argument("--timeout", type=float, default=120.0, help="seconds per run before giving up")
    p.add_argument("--settle", type=float, default=2.0, help="seconds without progress that end a run")
    p.add_argument("--max-bytes", type=int, default=1 << 30, help="Redis memory budget per run")
    p.add_argument("--keep-keys", action="store_true", help="keep the run's keys after it ends")
    p.add_argument("--duration", type=float, default=None,
                   help="time box in seconds for every selected run (0 = drain); default: the scenario's")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("list", help="list the expanded runs")
    common(p)
    p.set_defaults(func=cmd_list)

    p = sub.add_parser("clean", help=f"delete every {KEY_PREFIX}:* key (SCAN + UNLINK)")
    p.add_argument("--redis", default=DEFAULT_REDIS)
    p.set_defaults(func=cmd_clean)

    p = sub.add_parser("compare", help="compare two results.csv files")
    p.add_argument("a")
    p.add_argument("b")
    p.add_argument("--label-a")
    p.add_argument("--label-b")
    p.add_argument("--scenario", action="append", default=[], help="only scenarios matching this glob (repeatable)")
    p.set_defaults(func=cmd_compare)
    return ap


def main(argv: list[str] | None = None) -> int:
    args = parser().parse_args(argv)
    try:
        return args.func(args)
    except (BenchError, RedisError, OSError) as e:
        print(f"rmbench: error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
