"""Unit tests for the bench harness (no Redis needed).

Run: python3 -m unittest discover -s bench -p 'test_*.py'
"""

from __future__ import annotations

import argparse
import contextlib
import io
import json
import os
import re
import signal
import socket
import sys
import tempfile
import threading
import tomllib
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import render_rust  # noqa: E402
import rmbench  # noqa: E402
from resp import Redis, RedisError, encode, parse_url  # noqa: E402


def doc(text: str) -> dict:
    return tomllib.loads(text)


BASE = """
[defaults]
transport = "list"
count = 1000
payload = 200
children = 2
mode = "replicant"
clients = 1
ordering = false
filter = "off"
limits = "off"
"""


class MatrixTests(unittest.TestCase):
    def test_cartesian_expansion_and_names(self) -> None:
        runs = rmbench.expand(doc(BASE + """
[[scenario]]
name = "s"
tags = ["full"]
payload = [64, 200]
children = [1, 8]
"""))
        self.assertEqual([r.name for r in runs], ["s.p64.c1", "s.p64.c8", "s.p200.c1", "s.p200.c8"])
        self.assertEqual(runs[3].instances[0]["children"], 8)
        self.assertEqual(runs[3].instances[0]["payload"], 200)

    def test_mixed_instances_inherit(self) -> None:
        runs = rmbench.expand(doc(BASE + """
[[scenario]]
name = "m"
instances = [{ mode = "spreader", clients = 3 }, { ordering = true }]
"""))
        self.assertEqual(len(runs), 1)
        self.assertTrue(runs[0].mixed)
        a, b = runs[0].instances
        self.assertEqual((a["mode"], a["clients"], a["ordering"]), ("spreader", 3, False))
        self.assertEqual((b["mode"], b["clients"], b["ordering"]), ("replicant", 1, True))

    def test_scale_rounds_to_multiple_of_30(self) -> None:
        runs = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n'), scale=0.5)
        self.assertEqual(runs[0].instances[0]["count"], 510)

    def test_scale_one_keeps_the_count_as_written(self) -> None:
        for scale in (1.0, 1.0 + 1e-12):
            runs = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n'), scale=scale)
            self.assertEqual(runs[0].instances[0]["count"], 1000, scale)
        runs = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n'), scale=1.001)
        self.assertEqual(runs[0].instances[0]["count"], 990)

    def test_duplicate_names_rejected(self) -> None:
        with self.assertRaises(rmbench.BenchError):
            rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n[[scenario]]\nname = "s"\n'))

    def test_memory_budget(self) -> None:
        run = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n'))[0]
        with self.assertRaises(rmbench.BenchError):
            rmbench.validate(run, 1000)

    def test_only_list_transport(self) -> None:
        run = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\ntransport = "pubsub"\n'))[0]
        with self.assertRaises(rmbench.BenchError):
            rmbench.validate(run, 1 << 30)

    def test_repository_scenarios_are_valid(self) -> None:
        runs = rmbench.load_scenarios(rmbench.DEFAULT_SCENARIOS, 1.0)
        for run in runs:
            rmbench.validate(run, 1 << 30)
        tags = {t for r in runs for t in r.tags}
        self.assertEqual(tags, {"smoke", "fast", "quick", "full"})
        fast = [r for r in runs if "fast" in r.tags]
        self.assertTrue(fast and all(r.duration > 0 for r in fast))
        self.assertTrue(all(r.duration == 0 for r in runs if "fast" not in r.tags))

    def test_expected_counts(self) -> None:
        inst = {"count": 100, "mode": "replicant", "clients": 3, "filter": "client"}
        self.assertEqual(rmbench.expected_per_target(inst), [50, 100, 100])
        inst = {"count": 99, "mode": "spreader", "clients": 3, "filter": "off"}
        self.assertIsNone(rmbench.expected_per_target(inst))
        self.assertEqual(rmbench.expected_total(inst), 99)
        inst = {"count": 100, "mode": "replicant", "clients": 2, "filter": "root"}
        self.assertEqual(rmbench.expected_total(inst), 100)

    def test_expected_counts_for_first_n(self) -> None:
        # k = seq mod 10; the first 15 packages hold k 0-4 ten times.
        self.assertEqual(rmbench.matching(15), 10)
        self.assertEqual(rmbench.matching(14), 9)
        inst = {"count": 900, "mode": "replicant", "clients": 2, "filter": "client"}
        self.assertEqual(rmbench.expected_per_target(inst, 15), [10, 15])
        inst = {"count": 900, "mode": "spreader", "clients": 3, "filter": "root"}
        self.assertEqual(rmbench.expected_total(inst, 15), 10)

    def test_duration(self) -> None:
        runs = rmbench.expand(doc(BASE + '[[scenario]]\nname = "t"\nduration = 5\n[[scenario]]\nname = "d"\n'))
        self.assertEqual([r.duration for r in runs], [5.0, 0.0])
        self.assertNotIn("duration", runs[0].instances[0])
        self.assertEqual(rmbench.run_duration(runs[0], argparse.Namespace(duration=None)), 5.0)
        self.assertEqual(rmbench.run_duration(runs[0], argparse.Namespace(duration=0.0)), 0.0)
        self.assertEqual(rmbench.run_duration(runs[1], argparse.Namespace(duration=3.0)), 3.0)
        with self.assertRaises(rmbench.BenchError):
            rmbench.expand(doc(BASE + '[[scenario]]\nname = "t"\nduration = -1\n'))


