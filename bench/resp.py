"""Minimal RESP2 client on the Python standard library.

Only what the bench needs: one connection, commands, pipelines, AUTH, SELECT
and TLS (rediss://). Replies: bytes for bulk strings, str for status
replies, int, list, None; error replies raise RedisError.
"""

from __future__ import annotations

import socket
import ssl
from typing import Any, Iterable, Sequence
from urllib.parse import unquote, urlsplit


class RedisError(Exception):
    """Error reply from the server or protocol failure."""


Arg = bytes | str | int


def encode(args: Sequence[Arg]) -> bytes:
    """Encode one command as a RESP array of bulk strings."""
    out = [b"*%d\r\n" % len(args)]
    for a in args:
        if isinstance(a, bytes):
            b = a
        elif isinstance(a, str):
            b = a.encode()
        elif isinstance(a, int):
            b = str(a).encode()
        else:
            raise TypeError(f"unsupported argument type: {type(a).__name__}")
        out.append(b"$%d\r\n" % len(b))
        out.append(b)
        out.append(b"\r\n")
    return b"".join(out)


def parse_url(url: str) -> dict[str, Any]:
    """Split a redis:// or rediss:// URL into connection fields."""
    u = urlsplit(url)
    if u.scheme not in ("redis", "rediss"):
        raise ValueError(f"URL must start with redis:// or rediss://: {url}")
    db_text = u.path.strip("/")
    return {
        "host": u.hostname or "127.0.0.1",
        "port": u.port or 6379,
        "username": unquote(u.username) if u.username else None,
        "password": unquote(u.password) if u.password else "",
        "ssl": u.scheme == "rediss",
        "db": int(db_text) if db_text else 0,
    }


class Redis:
    def __init__(self, url: str, timeout: float = 5.0) -> None:
        self.info = parse_url(url)
        sock = socket.create_connection((self.info["host"], self.info["port"]), timeout=timeout)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        if self.info["ssl"]:
            ctx = ssl.create_default_context()
            sock = ctx.wrap_socket(sock, server_hostname=self.info["host"])
        self.sock = sock
        self.reader = sock.makefile("rb")
        if self.info["password"]:
            if self.info["username"]:
                self.execute("AUTH", self.info["username"], self.info["password"])
            else:
                self.execute("AUTH", self.info["password"])
        if self.info["db"]:
            self.execute("SELECT", self.info["db"])

    def close(self) -> None:
        try:
            self.reader.close()
        finally:
            self.sock.close()

    def __enter__(self) -> "Redis":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def execute(self, *args: Arg) -> Any:
        self.sock.sendall(encode(args))
        return self._read()

    def pipeline(self, commands: Iterable[Sequence[Arg]]) -> list[Any]:
        """Send all commands at once, then read every reply in order.

        Error replies are returned as RedisError instances, not raised, so one
        failing command does not desynchronise the stream.
        """
        cmds = list(commands)
        self.sock.sendall(b"".join(encode(c) for c in cmds))
        out: list[Any] = []
        for _ in cmds:
            try:
                out.append(self._read())
            except RedisError as e:
                out.append(e)
        return out

    def _line(self) -> bytes:
        line = self.reader.readline()
        if not line.endswith(b"\r\n"):
            raise RedisError("connection closed by server")
        return line[:-2]

    def _read(self) -> Any:
        line = self._line()
        kind, rest = line[:1], line[1:]
        if kind == b"+":
            return rest.decode()
        if kind == b"-":
            raise RedisError(rest.decode(errors="replace"))
        if kind == b":":
            return int(rest)
        if kind == b"$":
            n = int(rest)
            if n < 0:
                return None
            data = self.reader.read(n + 2)
            if len(data) != n + 2:
                raise RedisError("connection closed by server")
            return data[:-2]
        if kind == b"*":
            n = int(rest)
            if n < 0:
                return None
            items: list[Any] = []
            for _ in range(n):
                try:
                    items.append(self._read())
                except RedisError as e:
                    items.append(e)
            return items
        raise RedisError(f"unexpected reply type: {line[:20]!r}")
