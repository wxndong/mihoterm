#!/usr/bin/env python3
"""Opt-in real-core regression. All listeners, profiles and processes are isolated."""
import argparse
import base64
import json
import os
from pathlib import Path
import pty
import signal
import socket
import socketserver
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request


class EchoProxy(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(10)
        line = self.rfile.readline(16384)
        if not line.startswith(b"CONNECT "):
            return
        while self.rfile.readline(16384) not in (b"\r\n", b"\n", b""):
            pass
        self.wfile.write(b"HTTP/1.1 200 Connection established\r\n\r\n")
        self.wfile.flush()
        while True:
            data = self.connection.recv(1024)
            if not data:
                break
            self.connection.sendall(data)


class QuietServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, request, client_address):
        pass


class DnsTcp(socketserver.BaseRequestHandler):
    queries = 0

    def handle(self):
        self.request.settimeout(5)
        head = self.request.recv(2)
        if len(head) != 2:
            return
        size = struct.unpack("!H", head)[0]
        data = b""
        while len(data) < size:
            data += self.request.recv(size - len(data))
        i = 12
        while data[i]:
            i += data[i] + 1
        i += 5
        question = data[12:i]
        type_ = struct.unpack("!H", question[-4:-2])[0]
        answer = (b"\xc0\x0c\x00\x01\x00\x01\x00\x00\x00\x01\x00\x04\x7f\x00\x00\x01"
                  if type_ == 1 else b"")
        response = data[:2] + b"\x81\x80\x00\x01" + struct.pack("!H", bool(answer)) + b"\x00\x00\x00\x00" + question + answer
        DnsTcp.queries += 1
        self.request.sendall(struct.pack("!H", len(response)) + response)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--mihomo", required=True, type=Path)
    parser.add_argument("--root", required=True, type=Path)
    args = parser.parse_args()
    args.root.mkdir(parents=True, exist_ok=True)
    base = Path(tempfile.mkdtemp(prefix="live-resilience-", dir=args.root))
    os.chmod(base, 0o700)
    env = {k: v for k, v in os.environ.items()
           if not k.startswith("MIHOTERM_") and k.lower() not in ("http_proxy", "https_proxy", "all_proxy")}
    env.update(TMPDIR=str(base), XDG_STATE_HOME=str(base / "xdg-state"),
               XDG_CONFIG_HOME=str(base / "xdg-config"), XDG_CACHE_HOME=str(base / "cache"))
    config = base / "config.toml"
    config.write_text("")
    config.chmod(0o600)
    cmd = [str(args.binary.resolve()), "--state-dir", str(base / "state"),
           "--runtime-dir", str(base / "runtime"), "--config", str(config)]
    def run(*argv, check=True):
        p = subprocess.run(cmd + list(argv), env=env, capture_output=True, text=True, timeout=40)
        if check and p.returncode:
            raise AssertionError("command failed: " + argv[0] + ": " + p.stderr)
        return p
    def record():
        return json.loads((base / "runtime/session.json").read_text())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def api(path, method="GET", data=None):
        s = record()
        request = urllib.request.Request(s["controller_url"] + path, method=method,
                headers={"Authorization": "Bearer " + s["controller_secret"], "Content-Type": "application/json"},
                data=None if data is None else json.dumps(data).encode())
        with opener.open(request, timeout=5) as response:
            raw = response.read()
        return json.loads(raw) if raw else None
    servers = [QuietServer(("127.0.0.1", 0), EchoProxy) for _ in range(2)]
    dns = QuietServer(("127.0.0.1", 0), DnsTcp)
    blackhole = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    blackhole.bind(("127.0.0.1", 0))
    for server in servers + [dns]:
        threading.Thread(target=server.serve_forever, daemon=True).start()
    profile = {
        "dns": {"enable": True, "ipv6": False,
                "nameserver": ["tcp://127.0.0.1:" + str(dns.server_address[1])],
                "proxy-server-nameserver": [
                    "udp://127.0.0.1:" + str(blackhole.getsockname()[1]),
                    "tcp://127.0.0.1:" + str(dns.server_address[1])]},
        "proxies": [{"name": name, "type": "http", "server": "edge.test", "port": server.server_address[1]}
                    for name, server in zip(("A", "B"), servers)],
        "proxy-groups": [{"name": "Target", "type": "select", "proxies": ["A", "B"]},
                         {"name": "Other", "type": "select", "proxies": ["A"]}],
        "rules": ["DOMAIN,target.test,Target", "MATCH,Other"],
    }
    source = base / "profile.json"
    source.write_text(json.dumps(profile))
    source.chmod(0o600)
    streams = []
    def connect(host, session=None):
        s = record() if session is None else session
        token = base64.b64encode((s["proxy_username"] + ":" + s["proxy_password"]).encode())
        sock = socket.create_connection(("127.0.0.1", s["mixed_port"]), timeout=5)
        sock.sendall(b"CONNECT " + host.encode() + b":443 HTTP/1.1\r\nHost: " + host.encode()
                     + b":443\r\nProxy-Authorization: Basic " + token + b"\r\n\r\n")
        header = b""
        while b"\r\n\r\n" not in header:
            data = sock.recv(4096)
            if not data:
                raise AssertionError("proxy closed CONNECT")
            header += data
        assert b" 200 " in header, "authenticated CONNECT failed"
        streams.append(sock)
        sock.sendall(b"ping")
        assert sock.recv(4) == b"ping"
        return sock
    def echo(sock):
        sock.sendall(b"pong")
        assert sock.recv(4) == b"pong"
    results = []
    try:
        run("profile", "add", "fixture", "--file", str(source))
        run("start", "fixture", "--mihomo", str(args.mihomo.resolve()))
        initial = record()
        master, slave = pty.openpty()
        tui_env = dict(env, TERM="xterm-256color")
        tui = subprocess.Popen(cmd + ["run", "fixture", "--mihomo", str(args.mihomo.resolve())],
                               stdin=slave, stdout=slave, stderr=slave, env=tui_env)
        os.close(slave)
        try:
            time.sleep(.8)
            os.write(master, b"q")
            assert tui.wait(timeout=8) == 0
        finally:
            if tui.poll() is None:
                tui.terminate()
                tui.wait(timeout=5)
            os.close(master)
        assert record()["pid"] == initial["pid"]
        results.append("q exits the TUI while the proxy remains alive: PASS")
        assert api("/configs")["tcp-concurrent"] is True
        api("/configs", "PATCH", {"mode": "rule"})
        t = time.monotonic()
        old = connect("target.test")
        other = connect("other.test")
        assert time.monotonic() - t < 4 and DnsTcp.queries > 0
        results.append("DNS UDP blackhole with TCP alternative: PASS")
        failed = run("select", "--group", "Target", "--proxy", "Missing", "--reconnect", check=False)
        assert failed.returncode != 0
        echo(old)
        echo(other)
        run("select", "--group", "Target", "--proxy", "B", "--reconnect")
        assert old.recv(1) == b""
        echo(other)
        new = connect("target.test")
        run("select", "--group", "Target", "--proxy", "B", "--reconnect")
        echo(new)
        echo(other)
        results.append("scoped reconnect, failed selection and no-op: PASS")
        run("profile", "update", "fixture", "--apply")
        echo(new)
        echo(other)
        assert record()["pid"] == initial["pid"]
        results.append("profile hot reload preserves established streams and PID: PASS")
        # Crash only this test's exact managed child.
        os.kill(record()["pid"], signal.SIGKILL)
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            time.sleep(.2)
            try:
                after = record()
                if after["pid"] != initial["pid"] and api("/version"):
                    break
            except (OSError, ValueError, urllib.error.URLError):
                pass
        else:
            raise AssertionError("supervised child did not recover")
        keys = ("session_id", "mixed_port", "controller_url", "controller_secret", "proxy_username", "proxy_password")
        assert all(after[k] == initial[k] for k in keys)
        connect("target.test", initial)
        results.append("child crash restores same authenticated endpoint: PASS")
        run("stop")
        assert not (base / "runtime/session.json").exists()
        assert (base / "state/endpoint.json").stat().st_mode & 0o077 == 0
        run("start", "fixture", "--mihomo", str(args.mihomo.resolve()))
        after = record()
        assert all(after[k] == initial[k] for k in keys)
        connect("target.test", initial)
        results.append("stop/start permits old inherited credentials to reconnect: PASS")
    finally:
        for sock in streams:
            sock.close()
        run("stop", check=False)
        for server in servers + [dns]:
            server.shutdown()
            server.server_close()
        blackhole.close()
    assert not (base / "runtime/session.json").exists()
    for result in results:
        print(result)
    print("test-owned processes stopped; private evidence:", base)


if __name__ == "__main__":
    main()