class PayloadTests(unittest.TestCase):
    def test_exact_size_and_patterns(self) -> None:
        for size in (64, 200, 4096, 65536):
            for seq in (0, 7, 39999):
                p = rmbench.payload(seq, size)
                self.assertEqual(len(p), size)
                text = p.decode("ascii")
                head = text[: rmbench.ORDERING["limit"]]
                m = re.search(rmbench.ORDERING["regex"], head)
                self.assertIsNotNone(m)
                self.assertEqual(int(m.group("ts")), seq)

    def test_filters_pass_half(self) -> None:
        texts = [rmbench.payload(i, 200).decode()[:64] for i in range(1000)]
        root = sum(bool(re.search(rmbench.ROOT_FILTER["regex"], t)) for t in texts)
        client = sum(bool(re.search(rmbench.CLIENT_FILTER["regex"], t)) for t in texts)
        self.assertEqual((root, client), (500, 500))


class RenderTests(unittest.TestCase):
    def spec(self, **kw: object) -> dict:
        run = rmbench.expand(doc(BASE + '[[scenario]]\nname = "s"\n'))[0]
        inst = {**run.instances[0], **kw}
        return rmbench.make_spec(run, 0, inst, "redis://:pw@10.0.0.1:6390", Path("/tmp/x"), "id")

    def test_plain(self) -> None:
        text = render_rust.render(self.spec())
        assert text is not None
        self.assertIn("hostname: '10.0.0.1'\nport: 6390\npassword: 'pw'\n", text)
        self.assertIn("channel: 'rmbench:s:i0:src'", text)
        self.assertIn("  - name: 't0'\n", text)
        self.assertIn("    channel: 'rmbench:s:i0:t0'", text)
        self.assertNotIn("filter", text)

    def test_features(self) -> None:
        text = render_rust.render(self.spec(ordering=True, filter="client", limits="countdown"))
        assert text is not None
        self.assertIn("ordering: '\"ts\": *(?P<ts>\\d+),'", text)
        self.assertIn("ordering_buffer_time: 1\n", text)
        self.assertIn("    filter_replace: '\"K\": $1,'", text)
        self.assertIn("    timelimit: 3600\n    checklimit: 1000\n", text)

    def test_unsupported_transport(self) -> None:
        spec = self.spec()
        spec["transport"] = "pubsub"
        self.assertIsNone(render_rust.render(spec))
        with tempfile.TemporaryDirectory() as d:
            sp = Path(d) / "spec.json"
            sp.write_text(json.dumps(spec))
            self.assertEqual(render_rust.main(["r", str(sp), str(Path(d) / "out")]), 3)


