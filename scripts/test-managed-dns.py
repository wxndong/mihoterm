#!/usr/bin/env python3
"""Verify provider DNS and listener adaptation with isolated, real Mihomo."""
import argparse
import base64
import copy
import json
import os
from pathlib import Path
import socket
import socketserver
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request


class DNS(socketserver.BaseRequestHandler):
    def handle(self):
        data, transport = self.request
        position, labels = 12, []
        while data[position]:
            length = data[position]
            labels.append(data[position + 1:position + 1 + length].decode())
            position += length + 1
        position += 1
        kind = struct.unpack('!H', data[position:position + 2])[0]
        name = '.'.join(labels)
        self.server.queries.append(name)
        answer = kind == 1
        response = data[:2] + struct.pack('!HHHHH', 0x8180, 1, int(answer), 0, 0)
        response += data[12:position + 4]
        if answer:
            response += b'\xc0\x0c' + struct.pack('!HHIH', 1, 1, 5, 4)
            response += socket.inet_aton(self.server.answer)
        transport.sendto(response, self.client_address)


class DatagramServer(socketserver.ThreadingUDPServer):
    daemon_threads = True


class EchoProxy(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(15)
        request = self.rfile.readline(16384)
        if not request.startswith(b'CONNECT '):
            return
        while self.rfile.readline(16384) not in (b'\r\n', b'\n', b''):
            pass
        self.server.connects += 1
        self.wfile.write(b'HTTP/1.1 200 Connection established\r\n\r\n')
        self.wfile.flush()
        while True:
            data = self.connection.recv(4096)
            if not data:
                return
            self.connection.sendall(data)


class StreamServer(socketserver.ThreadingTCPServer):
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
    base = Path(tempfile.mkdtemp(prefix='managed-dns-test-', dir=args.root))
    good = DatagramServer(('127.0.0.1', 0), DNS)
    bad = DatagramServer(('127.0.0.1', 0), DNS)
    good.answer, good.queries = '127.0.0.1', []
    bad.answer, bad.queries = '127.0.0.2', []
    upstream = StreamServer(('127.0.0.1', 0), EchoProxy)
    upstream.connects = 0
    for server in (good, bad, upstream):
        threading.Thread(target=server.serve_forever, daemon=True).start()
    reservation = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    reservation.bind(('127.0.0.1', 0))
    removed_port = reservation.getsockname()[1]
    reservation.close()
    good_dns = 'udp://127.0.0.1:' + str(good.server_address[1])
    bad_dns = 'udp://127.0.0.1:' + str(bad.server_address[1])
    self_dns = 'udp://127.0.0.1:' + str(removed_port)
    config = base / 'config.toml'
    config.write_text('')
    env = {k: v for k, v in os.environ.items()
           if not k.startswith('MIHOTERM_')
           and k.lower() not in ('http_proxy', 'https_proxy', 'all_proxy')}
    env.update(TMPDIR=str(base), XDG_CACHE_HOME=str(base / 'cache'))
    command = [str(args.binary.resolve()), '--config', str(config),
               '--state-dir', str(base / 'state'), '--runtime-dir', str(base / 'runtime')]

    def run(*argv, check=True):
        result = subprocess.run(command + list(argv), env=env, capture_output=True,
                                text=True, timeout=45)
        if check and result.returncode:
            raise AssertionError(result.stderr)
        return result

    def record():
        return json.loads((base / 'runtime/session.json').read_text())

    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(path, method='GET', data=None):
        current = record()
        request = urllib.request.Request(current['controller_url'] + path, method=method,
            headers={'Authorization': 'Bearer ' + current['controller_secret'],
                     'Content-Type': 'application/json'},
            data=None if data is None else json.dumps(data).encode())
        with opener.open(request, timeout=4) as response:
            body = response.read()
        return json.loads(body) if body else None

    def document(index):
        return {'ipv6': False, 'unified-delay': True,
            'dns': {'enable': True, 'ipv6': False,
                    'listen': '127.0.0.1:' + str(removed_port),
                    'nameserver': [bad_dns], 'default-nameserver': [bad_dns],
                    'nameserver-policy': {'+.nodes.test': [good_dns]}},
            'proxies': [{'name': 'Fixture', 'type': 'http',
                         'server': 'proxy' + str(index) + '.nodes.test',
                         'port': upstream.server_address[1]}],
            'proxy-groups': [{'name': 'AI', 'type': 'select', 'proxies': ['Fixture']}],
            'rules': ['MATCH,AI']}

    def source(name, data):
        path = base / (name + '.json')
        path.write_text(json.dumps(data))
        return str(path)

    def connect(auth=True):
        current = record()
        stream = socket.create_connection(('127.0.0.1', current['mixed_port']), timeout=4)
        credentials = base64.b64encode((current['proxy_username'] + ':' + current['proxy_password']).encode())
        headers = b'Proxy-Authorization: Basic ' + credentials + b'\r\n' if auth else b''
        stream.sendall(b'CONNECT echo.test:443 HTTP/1.1\r\nHost: echo.test:443\r\n' + headers + b'\r\n')
        response = b''
        while b'\r\n\r\n' not in response:
            chunk = stream.recv(4096)
            assert chunk, 'proxy closed during CONNECT'
            response += chunk
        assert (b' 200 ' if auth else b' 407 ') in response, response
        return stream

    def echo(stream):
        stream.sendall(b'dns-fixture-ping')
        received = b''
        while len(received) < 16:
            chunk = stream.recv(16 - len(received))
            assert chunk, 'established stream was closed'
            received += chunk
        assert received == b'dns-fixture-ping'

    held = None
    results = []
    started = time.monotonic()
    try:
        run('profile', 'add', 'fixture', '--file', source('initial', document(1)))
        run('start', 'fixture', '--mihomo', str(args.mihomo.resolve()))
        original = record()
        api('/configs', 'PATCH', {'mode': 'rule'})
        held = connect()
        echo(held)
        results.append('provider split-DNS works without synthetic system/node resolver overrides')

        second = document(2)
        second['dns']['proxy-server-nameserver'] = [self_dns]
        run('profile', 'source', 'fixture', '--file', source('self-reference', second), '--apply')
        echo(held)
        with connect() as fresh:
            echo(fresh)
        check_port = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        check_port.bind(('127.0.0.1', removed_port))
        check_port.close()
        results.append('self-reference inherits the complete provider policy without opening a DNS listener')

        third = document(3)
        third['dns']['proxy-server-nameserver'] = [good_dns]
        run('profile', 'source', 'fixture', '--file', source('independent-local-dns', third), '--apply')
        echo(held)
        with connect() as fresh:
            echo(fresh)
        results.append('independent local DNS service remains usable after hot reload')

        profile = base / 'state/profiles/fixture/profile.yaml'
        descriptor = base / 'state/profiles/fixture/source.toml'
        previous = profile.read_bytes(), descriptor.read_bytes()
        rejected = copy.deepcopy(third)
        rejected['dns']['proxy-server-nameserver'] = [self_dns, good_dns]
        failure = run('profile', 'source', 'fixture', '--file', source('ambiguous', rejected), '--apply', check=False)
        assert failure.returncode != 0 and 'DNS' in failure.stderr
        assert (profile.read_bytes(), descriptor.read_bytes()) == previous
        echo(held)
        with connect() as fresh:
            echo(fresh)
        with connect(auth=False):
            pass
        final = record()
        assert all(final[k] == original[k] for k in
                   ['pid', 'process_start_ticks', 'mixed_port', 'controller_url',
                    'controller_secret', 'proxy_username', 'proxy_password'])
        assert api('/configs')['mode'] == 'rule'
        assert api('/configs')['ipv6'] is False
        runtime = Path(final['runtime_dir']) / 'runtime.yaml'
        assert '\n- system\n' not in runtime.read_text()
        assert any(name.endswith('.nodes.test') for name in good.queries)
        assert not any(name.endswith('.nodes.test') for name in bad.queries)
        results.append('rejected DNS changes preserve cached source, credentials, core PID and existing stream')
        print(json.dumps({'passed': results, 'elapsed_seconds': round(time.monotonic() - started, 2),
                          'provider_node_queries': len(good.queries), 'wrong_resolver_node_queries': 0,
                          'fixture_connections': upstream.connects}, indent=2))
    finally:
        if held is not None:
            held.close()
        for _ in range(20):
            if run('stop', check=False).returncode == 0:
                break
            time.sleep(.2)
        for server in (good, bad, upstream):
            server.shutdown()
            server.server_close()
    assert not (base / 'runtime/session.json').exists()
    print('Test-owned processes stopped; private evidence:', base)


if __name__ == '__main__':
    main()
