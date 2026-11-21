#!/usr/bin/env python3
"""Exercise hosts redirects through a real managed core, updates and restart."""
import argparse
import base64
import copy
import http.server
import json
import os
import signal
from pathlib import Path
import socket
import socketserver
import ssl
import struct
import subprocess
import tempfile
import threading
import time
import urllib.request

import yaml


class Subscription(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.server.user_agents.append(self.headers.get('User-Agent'))
        self.send_response(200)
        self.send_header('Content-Length', str(len(self.server.body)))
        self.end_headers()
        self.wfile.write(self.server.body)

    def log_message(self, *_):
        pass


class DNS(socketserver.BaseRequestHandler):
    def handle(self):
        data, transport = self.request
        pos, labels = 12, []
        while data[pos]:
            n = data[pos]
            labels.append(data[pos + 1:pos + 1 + n].decode())
            pos += n + 1
        pos += 1
        qtype = struct.unpack('!H', data[pos:pos + 2])[0]
        name = '.'.join(labels)
        self.server.queries.append(name)
        answer = qtype == 1
        reply = data[:2] + struct.pack('!HHHHH', 0x8180, 1, int(answer), 0, 0)
        reply += data[12:pos + 4]
        if answer:
            # The original name deliberately resolves to the wrong address;
            # only the configured hosts redirect reaches the local proxy.
            ip = '127.0.0.2' if name == 'original.node.test' else '127.0.0.1'
            reply += b'\xc0\x0c' + struct.pack('!HHIH', 1, 1, 1, 4) + socket.inet_aton(ip)
        transport.sendto(reply, self.client_address)


class DNSServer(socketserver.ThreadingUDPServer):
    daemon_threads = True


class EchoProxy(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(15)
        if not self.rfile.readline(16384).startswith(b'CONNECT '):
            return
        while self.rfile.readline(16384) not in (b'\r\n', b'\n', b''):
            pass
        self.wfile.write(b'HTTP/1.1 200 Connection established\r\n\r\n')
        self.wfile.flush()
        while True:
            data = self.connection.recv(4096)
            if not data:
                return
            self.connection.sendall(data)


class EchoServer(socketserver.ThreadingTCPServer):
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
    base = Path(tempfile.mkdtemp(prefix='managed-hosts-dns-test-', dir=args.root))

    def openssl(*argv):
        subprocess.run(['openssl', *argv], cwd=base, check=True, capture_output=True, timeout=15)

    openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'ca.key', '-out', 'ca.crt',
            '-days', '2', '-subj', '/CN=MihoTerm DNS fixture CA')
    openssl('req', '-newkey', 'rsa:2048', '-nodes', '-keyout', 'server.key', '-out', 'server.csr',
            '-subj', '/CN=127.0.0.1')
    (base / 'server.ext').write_text('subjectAltName=IP:127.0.0.1\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n')
    openssl('x509', '-req', '-in', 'server.csr', '-CA', 'ca.crt', '-CAkey', 'ca.key', '-CAcreateserial',
            '-out', 'server.crt', '-days', '2', '-extfile', 'server.ext')
    subscription = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Subscription)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(base / 'server.crt', base / 'server.key')
    subscription.socket = tls.wrap_socket(subscription.socket, server_side=True)
    subscription.user_agents = []
    upstream = DNSServer(('127.0.0.1', 0), DNS)
    upstream.queries = []
    echo_server = EchoServer(('127.0.0.1', 0), EchoProxy)
    for server in (subscription, upstream, echo_server):
        threading.Thread(target=server.serve_forever, daemon=True).start()

    reservation = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    reservation.bind(('127.0.0.1', 0))
    listener_port = reservation.getsockname()[1]
    udp_reservation = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    udp_reservation.bind(('127.0.0.1', listener_port))
    dns_listener = '127.0.0.1:' + str(listener_port)
    url_file = base / 'subscription.url'
    url_file.write_text('https://127.0.0.1:' + str(subscription.server_port) + '/profile\n')
    config = base / 'config.toml'
    config.write_text('')
    original_node = {'name': 'Fixture', 'type': 'http', 'server': 'original.node.test',
                     'port': echo_server.server_address[1]}
    profile = {'ipv6': False, 'hosts': {'original.node.test': 'first.provider.test'},
               'dns': {'enable': True, 'ipv6': False, 'listen': dns_listener,
                       'use-hosts': True, 'nameserver': ['udp://127.0.0.1:' + str(upstream.server_address[1])],
                       'proxy-server-nameserver': ['udp://' + dns_listener]},
               'proxies': [original_node],
               'proxy-groups': [{'name': 'AI', 'type': 'select', 'proxies': ['Fixture']}],
               'rules': ['MATCH,AI']}
    subscription.body = json.dumps(profile).encode()
    env = {k: v for k, v in os.environ.items() if not k.startswith('MIHOTERM_')
           and k.lower() not in ('http_proxy', 'https_proxy', 'all_proxy')}
    env.update(TMPDIR=str(base), XDG_CACHE_HOME=str(base / 'cache'), SSL_CERT_FILE=str(base / 'ca.crt'))
    command = [str(args.binary.resolve()), '--config', str(config),
               '--state-dir', str(base / 'state'), '--runtime-dir', str(base / 'runtime')]

    def run(*argv, check=True):
        result = subprocess.run(command + list(argv), env=env, capture_output=True, text=True, timeout=45)
        if check and result.returncode:
            raise AssertionError('command failed: ' + ' '.join(argv[:3]) + ': ' + result.stderr[-800:])
        return result

    def record():
        return json.loads((base / 'runtime/session.json').read_text())

    def verify(expected_alias):
        current = record()
        runtime = yaml.safe_load((Path(current['runtime_dir']) / 'runtime.yaml').read_text())
        assert runtime['proxies'][0] == original_node, 'node parameters changed'
        assert runtime['hosts']['original.node.test'] == expected_alias
        assert runtime['dns']['listen'] == dns_listener
        assert runtime['dns']['proxy-server-nameserver'] == ['udp://' + dns_listener]
        request = urllib.request.Request(current['controller_url'] + '/proxies/GLOBAL',
                                         headers={'Authorization': 'Bearer ' + current['controller_secret']})
        with urllib.request.urlopen(request, timeout=5) as response:
            selected = json.load(response)
        assert selected.get('now') == 'Fixture', ('unexpected selected test proxy', selected.get('now'), selected.get('type'), selected.get('all'))
        stream = socket.create_connection(('127.0.0.1', current['mixed_port']), timeout=10)
        try:
            stream.settimeout(12)
            credentials = base64.b64encode((current['proxy_username'] + ':' + current['proxy_password']).encode())
            stream.sendall(b'CONNECT echo.test:443 HTTP/1.1\r\nHost: echo.test:443\r\n'
                           + b'Proxy-Authorization: Basic ' + credentials + b'\r\n\r\n')
            response = b''
            while b'\r\n\r\n' not in response:
                chunk = stream.recv(4096)
                assert chunk, 'proxy closed during CONNECT'
                response += chunk
            assert b' 200 ' in response, response[:100]
            payload = b'original-hosts-routing'
            stream.sendall(payload)
            received = b''
            while len(received) < len(payload):
                part = stream.recv(len(payload) - len(received))
                assert part, 'proxy closed before DNS/CONNECT succeeded'
                received += part
            assert received == payload
        finally:
            stream.close()
        assert expected_alias in upstream.queries, 'upstream never received the hosts redirect'
        assert 'original.node.test' not in upstream.queries, 'original node bypassed hosts redirect'
        return current

    results = []
    started = time.monotonic()
    try:
        run('profile', 'add', 'fixture', '--url-file', str(url_file))
        blocked = run('start', 'fixture', '--mihomo', str(args.mihomo.resolve()), check=False)
        assert blocked.returncode != 0, 'DNS port collision was incorrectly reported as a successful start'
        assert not (base / 'runtime/session.json').exists(), 'failed start left an active session'
        print('PASS occupied DNS port rejects start safely', flush=True)
        reservation.close()
        udp_reservation.close()
        run('start', 'fixture', '--mihomo', str(args.mihomo.resolve()))
        run('select', '--group', 'GLOBAL', '--proxy', 'Fixture')
        first = verify('first.provider.test')
        results.append('initial start preserves hosts redirect and original node')
        print('PASS initial start with hosts redirect', flush=True)

        updated = copy.deepcopy(profile)
        updated['hosts']['original.node.test'] = 'second.provider.test'
        subscription.body = json.dumps(updated).encode()
        time.sleep(1.2)
        run('profile', 'update', 'fixture', '--apply')
        after_update = verify('second.provider.test')
        for field in ['pid', 'mixed_port', 'controller_url', 'controller_secret', 'proxy_username', 'proxy_password']:
            assert after_update[field] == first[field], field
        results.append('HTTPS subscription update applies new hosts redirect without changing node or endpoints')
        print('PASS real profile update and hosts redirect', flush=True)

        run('stop')
        run('start', 'fixture', '--mihomo', str(args.mihomo.resolve()))
        after_restart = verify('second.provider.test')
        assert after_restart['pid'] != first['pid']
        for field in ['mixed_port', 'controller_url', 'controller_secret', 'proxy_username', 'proxy_password']:
            assert after_restart[field] == first[field], field
        results.append('stop/start restores updated hosts redirect and inherited client endpoints')
        print('PASS managed stop/start and hosts redirect', flush=True)

        updated['hosts']['original.node.test'] = 'third.provider.test'
        subscription.body = json.dumps(updated).encode()
        time.sleep(1.2)
        run('profile', 'update', 'fixture', '--apply')
        last = verify('third.provider.test')
        assert last['pid'] == after_restart['pid']
        results.append('HTTPS subscription update after restart preserves the DNS path')

        occupied = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        occupied.bind(('127.0.0.1', 0))
        occupied.listen(1)
        conflict_port = occupied.getsockname()[1]
        occupied_udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        occupied_udp.bind(('127.0.0.1', conflict_port))
        profile_path = base / 'state/profiles/fixture/profile.yaml'
        previous_profile = profile_path.read_bytes()
        blocked_update = copy.deepcopy(updated)
        blocked_update['dns']['listen'] = '127.0.0.1:' + str(conflict_port)
        blocked_update['dns']['proxy-server-nameserver'] = ['udp://127.0.0.1:' + str(conflict_port)]
        subscription.body = json.dumps(blocked_update).encode()
        try:
            rejected = run('profile', 'update', 'fixture', '--apply', check=False)
            assert rejected.returncode != 0, 'DNS port collision was incorrectly reported as a successful update'
            assert profile_path.read_bytes() == previous_profile, 'failed update did not restore the profile'
            after_rejected = verify('third.provider.test')
            assert after_rejected['pid'] == last['pid']
        finally:
            occupied.close()
            occupied_udp.close()
            subscription.body = json.dumps(updated).encode()
        results.append('occupied DNS port rejects update and preserves working session and profile')
        print('PASS occupied DNS port rejects update safely', flush=True)

        killed_pid = last['pid']
        process = Path('/proc') / str(killed_pid)
        assert process.stat().st_uid == os.getuid()
        assert str(base).encode() in (process / 'cmdline').read_bytes(), 'refusing to interrupt a non-test core'
        os.kill(killed_pid, signal.SIGKILL)
        deadline = time.monotonic() + 35
        recovered = None
        while time.monotonic() < deadline:
            try:
                candidate = record()
                if candidate['pid'] != killed_pid:
                    recovered = candidate
                    break
            except (OSError, KeyError, ValueError):
                pass
            time.sleep(.2)
        assert recovered is not None, 'supervisor did not recover its test core'
        recovered = verify('third.provider.test')
        for field in ['mixed_port', 'controller_url', 'controller_secret', 'proxy_username', 'proxy_password']:
            assert recovered[field] == first[field], field
        results.append('automatic recovery after core exit preserves the DNS path and endpoints')
        assert len(subscription.user_agents) >= 4 and all(ua == 'clash.meta' for ua in subscription.user_agents)
        print('PASS automatic core recovery and hosts redirect', flush=True)
        print(json.dumps({'passed': results, 'elapsed_seconds': round(time.monotonic() - started, 2),
                          'queried_expected_aliases': sorted(set(upstream.queries)),
                          'node_unchanged': True, 'original_name_direct_queries': 0}, indent=2))
    except Exception:
        try:
            current = record()
            core_log = Path(current['runtime_dir']) / 'mihomo.log'
            if core_log.exists():
                print('TEST_CORE_LOG_TAIL', '\n'.join(core_log.read_text(errors='replace').splitlines()[-20:]), flush=True)
        except (OSError, KeyError, ValueError):
            pass
        print('TEST_EVIDENCE', base, flush=True)
        raise
    finally:
        reservation.close()
        udp_reservation.close()
        for _ in range(20):
            if run('stop', check=False).returncode == 0:
                break
            time.sleep(.2)
        for server in (subscription, upstream, echo_server):
            server.shutdown()
            server.server_close()
    assert not (base / 'runtime/session.json').exists()
    print('Test-owned processes stopped; private evidence:', base)


if __name__ == '__main__':
    main()
