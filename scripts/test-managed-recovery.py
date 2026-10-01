#!/usr/bin/env python3
"""Isolated HTTPS/SSE and diagnostic-feedback recovery with a real Mihomo core."""
import argparse
import base64
import http.server
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import socket
import socketserver
import sqlite3
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request


class Target(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def do_HEAD(self):
        time.sleep(.02)
        host = self.headers.get('Host', '')
        code = 405 if 'chatgpt' in host else 401 if 'openai' in host else 204 if 'gstatic' in host else 200
        if 'chatgpt' in host and not self.server.codex_ok:
            code = 503
        self.send_response(code)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'text/event-stream')
        self.send_header('Connection', 'close')
        self.end_headers()
        try:
            while not self.server.stopping:
                if not self.server.stall:
                    self.wfile.write(b'data: heartbeat\n\n')
                    self.wfile.flush()
                time.sleep(.1)
        except (OSError, ssl.SSLError):
            pass

    def log_message(self, *_):
        pass


class Tunnel(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(180)
        request = self.rfile.readline(16384)
        if not request.startswith(b'CONNECT '):
            return
        while self.rfile.readline(16384) not in (b'\r\n', b'\n', b''):
            pass
        self.wfile.write(b'HTTP/1.1 200 Connection established\r\n\r\n')
        self.wfile.flush()
        if b'echo.test:' in request:
            while True:
                data = self.connection.recv(1024)
                if not data:
                    return
                self.connection.sendall(data)
        else:
            with socket.create_connection(self.server.target, timeout=3) as remote:
                while True:
                    readable, _, _ = select.select([remote, self.connection], [], [], 20)
                    if not readable:
                        continue
                    for src in readable:
                        data = src.recv(16384)
                        if not data:
                            return
                        (remote if src is self.connection else self.connection).sendall(data)


class QuietHTTP(http.server.ThreadingHTTPServer):
    def handle_error(self, *_):
        pass


class Proxy(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, *_):
        pass


def wait_for(predicate, timeout=80):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(.2)
    raise AssertionError('isolated recovery condition timed out')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--binary', type=Path, required=True)
    ap.add_argument('--mihomo', type=Path, required=True)
    ap.add_argument('--root', type=Path, required=True)
    ap.add_argument('--legacy-binary', type=Path)
    ap.add_argument('--scenario', choices=['probe','stream','legacy'])
    args = ap.parse_args()
    if args.scenario == 'legacy' and not args.legacy_binary:
        ap.error('--scenario legacy requires --legacy-binary')
    args.root.mkdir(parents=True, exist_ok=True)
    os.umask(0o077)
    base = Path(tempfile.mkdtemp(prefix='managed-recovery-', dir=args.root))
    def openssl(*argv):
        subprocess.run(['openssl', *argv], cwd=base, check=True, capture_output=True)
    openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'ca.key', '-out', 'ca.crt', '-days', '2', '-subj', '/CN=MihoTerm recovery test CA', '-addext', 'basicConstraints=critical,CA:TRUE', '-addext', 'keyUsage=critical,keyCertSign,cRLSign')
    openssl('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key', '-out', 'server.csr', '-subj', '/CN=chatgpt.com')
    (base/'ext').write_text('subjectAltName=DNS:chatgpt.com,DNS:api.openai.com,DNS:www.gstatic.com,DNS:github.com\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n')
    openssl('x509', '-req', '-in', 'server.csr', '-CA', 'ca.crt', '-CAkey', 'ca.key', '-CAcreateserial', '-out', 'server.crt', '-days', '2', '-extfile', 'ext')
    wrapper = base/'mihomo-fixture'
    wrapper.write_text('#!/bin/sh\nexport SSL_CERT_FILE='+shlex.quote(str(base/'ca.crt'))+'\nexec '+shlex.quote(str(args.mihomo.resolve()))+' "$@"\n')
    wrapper.chmod(0o700)
    servers = []
    targets = {}
    ports = {}
    for name in ['A', 'B', 'Outside']:
        tls = QuietHTTP(('127.0.0.1', 0), Target)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(base/'server.crt', base/'server.key')
        tls.socket = ctx.wrap_socket(tls.socket, server_side=True)
        tls.codex_ok, tls.stall, tls.stopping = True, False, False
        proxy = Proxy(('127.0.0.1', 0), Tunnel)
        proxy.target = tls.server_address
        ports[name], targets[name] = proxy.server_address[1], tls
        servers.extend([tls, proxy])
        for server in [tls, proxy]:
            threading.Thread(target=server.serve_forever, daemon=True).start()
    class Results(list):
        def append(self, value):
            super().append(value)
            print(value, flush=True)
    results = Results()
    try:
        for scenario in ([args.scenario] if args.scenario else ['probe', 'stream'] + (['legacy'] if args.legacy_binary else [])):
            case = base/scenario
            case.mkdir()
            config = case/'config.toml'
            config.write_text('')
            source = case/'subscription.json'
            document = {'proxies': [{'name': n, 'type': 'http', 'server': '127.0.0.1', 'port': p} for n,p in ports.items()],
                        'proxy-groups': [{'name': 'Research fixture', 'type': 'select', 'proxies': ['Nested', 'A']},
                                         {'name': 'Nested', 'type': 'select', 'proxies': ['B', 'A']},
                                         {'name': 'Unrelated', 'type': 'fallback', 'proxies': ['Outside'], 'url': 'https://www.gstatic.com/generate_204', 'interval': 30}],
                        'rules': ['MATCH,Research fixture']}
            source.write_text(json.dumps(document))
            dbpath = case/'logs.sqlite'
            db = sqlite3.connect(dbpath)
            db.execute('CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,target TEXT,level TEXT,feedback_log_body TEXT)')
            db.commit()
            env = {k:v for k,v in os.environ.items() if not k.startswith('MIHOTERM_') and k.lower() not in ('http_proxy','https_proxy','all_proxy')}
            env.update(TMPDIR=str(case), SSL_CERT_FILE=str(base/'ca.crt'))
            cmd = [str(args.binary.resolve()), '--state-dir', str(case/'state'), '--runtime-dir', str(case/'runtime'), '--config', str(config)]
            def run(*argv, check=True):
                r = subprocess.run(cmd+list(argv), env=env, capture_output=True, text=True, timeout=60)
                if check and r.returncode:
                    raise AssertionError('command failed: '+str(argv)+'\n'+r.stderr)
                return r
            def record():
                return json.loads((case/'runtime/session.json').read_text())
            op = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            def api(path):
                s = record()
                req = urllib.request.Request(s['controller_url']+path, headers={'Authorization': 'Bearer '+s['controller_secret']})
                with op.open(req, timeout=8) as response:
                    return json.load(response)
            def selected():
                return api('/proxies/'+urllib.parse.quote('Research fixture',safe=''))['now']
            def state():
                try:
                    return json.loads((case/'state/recovery.json').read_text())
                except FileNotFoundError:
                    return {}
            def connect(host):
                s = record()
                sock = socket.create_connection(('127.0.0.1',s['mixed_port']),timeout=5)
                token = base64.b64encode((s['proxy_username']+':'+s['proxy_password']).encode())
                sock.sendall(b'CONNECT '+host.encode()+b':443 HTTP/1.1\r\nHost: '+host.encode()+b'\r\nProxy-Authorization: Basic '+token+b'\r\n\r\n')
                assert b' 200 ' in sock.recv(4096)
                return sock
            def signal(body):
                db.execute('INSERT INTO logs(ts,target,level,feedback_log_body) VALUES(?,?,?,?)',
                           (int(time.time()),'codex_core::responses_retry','WARN',body))
                db.commit()
            supervisor = None
            watcher = None
            held = None
            stream_socket = None
            try:
                run('profile','add','fixture','--file',str(source))
                if scenario == 'legacy':
                    subprocess.run([str(args.legacy_binary.resolve())]+cmd[1:]+['start','fixture','--mihomo',str(wrapper)],env=env,check=True,capture_output=True,timeout=60)
                else:
                    supervisor=subprocess.Popen(cmd+['supervise','fixture','--mihomo',str(wrapper)],env=env,stdout=subprocess.DEVNULL,stderr=(case/'supervisor.log').open('w'))
                    wait_for(lambda: (case/'runtime/session.json').exists())
                original = record()
                run('profile','policy','fixture','--group','Research fixture','--codex-log-db',str(dbpath),'--apply')
                assert selected() == 'B', 'activation must retain the previously selected nested leaf'
                assert 'Research fixture Auto' in api('/proxies/'+urllib.parse.quote('Research fixture',safe=''))['all']
                assert api('/proxies/GLOBAL')['now'] == 'Research fixture'
                run('select','--group','Research fixture','--proxy','A')
                if scenario == 'legacy':
                    watcher = subprocess.Popen(cmd+['watch'],env=env,stdout=subprocess.DEVNULL,stderr=(case/'watch.log').open('w'))
                held = connect('echo.test')
                def echo():
                    held.sendall(b'ping'); assert held.recv(4)==b'ping'
                echo()
                wait_for(lambda: state().get('active_node')=='A' and state().get('feedback_status')=='watching')
                if scenario in ('probe','legacy'):
                    targets['A'].codex_ok = False
                    # Google succeeds on A while the Codex endpoint fails.
                    google = run('probe','--proxy','A','--target','Google')
                    assert google.returncode == 0
                    wait_for(lambda: selected()=='B' and state().get('active_node')=='B')
                    echo()
                    assert any(e['reason']=='confirmed-probe-failover' for e in state()['events'])
                    results.append('Google healthy / Codex failed: automatic in-scope recovery; old stream survives: PASS')
                    targets['B'].codex_ok = False
                    run('doctor','--repair',check=False)
                    assert selected()=='B' and state()['status']=='no-healthy-in-scope-candidate'
                    results.append('all allowed nodes failed: explicit degraded state; healthy unrelated node never selected: PASS')
                    targets['A'].codex_ok = targets['B'].codex_ok = True
                    run('profile','update','fixture','--apply'); echo()
                    assert selected()=='B'
                    desc = (case/'state/profiles/fixture/source.toml').read_text()
                    assert 'managed = true' in desc and 'codex_log_db' in desc
                    assert record()['pid']==original['pid']
                    if scenario == 'legacy':
                        desired=json.loads((case/'state/desired.json').read_text())
                        assert int(time.time())-desired['last_recovery_unix_seconds']<60
                        watcher.terminate(); watcher.wait(timeout=10); watcher=None
                        assert record()['pid']==original['pid']
                        echo()
                        results.append('released legacy supervisor plus new watcher: hot policy, lease, recovery and watcher stop preserve the core and stream: PASS')
                        continue
                    old_endpoint = {k:original[k] for k in ['mixed_port','proxy_username','proxy_password','controller_url']}
                    run('stop'); held.close(); held=None
                    run('start','fixture','--mihomo',str(wrapper))
                    assert {k:record()[k] for k in old_endpoint}==old_endpoint
                    assert selected()=='B' and state()['switched_at']>0
                    results.append('refresh and restart retain policy, selection, cooldown and endpoint credentials: PASS')
                else:
                    signal('HTTP 429 idle timeout waiting for SSE')
                    signal('HTTP 401 error decoding response body')
                    before = state()['checked_at']
                    wait_for(lambda: state().get('checked_at',0)>before)
                    assert selected()=='A'
                    results.append('auth and rate-limit feedback does not rotate nodes: PASS')
                    stream_socket = ssl.create_default_context(cafile=str(base/'ca.crt')).wrap_socket(connect('chatgpt.com'),server_hostname='chatgpt.com')
                    stream_socket.sendall(b'GET /events HTTP/1.1\r\nHost: chatgpt.com\r\n\r\n')
                    data=b''
                    while b'data: heartbeat' not in data: data+=stream_socket.recv(4096)
                    targets['A'].stall=True
                    signal('stream disconnected before completion: idle timeout waiting for SSE')
                    signal('stream disconnected before completion: Transport error: network error: error decoding response body')
                    wait_for(lambda: selected()=='B' and any(e['reason']=='stream-feedback-trial-switch' for e in state().get('events',[])))
                    assert state()['active_node']=='B'
                    echo()
                    targets['A'].stall=False
                    assert b'data: heartbeat' in stream_socket.recv(4096)
                    results.append('stalled SSE with healthy HEAD: bounded feedback trial switches new requests without killing existing streams: PASS')
                    signal('Connection failed: error sending request'); signal('Connection failed: error sending request')
                    before=state()['checked_at']
                    wait_for(lambda: state().get('checked_at',0)>before)
                    assert selected()=='B' and state()['status']=='switch-cooldown'
                    results.append('repeated feedback respects cooldown and avoids immediate switch-back: PASS')
                    run('profile','policy','fixture','--disable','--apply')
                    assert 'managed = true' not in (case/'state/profiles/fixture/source.toml').read_text()
                    echo()
                    results.append('explicit policy disable restores subscription groups and preserves streams: PASS')
            finally:
                if watcher:
                    watcher.terminate(); watcher.wait(timeout=10)
                if stream_socket: stream_socket.close()
                if held: held.close()
                try:
                    shutil.copy2(Path(record()['runtime_dir'])/'mihomo.log',case/'core.log')
                except (OSError, KeyError):
                    pass
                run('stop',check=False)
                if supervisor: supervisor.wait(timeout=10)
                db.close()
                targets['A'].codex_ok = targets['B'].codex_ok = True
                targets['A'].stall=False
    finally:
        for server in servers:
            if hasattr(server,'stopping'): server.stopping=True
            server.shutdown(); server.server_close()
    print('Test-owned processes stopped; private evidence:',base)


if __name__ == '__main__':
    main()