class RespTests(unittest.TestCase):
    def test_encode(self) -> None:
        self.assertEqual(encode(["LLEN", "k", 5, b"\xff"]), b"*4\r\n$4\r\nLLEN\r\n$1\r\nk\r\n$1\r\n5\r\n$1\r\n\xff\r\n")

    def test_parse_url(self) -> None:
        self.assertEqual(parse_url("rediss://u:p%40w@h:7000/2"),
                         {"host": "h", "port": 7000, "username": "u", "password": "p@w", "ssl": True, "db": 2})
        with self.assertRaises(ValueError):
            parse_url("http://h")

    def test_replies_over_socket(self) -> None:
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        port = srv.getsockname()[1]
        replies = b"+PONG\r\n:3\r\n$-1\r\n*2\r\n$1\r\na\r\n-ERR inner\r\n-ERR bad\r\n"

        def serve() -> None:
            conn, _ = srv.accept()
            got = b""
            while got.count(b"*") < 4:
                got += conn.recv(4096)
            conn.sendall(replies)
            conn.recv(1)
            conn.close()

        t = threading.Thread(target=serve)
        t.start()
        try:
            with Redis(f"redis://127.0.0.1:{port}") as r:
                out = r.pipeline([["PING"], ["LLEN", "k"], ["GET", "x"], ["LRANGE", "k", 0, -1]])
                self.assertEqual(out[:3], ["PONG", 3, None])
                self.assertEqual(out[3][0], b"a")
                self.assertIsInstance(out[3][1], RedisError)
                with self.assertRaises(RedisError):
                    r._read()
        finally:
            t.join()
            srv.close()


class WindowTests(unittest.TestCase):
    ENV = {"redis_version": "7", "host_cpus": 2}

    def proc(self, **kw: object) -> rmbench.Proc:
        inst = {"transport": "list", "count": 900, "payload": 200, "children": 2, "mode": "replicant",
                "clients": 1, "ordering": False, "filter": "off", "limits": "off"}
        p = rmbench.Proc(0, inst, "src", ["t0"])
        p.popen = object()  # type: ignore[assignment]
        for k, v in kw.items():
            setattr(p, k, v)
        return p

    def row(self, run: rmbench.Run, p: rmbench.Proc, duration: float | None = None) -> dict:
        args = argparse.Namespace(label="x", duration=duration)
        return rmbench.build_rows(run, 0, "id", [p], args, self.ENV, 0.0)[0]

    def test_time_boxed_counts_the_window(self) -> None:
        run = rmbench.Run("t", ["fast"], [], duration=5.0)
        # At the box: 500 taken, 480 delivered; after the stop: 510 taken, all delivered.
        p = self.proc(t0=10.0, t_change=14.9, t_end=15.0, win=(400, 480), src_left=390, counts=[510], boxed=True)
        r = self.row(run, p)
        self.assertEqual((r["msg_s"], r["mib_s"], r["drain_s"]), ("100.0", "0.018", "5.000"))
        self.assertEqual((r["expected_total"], r["delivered_total"], r["missing"]), (510, 510, 0))
        self.assertEqual(r["duration_s"], "5")

    def test_drain_run_unchanged(self) -> None:
        run = rmbench.Run("d", ["quick"], [])
        p = self.proc(t0=10.0, t_change=12.0, t_end=12.0, win=(0, 900), src_left=0, counts=[900])
        r = self.row(run, p)
        self.assertEqual((r["msg_s"], r["drain_s"], r["missing"], r["duration_s"]), ("450.0", "2.000", 0, ""))

    def test_duration_override_turns_box_off(self) -> None:
        run = rmbench.Run("t", ["fast"], [], duration=5.0)
        p = self.proc(t0=0.0, t_change=3.0, t_end=3.0, win=(0, 900), src_left=0, counts=[900])
        r = self.row(run, p, duration=0.0)
        self.assertEqual((r["msg_s"], r["duration_s"]), ("300.0", ""))

    def test_mixed_aggregate_is_boxed_only_when_every_instance_is(self) -> None:
        # M3R-11: one instance boxed, the other drained first: the instance rows say so and
        # the aggregate is not a time-boxed row.
        a = self.proc(t0=10.0, t_change=14.9, t_end=15.0, win=(400, 480), src_left=390, counts=[510], boxed=True)
        b = self.proc(idx=1, t0=10.0, t_change=12.0, t_end=12.0, win=(0, 900), src_left=0, counts=[900])
        run = rmbench.Run("m", ["fast"], [a.inst, b.inst], mixed=True, duration=5.0)
        rows = rmbench.build_rows(run, 0, "id", [a, b], argparse.Namespace(label="x", duration=None), self.ENV, 0.0)
        self.assertEqual([r["scenario"] for r in rows], ["m.i0", "m.i1", "m"])
        self.assertEqual([r["duration_s"] for r in rows], ["5", "", ""])
        a.boxed = b.boxed = True
        b.win = (100, 800)
        rows = rmbench.build_rows(run, 0, "id", [a, b], argparse.Namespace(label="x", duration=None), self.ENV, 0.0)
        self.assertEqual(rows[-1]["duration_s"], "5")


