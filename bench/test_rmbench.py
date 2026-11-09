"""Unit tests for the bench harness (no Redis needed).

Run: python3 -m unittest discover -s bench -p 'test_*.py'
"""

from __future__ import annotations

import json
import re
import socket
import sys
import tempfile
import threading
import tomllib
import unittest
from pathlib import Path

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
        self.assertEqual(tags, {"smoke", "quick", "full"})

    def test_expected_counts(self) -> None:
        inst = {"count": 100, "mode": "replicant", "clients": 3, "filter": "client"}
        self.assertEqual(rmbench.expected_per_target(inst), [50, 100, 100])
        inst = {"count": 99, "mode": "spreader", "clients": 3, "filter": "off"}
        self.assertIsNone(rmbench.expected_per_target(inst))
        self.assertEqual(rmbench.expected_total(inst), 99)
        inst = {"count": 100, "mode": "replicant", "clients": 2, "filter": "root"}
        self.assertEqual(rmbench.expected_total(inst), 100)


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


if __name__ == "__main__":
    unittest.main()
