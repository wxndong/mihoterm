#!/usr/bin/env python3
"""Real HTTPS subscription refresh through an isolated authenticated Mihomo proxy."""
import argparse
import base64
import hashlib
import http.server
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import ssl
import subprocess
import tempfile
import threading
import time
import urllib.request


class Subscription(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.requests.append(self.headers.get('User-Agent'))
        self.send_response(self.server.status)
        self.send_header('Content-Length', str(len(self.server.body)))
        self.end_headers()
        self.wfile.write(self.server.body)

    def log_message(self, *_):
        pass


class Upstream(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(15)
        first = self.rfile.readline(16384)
        if not first.startswith(b'CONNECT '):
            return
        while self.rfile.readline(16384) not in (b'\r\n', b'\n', b''):
            pass
        self.wfile.write(b'HTTP/1.1 200 Connection established\r\n\r\n')
        self.wfile.flush()
        if b'127.0.0.1:' in first:
            with socket.create_connection(self.server.tls_address, timeout=3) as remote:
                while True:
                    readable, _, _ = select.select([self.connection, remote], [], [], 15)
                    if not readable:
                        return
                    for src in readable:
                        data = src.recv(16384)
                        if not data:
                            return
                        (remote if src is self.connection else self.connection).sendall(data)
        else:
            while True:
                data = self.connection.recv(16384)
                if not data:
                    return
                self.connection.sendall(data)


class ProxyServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def handle_error(self, *_):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    parser.add_argument('--mihomo', required=True, type=Path)
    parser.add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    base = Path(tempfile.mkdtemp(prefix='subscription-test-', dir=args.root))
    def openssl(*argv):
        subprocess.run(['openssl', *argv], cwd=base, check=True, capture_output=True, timeout=15)
    openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'ca.key', '-out', 'ca.crt', '-days', '2', '-subj', '/CN=MihoTerm test CA')
    openssl('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key', '-out', 'server.csr', '-subj', '/CN=127.0.0.1')
    (base/'server.ext').write_text('subjectAltName=IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n')
    openssl('x509', '-req', '-in', 'server.csr', '-CA', 'ca.crt', '-CAkey', 'ca.key', '-CAcreateserial', '-out', 'server.crt', '-days', '2', '-extfile', 'server.ext')
    tls = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Subscription)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(base/'server.crt', base/'server.key')
    tls.socket = context.wrap_socket(tls.socket, server_side=True)
    tls.status, tls.requests = 200, []
    upstream = ProxyServer(('127.0.0.1', 0), Upstream)
    upstream.tls_address = tls.server_address
    for server in (tls, upstream):
        threading.Thread(target=server.serve_forever, daemon=True).start()
    def document(second):
        return json.dumps({'proxies': [{'name': name, 'type': 'http', 'server': '127.0.0.1', 'port': upstream.server_address[1]} for name in ['A', second]], 'proxy-groups': [{'name': 'AI', 'type': 'select', 'proxies': ['A', second]}], 'rules': ['MATCH,AI']}).encode()
    tls.body = document('B')
    source = base/'cached.json'; source.write_bytes(tls.body)
    config = base/'config.toml'; config.write_text('')
    reserved = socket.socket(); reserved.bind(('127.0.0.1', 0))
    # Direct connects are refused. The fake upstream maps the tunnel to TLS.
    url_file = base/'subscription.url'
    url_file.write_text(f'https://127.0.0.1:{reserved.getsockname()[1]}/subscription\n')
    env = {k:v for k,v in os.environ.items() if not k.startswith('MIHOTERM_') and k.lower() not in ('http_proxy', 'https_proxy', 'all_proxy')}
    env.update(TMPDIR=str(base), XDG_CACHE_HOME=str(base/'cache'), SSL_CERT_FILE=str(base/'ca.crt'), HTTPS_PROXY='http://127.0.0.1:1')
    cmd = [str(args.binary.resolve()), '--state-dir', str(base/'state'), '--runtime-dir', str(base/'runtime'), '--config', str(config)]
    def run(*argv, check=True):
        result = subprocess.run(cmd+list(argv), env=env, capture_output=True, text=True, timeout=45)
        if check and result.returncode:
            raise AssertionError(result.stderr)
        return result
    def record():
        return json.loads((base/'runtime/session.json').read_text())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def api(path, method='GET', data=None):
        s = record()
        req = urllib.request.Request(s['controller_url']+path, method=method, headers={'Authorization':'Bearer '+s['controller_secret'], 'Content-Type':'application/json'}, data=None if data is None else json.dumps(data).encode())
        with opener.open(req, timeout=3) as response:
            body = response.read()
        return json.loads(body) if body else None
    stream = None
    results = []
    try:
        run('profile', 'add', 'fixture', '--file', str(source))
        run('start', 'fixture', '--mihomo', str(args.mihomo.resolve()))
        original = record(); api('/configs', 'PATCH', {'mode':'rule'})
        stream = socket.create_connection(('127.0.0.1', original['mixed_port']), timeout=4)
        token = base64.b64encode((original['proxy_username']+':'+original['proxy_password']).encode())
        stream.sendall(b'CONNECT echo.test:443 HTTP/1.1\r\nHost: echo.test:443\r\nProxy-Authorization: Basic '+token+b'\r\n\r\n')
        assert b' 200 ' in stream.recv(4096)
        def echo():
            stream.sendall(b'ping'); assert stream.recv(4)==b'ping'
        echo()
        run('profile', 'source', 'fixture', '--url-file', str(url_file), '--fallback-group', 'AI', '--preferred-proxy', 'B', '--apply')
        echo()
        assert api('/proxies/AI%20Auto')['all']==['B', 'A']
        results.append('HTTPS direct failure falls back to the authenticated managed proxy: PASS')
        tls.body = document('New')
        run('profile', 'update', 'fixture', '--apply'); echo()
        assert api('/proxies/AI%20Auto')['all']==['A', 'New']
        assert len(tls.requests)==2 and all(ua=='clash.meta' for ua in tls.requests)
        assert record()['pid']==original['pid']
        results.append('remote node changes regenerate fallback and preserve live stream: PASS')
        path = base/'state/profiles/fixture/profile.yaml'
        before = hashlib.sha256(path.read_bytes()).hexdigest()
        for status, body in [(200, b'not: [valid'), (503, b'unavailable')]:
            tls.status, tls.body = status, body
            assert run('profile', 'update', 'fixture', '--apply', check=False).returncode!=0
            assert hashlib.sha256(path.read_bytes()).hexdigest()==before
            assert record()['pid']==original['pid']; echo()
        results.append('invalid or unavailable subscription retains last good profile and stream: PASS')
        tls.status, tls.body = 200, document('Untrusted')
        env['SSL_CERT_FILE'] = str(base/'missing-ca.pem')
        assert run('profile', 'update', 'fixture', '--apply', check=False).returncode!=0
        assert hashlib.sha256(path.read_bytes()).hexdigest()==before; echo()
        results.append('untrusted TLS remains rejected with the proxy fallback: PASS')
    finally:
        if stream is not None: stream.close()
        for _ in range(20):
            result = run('stop', check=False)
            if result.returncode==0: break
            time.sleep(.2)
        for server in (tls, upstream): server.shutdown(); server.server_close()
        reserved.close()
    assert not (base/'runtime/session.json').exists()
    print('\n'.join(results))
    print('Test-owned processes stopped; private evidence:', base)


if __name__ == '__main__':
    main()