class ReportTests(unittest.TestCase):
    def rows(self, label: str, msg: list[str]) -> list[dict]:
        return [{"label": label, "scenario": "s", "status": "ok", "msg_s": m, "missing": "0"} for m in msg]

    def test_compare(self) -> None:
        a = {"s": self.rows("a", ["100", "300", "200"]), "only_a": self.rows("a", ["5"])}
        b = {"s": self.rows("b", ["400"])}
        table = rmbench.compare_table("a", a, "b", b)
        self.assertIn("| s | 200 | 400 | 2.00 | 0 | 0 |", table)
        self.assertIn("| only_a | 5 | n/a | n/a | 0 | n/a |", table)

    def test_read_results_requires_label_when_mixed(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "r.csv"
            rmbench.append_csv(p, [dict.fromkeys(rmbench.CSV_FIELDS, "") | {"label": "x", "scenario": "s"},
                                   dict.fromkeys(rmbench.CSV_FIELDS, "") | {"label": "y", "scenario": "s"}])
            with self.assertRaises(rmbench.BenchError):
                rmbench.read_results(p, None)
            label, groups = rmbench.read_results(p, "y")
            self.assertEqual((label, list(groups)), ("y", ["s"]))

    def full_row(self, missing: str, rep: int = 0) -> dict:
        return dict.fromkeys(rmbench.CSV_FIELDS, "") | {
            "label": "a", "scenario": "s", "status": "ok", "msg_s": "100", "missing": missing,
            "repeat_idx": rep, "count": "900", "duration_s": "5"}

    def test_report_and_compare_show_the_worst_missing(self) -> None:
        # One lossy repetition out of three must not disappear behind the median (M2R-4).
        rows = [self.full_row(m, i) for i, m in enumerate(["0", "26955", "0"])]
        self.assertIn("| 26955 |", rmbench.markdown_report(rows, "a"))
        # M3R-11: a repetition that failed still counts for the worst `missing`.
        failed = [self.full_row("0"), self.full_row("7", 1) | {"status": "error"}]
        self.assertIn("| 7 |", rmbench.markdown_report(failed, "a"))
        table = rmbench.compare_table("a", {"s": rows}, "b", {"s": [self.full_row("0")]})
        self.assertIn("| s | 100 | 100 | 1.00 | 26955 | 0 |", table)
        # Duplicates (negative missing) are reported as well.
        self.assertIn("| -2 |", rmbench.markdown_report([self.full_row("0"), self.full_row("-2")], "a"))

    def test_lossy_rows(self) -> None:
        rows = [self.full_row("0"), self.full_row("7", 1), self.full_row(""), self.full_row("-1", 2)]
        self.assertEqual([r["repeat_idx"] for r in rmbench.lossy(rows)], [1, 2])

    def test_run_exit_code(self) -> None:
        self.assertEqual(rmbench.run_exit_code([self.full_row("0"), self.full_row("0")]), 0)
        self.assertEqual(rmbench.run_exit_code([self.full_row("0"), self.full_row("3")]), rmbench.EXIT_MISSING)
        self.assertEqual(rmbench.run_exit_code([self.full_row("3"), self.full_row("0") | {"status": "error"}]), 1)

    def test_count_mismatches(self) -> None:
        a = {"s": [self.full_row("0")], "only_a": [self.full_row("0")]}
        b = {"s": [self.full_row("0") | {"count": "27000"}]}
        self.assertEqual(rmbench.count_mismatches(a, b), ["s: count 900 vs 27000"])
        b = {"s": [self.full_row("0") | {"duration_s": ""}]}
        self.assertEqual(rmbench.count_mismatches(a, b), ["s: duration_s 5 vs -"])
        self.assertEqual(rmbench.count_mismatches(a, {"s": [self.full_row("0")]}), [])

    def test_append_refuses_other_columns(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "r.csv"
            p.write_text("ts_utc,label\n")
            with self.assertRaises(rmbench.BenchError):
                rmbench.append_csv(p, [dict.fromkeys(rmbench.CSV_FIELDS, "")])


class FakeRedis:
    """Stand-in for resp.Redis in run_once: the source loses `step` packages per LLEN round
    and they appear in the first target, so every package is accounted for."""

    def __init__(self, step: int, lose: bool = False) -> None:
        self.step = step
        self.lose = lose  # packages leave the source but never reach a target
        self.lists: dict[str, int] = {}
        self.src = ""

    def __enter__(self) -> "FakeRedis":
        return self

    def __exit__(self, *exc: object) -> None:
        pass

    def execute(self, *cmd: object) -> bytes:
        assert cmd == ("INFO", "server"), cmd
        return b"redis_version:7.0.0\r\n"

    def pipeline(self, cmds: list[list[object]]) -> list[object]:
        out: list[object] = []
        if cmds and cmds[0][0] == "LLEN":
            src, tgt = str(cmds[0][1]), str(cmds[1][1])
            moved = min(self.step, self.lists.get(src, 0))
            self.lists[src] = self.lists.get(src, 0) - moved
            if not self.lose:
                self.lists[tgt] = self.lists.get(tgt, 0) + moved
        for cmd in cmds:
            name, key = cmd[0], str(cmd[1])
            if name == "RPUSH":
                self.lists[key] = self.lists.get(key, 0) + len(cmd) - 2
                out.append(self.lists[key])
            elif name == "LLEN":
                out.append(self.lists.get(key, 0))
            else:  # DEL, UNLINK
                self.lists.pop(key, None)
                out.append(0)
        return out


class RunOnceTests(unittest.TestCase):
    """run_once end to end with a fake Redis and a program that only waits for SIGINT (M2R-3)."""

    PROGRAM = (f"{sys.executable} -c 'import signal, sys, time; "
               f"signal.signal(signal.SIGINT, lambda *a: sys.exit(0)); time.sleep(60)' {{config}}")

    def run_box(self, box: float, count: int = 900000) -> dict:
        run = rmbench.expand(doc(BASE + f'[[scenario]]\nname = "t"\nduration = {box}\ncount = {count}\n'))[0]
        with tempfile.TemporaryDirectory() as d:
            args = argparse.Namespace(
                label="x", results=d, renderer=rmbench.DEFAULT_RENDERER, config_ext=".yaml",
                redis="redis://127.0.0.1:6379", cmd=self.PROGRAM, settle=2.0, timeout=10.0,
                keep_keys=False, duration=None)
            rows = rmbench.run_once(run, 0, args, FakeRedis(step=100), {"redis_version": "7", "host_cpus": 2})
        self.assertEqual(len(rows), 1)
        return rows[0]

    def test_time_box_closes_the_run(self) -> None:
        r = self.run_box(0.5)
        self.assertEqual((r["status"], r["duration_s"], r["missing"]), ("ok", "0.5", 0))
        # The box closes at the first poll after 0.5 s; a doubled or ignored box fails here.
        self.assertGreaterEqual(float(r["drain_s"]), 0.5)
        self.assertLess(float(r["drain_s"]), 0.9)
        self.assertGreater(int(r["delivered_total"]), 0)
        self.assertGreater(float(r["msg_s"]), 0.0)
        self.assertEqual(r["reason"], "")

    def test_source_drained_before_the_box_is_a_drain_run(self) -> None:
        # MQF-2: 500 packages at 100 per poll drain in about 50 ms, long before the 3 s box.
        r = self.run_box(3.0, count=500)
        self.assertEqual((r["status"], r["duration_s"], r["missing"]), ("ok", "", 0))
        self.assertEqual(r["reason"], "source drained before the time box")
        self.assertEqual((r["expected_total"], r["delivered_total"]), (500, 500))
        self.assertLess(float(r["drain_s"]), 1.0)

    def test_interrupted_run_stops_its_programs(self) -> None:
        # M3F-12: Ctrl-C (or SIGTERM, SIGHUP) in the middle of a run raises
        # KeyboardInterrupt; the programs started so far must not outlive the run.
        program = (f"{sys.executable} -c 'import os, signal, sys, time; "
                   f"open(sys.argv[1] + \".pid\", \"w\").write(str(os.getpid())); "
                   f"signal.signal(signal.SIGINT, lambda *a: sys.exit(0)); time.sleep(60)' {{config}}")

        class Interrupting(FakeRedis):
            polls = 0

            def pipeline(self, cmds: list[list[object]]) -> list[object]:
                if cmds and cmds[0][0] == "LLEN":
                    self.polls += 1
                    if self.polls == 20:
                        raise KeyboardInterrupt
                return super().pipeline(cmds)

        run = rmbench.expand(doc(BASE + '[[scenario]]\nname = "t"\nduration = 5\ncount = 900000\n'))[0]
        with tempfile.TemporaryDirectory() as d:
            args = argparse.Namespace(
                label="x", results=d, renderer=rmbench.DEFAULT_RENDERER, config_ext=".yaml",
                redis="redis://127.0.0.1:6379", cmd=program, settle=2.0, timeout=10.0,
                keep_keys=False, duration=None)
            with self.assertRaises(KeyboardInterrupt):
                rmbench.run_once(run, 0, args, Interrupting(step=100), {"redis_version": "7", "host_cpus": 2})
            pid_files = list(Path(d).glob("logs/*/i0.config.yaml.pid"))
            self.assertEqual(len(pid_files), 1)
            pid = int(pid_files[0].read_text())
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        os.kill(pid, signal.SIGKILL)  # do not leak it when the test fails
        self.fail(f"program {pid} still running after the interrupted run")

    def test_sigterm_and_sighup_interrupt_like_ctrl_c(self) -> None:
        saved = {s: signal.getsignal(s) for s in (signal.SIGTERM, signal.SIGHUP)}
        try:
            rmbench.interrupt_on_termination()
            for sig in saved:
                self.assertIs(signal.getsignal(sig), signal.default_int_handler)
        finally:
            for sig, handler in saved.items():
                signal.signal(sig, handler)

    def test_main_installs_the_handlers_before_the_command(self) -> None:
        # M3R-18 b: the function alone is tested above; this pins that `main` calls it
        saved = {s: signal.getsignal(s) for s in (signal.SIGTERM, signal.SIGHUP)}
        seen: dict[int, object] = {}

        def command(_args: argparse.Namespace) -> int:
            seen.update({s: signal.getsignal(s) for s in saved})
            return 0

        fake = mock.Mock()
        fake.return_value.parse_args.return_value = argparse.Namespace(func=command)
        try:
            for sig in saved:
                signal.signal(sig, signal.SIG_DFL)
            with mock.patch.object(rmbench, "parser", fake):
                self.assertEqual(rmbench.main([]), 0)
            for sig in saved:
                self.assertIs(seen[sig], signal.default_int_handler)
        finally:
            for sig, handler in saved.items():
                signal.signal(sig, handler)


class CommandTests(unittest.TestCase):
    """`run` and `compare` end to end, with the fake Redis of RunOnceTests (M3R-5)."""

    def run_cmd(self, fake: FakeRedis, d: str) -> tuple[int, str]:
        scen = Path(d) / "s.toml"
        scen.write_text(BASE + '[[scenario]]\nname = "t"\nduration = 0.3\ncount = 900000\n')
        args = rmbench.parser().parse_args(
            ["run", "--scenarios", str(scen), "--cmd", RunOnceTests.PROGRAM, "--label", "x",
             "--results", str(Path(d) / "res"), "--redis", "redis://127.0.0.1:6379"])
        err, out = io.StringIO(), io.StringIO()
        with mock.patch.object(rmbench, "Redis", lambda url: fake), \
                contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
            code = rmbench.cmd_run(args)
        return code, err.getvalue()

    def test_run_exits_0_without_losses(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            code, err = self.run_cmd(FakeRedis(step=100), d)
        self.assertEqual(code, 0)
        self.assertNotIn("MISSING:", err)

    def test_run_reports_and_exits_3_on_losses(self) -> None:
        with tempfile.TemporaryDirectory() as d:
            code, err = self.run_cmd(FakeRedis(step=100, lose=True), d)
        self.assertEqual(code, rmbench.EXIT_MISSING)
        self.assertRegex(err, r"MISSING: t r0 missing=\d+")

    def compare(self, rows_a: list[dict], rows_b: list[dict]) -> tuple[str, str]:
        with tempfile.TemporaryDirectory() as d:
            a, b = Path(d) / "a.csv", Path(d) / "b.csv"
            rmbench.append_csv(a, rows_a)
            rmbench.append_csv(b, rows_b)
            args = rmbench.parser().parse_args(["compare", str(a), str(b)])
            err, out = io.StringIO(), io.StringIO()
            with contextlib.redirect_stderr(err), contextlib.redirect_stdout(out):
                self.assertEqual(rmbench.cmd_compare(args), 0)
        return out.getvalue(), err.getvalue()

    def row(self, **kw: object) -> dict:
        return dict.fromkeys(rmbench.CSV_FIELDS, "") | {
            "label": "x", "scenario": "s", "status": "ok", "msg_s": "100", "missing": "0",
            "count": "900", "duration_s": "5"} | kw

    def test_compare_warns_on_different_counts(self) -> None:
        out, err = self.compare([self.row()], [self.row(label="y", count="27000")])
        self.assertIn("warning: not comparable, s: count 900 vs 27000", err)
        self.assertIn("| s |", out)
        _, err = self.compare([self.row()], [self.row(label="y")])
        self.assertEqual(err, "")

    def test_compare_worst_missing_includes_failed_runs(self) -> None:
        # A lossy repetition that also failed must still show in the missing column
        out, _ = self.compare([self.row(), self.row(status="error", missing="5", repeat_idx=1)],
                              [self.row(label="y")])
        self.assertIn("| 5 | 0 |", out)


if __name__ == "__main__":
    unittest.main()
