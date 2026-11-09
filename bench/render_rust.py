#!/usr/bin/env python3
"""Renderer for redismultiplexer (Rust): bench spec JSON -> YAML configuration.

Usage: render_rust.py <spec.json> <out_config>
Exit 0 = written, 3 = scenario not supported, other = error.
See bench/README.md for the spec format.
"""

from __future__ import annotations

import json
import sys
from typing import Any

UNSUPPORTED = 3


def q(value: Any) -> str:
    """YAML scalar: single-quoted string, or a plain bool/int."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    return "'" + str(value).replace("'", "''") + "'"


def connection(redis: dict[str, Any], indent: str) -> list[str]:
    return [
        f"{indent}ssl: {q(bool(redis['ssl']))}",
        f"{indent}hostname: {q(redis['host'])}",
        f"{indent}port: {q(int(redis['port']))}",
        f"{indent}password: {q(redis['password'])}",
    ]


def filter_lines(flt: dict[str, Any] | None, indent: str) -> list[str]:
    if not flt:
        return []
    lines = [f"{indent}filter: {q(flt['regex'])}"]
    if flt.get("until") is not None:
        lines.append(f"{indent}filter_until: {q(flt['until'])}")
    if flt.get("limit") is not None:
        lines.append(f"{indent}filter_limit: {q(int(flt['limit']))}")
    if flt.get("replace") is not None:
        lines.append(f"{indent}filter_replace: {q(flt['replace'])}")
    return lines


def render(spec: dict[str, Any]) -> str | None:
    if spec.get("transport", "list") != "list":
        return None
    redis = spec["redis"]
    lines = [f"name: {q(spec['scenario'])}"]
    lines += connection(redis, "")
    lines += [
        f"channel: {q(spec['source']['key'])}",
        f"children: {q(int(spec['children']))}",
        f"mode: {q(spec['mode'])}",
        f"pid: {q(spec['pid_file'])}",
        f"status: {q(spec['status_file'])}",
    ]
    lines += filter_lines(spec.get("filter"), "")
    ordering = spec.get("ordering")
    if ordering:
        lines += [
            f"ordering: {q(ordering['regex'])}",
            f"ordering_buffer_time: {q(int(ordering['buffer_time']))}",
            f"ordering_limit: {q(int(ordering['limit']))}",
        ]
    lines.append("clients:")
    for c in spec["clients"]:
        lines.append(f"  - name: {q(c['name'])}")
        lines += connection(redis, "    ")
        lines.append(f"    channel: {q(c['key'])}")
        limits = c.get("limits")
        if limits:
            for k in ("timelimit", "checklimit", "softlimit", "hardlimit", "deleteblock"):
                if limits.get(k) is not None:
                    lines.append(f"    {k}: {q(int(limits[k]))}")
        lines += filter_lines(c.get("filter"), "    ")
    return "\n".join(lines) + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(f"usage: {argv[0]} <spec.json> <out_config>", file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as f:
        spec = json.load(f)
    text = render(spec)
    if text is None:
        print(f"unsupported transport: {spec.get('transport')}", file=sys.stderr)
        return UNSUPPORTED
    with open(argv[2], "w", encoding="utf-8") as f:
        f.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
