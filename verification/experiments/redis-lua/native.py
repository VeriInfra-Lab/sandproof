"""Finite real-engine checks for the frozen Lua inputs, not durability proofs.

The engine is new, private and has no TCP listener. No application database is
opened. Counterexamples must first compile in Redis and violate the SAME check.
"""

import argparse
import contextlib
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time

from source import ROOT, mutants, source_record


class RedisError(Exception):
    pass


class Client:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(5)
        self.socket.connect(str(path))
        self.reader = self.socket.makefile('rb')

    def close(self):
        self.reader.close()
        self.socket.close()

    def read(self):
        line = self.reader.readline(1024 * 1024)
        if not line.endswith(b'\r\n'):
            raise ValueError('truncated or oversized RESP line')
        tag, body = line[:1], line[1:-2]
        if tag == b'+':
            return body
        if tag == b'-':
            raise RedisError(body.decode('ascii', errors='replace'))
        if tag == b':':
            return int(body)
        if tag == b'$':
            n = int(body)
            if n == -1:
                return None
            if not 0 <= n <= 1024 * 1024:
                raise ValueError('RESP bulk outside this finite probe domain')
            raw = self.reader.read(n + 2)
            if len(raw) != n + 2 or raw[-2:] != b'\r\n':
                raise ValueError('truncated RESP bulk')
            return raw[:-2]
        if tag == b'*':
            n = int(body)
            if not 0 <= n <= 128:
                raise ValueError('RESP array outside this finite probe domain')
            return [self.read() for _ in range(n)]
        raise ValueError('unexpected RESP type')

    def call(self, *arguments):
        values = [a if isinstance(a, bytes) else str(a).encode('ascii') for a in arguments]
        request = b'*' + str(len(values)).encode() + b'\r\n'
        for value in values:
            request += b'$' + str(len(value)).encode() + b'\r\n' + value + b'\r\n'
        self.socket.sendall(request)
        return self.read()


@contextlib.contextmanager
def engine(binary, output):
    version = subprocess.check_output([str(binary), '--version'], text=True, timeout=10)
    if not re.fullmatch(r'Redis server v=7\.2\.16 sha=[^\r\n]*\n?', version):
        raise ValueError('the original fixed Redis 7.2.16 is required')
    # Short private path, independent of potentially long caller TMPDIR.
    with tempfile.TemporaryDirectory(prefix='sp335-', dir='/tmp') as directory:
        os.chmod(directory, 0o700)
        path = Path(directory) / 'redis.sock'
        with (output / 'redis.log').open('wb') as log:
            process = subprocess.Popen([
                str(binary), '--port', '0', '--unixsocket', str(path),
                '--unixsocketperm', '700', '--save', '', '--appendonly', 'no',
                '--maxmemory', '134217728', '--maxmemory-policy', 'noeviction',
                '--dir', directory,
            ], stdout=log, stderr=subprocess.STDOUT)
            client = None
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError('private Redis exited before readiness')
                    try:
                        client = Client(path)
                        if client.call('PING') == b'PONG':
                            break
                    except (ConnectionError, FileNotFoundError):
                        pass
                    time.sleep(0.01)
                if client is None:
                    raise TimeoutError('private Redis never became ready')
                yield client
            finally:
                if client is not None:
                    client.close()
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                (output / 'engine-exit.json').write_text(json.dumps({
                    'pid': process.pid, 'returncode': process.returncode,
                    'reaped': True, 'only_private_engine_stopped': True,
                }) + '\n')


KEY = b'sandproof:335:case'
OTHER = b'sandproof:335:unrelated'
FIELDS = {b'deployment': b'dep', b'format': b'1', b'revision': b'1', b'payload': b'old'}


def prepare(client, fields=None, *, text=None, ttl=False):
    client.call('DEL', KEY)
    client.call('SET', OTHER, b'untouched')
    if fields is not None:
        values = [v for pair in fields.items() for v in pair]
        client.call('HSET', KEY, *values)
    if text is not None:
        client.call('SET', KEY, text)
    if ttl:
        client.call('PEXPIRE', KEY, 60000)


def snapshot(client):
    kind = client.call('TYPE', KEY)
    if kind == b'hash':
        raw = client.call('HGETALL', KEY)
        value = sorted(zip(raw[::2], raw[1::2]))
    elif kind == b'string':
        value = client.call('GET', KEY)
    else:
        value = None
    # Expiring-key time decreases; preserve the expiry category, not a timestamp.
    ttl = client.call('PTTL', KEY)
    return kind, value, ttl if ttl < 0 else 'expiring'


def hsets(client):
    info = client.call('INFO', 'commandstats')
    match = re.search(rb'^cmdstat_hset:calls=(\d+),', info, re.MULTILINE)
    return int(match[1]) if match else 0


def cas_arguments(**changes):
    arguments = dict(deployment=b'dep', format=b'1', prior=b'1', next=b'2',
                     payload=b'new', maximum=b'64', old=b'old')
    arguments.update(changes)
    return list(arguments.values())


def check_cas(client, source, fields, arguments, code, *, text=None, ttl=False):
    prepare(client, fields, text=text, ttl=ttl)
    client.call('SCRIPT', 'LOAD', source)  # Compilation failure is NOT a counterexample.
    before, count = snapshot(client), hsets(client)
    actual = client.call('EVAL', source, 1, KEY, *arguments)
    after, delta = snapshot(client), hsets(client) - count
    assert actual == code, (actual, code)
    assert client.call('GET', OTHER) == b'untouched'
    if code != 0:
        assert after == before, 'normal guard rejection changed the original record'
        assert delta == 0, 'normal guard rejection performed HSET'
    else:
        expected = {b'deployment': arguments[0], b'format': arguments[1],
                    b'revision': arguments[3], b'payload': arguments[4]}
        assert after == (b'hash', sorted(expected.items()), -1), 'accepted CAS has wrong fields'
        assert delta == 1, 'accepted CAS must execute exactly one real HSET'
    return {'code': actual, 'actual_HSET_calls': delta, 'unrelated_key_preserved': True}


def run(binary, output):
    record, scripts = source_record(ROOT)
    output.mkdir(parents=True, exist_ok=False)
    results = []
    with engine(binary, output) as client:
        def case(name, fields=FIELDS, code=0, *, text=None, ttl=False, **changes):
            result = check_cas(client, scripts['CAS'], fields, cas_arguments(**changes),
                               code, text=text, ttl=ttl)
            results.append(dict(name=name, **result))
        case('initialize', None, prior=b'', next=b'1')
        case('already-initialized', code=2, prior=b'')
        case('missing', None, code=1)
        case('wrong-type', None, code=3, text=b'value')
        case('three-fields', {k: v for k, v in FIELDS.items() if k != b'format'}, code=3)
        case('five-fields', {**FIELDS, b'extra': b'x'}, code=3)
        case('foreign-deployment', code=3, deployment=b'other')
        case('foreign-format', code=3, format=b'2')
        case('revision-conflict', code=4, prior=b'9')
        case('old-payload-mismatch', code=3, old=b'other')
        case('expiry-rejected', code=3, ttl=True)
        case('empty-next', code=3, payload=b'')
        case('oversized-next', code=3, payload=b'x' * 65)
        case('oversized-old', {**FIELDS, b'payload': b'x' * 65}, code=3, old=b'x' * 65)
        case('changed-payload')
        case('reaffirm-unchanged', payload=b'old')
        case('full-u64-revision', {**FIELDS, b'revision': b'18446744073709551614'},
             prior=b'18446744073709551614', next=b'18446744073709551615')
        case('binary-payload', {**FIELDS, b'payload': b'\x00\xffold'},
             old=b'\x00\xffold', payload=b'\x00\xfe\x80new')
        case('revision-first-error', code=4, prior=b'9', old=b'wrong')
        case('size-before-already-initialized', code=3, prior=b'', payload=b'')
        for name, fields, expected, ttl in [
            ('read-missing', None, [1], False),
            ('read-valid', FIELDS, [0, b'1', b'old'], False),
            ('read-expiring', FIELDS, [3], True),
            ('read-empty-payload', {**FIELDS, b'payload': b''}, [3], False),
            # Lua '' is truthy; the Rust caller still has to validate revisions.
            ('read-noncanonical-revision', {**FIELDS, b'revision': b'00'},
             [0, b'00', b'old'], False),
        ]:
            prepare(client, fields, ttl=ttl)
            before, count = snapshot(client), hsets(client)
            actual = client.call('EVAL', scripts['READ'], 1, KEY, b'dep', b'1', b'64')
            assert actual == expected and snapshot(client) == before and hsets(client) == count
            results.append({'name': name, 'READ_reply_hex': repr(actual), 'actual_HSET_calls': 0})
        negative = []
        for name, bad in mutants(scripts['CAS']).items():
            compiled = client.call('SCRIPT', 'LOAD', bad)
            try:
                check_cas(client, bad, FIELDS, cas_arguments(payload=b'old'), 0)
            except AssertionError as error:
                negative.append({'name': name, 'compiled_sha1': compiled.decode(),
                                 'same_contract_rejected': True, 'reason': str(error)})
            else:
                raise AssertionError('wrong Lua was not rejected by the original check')
        # A real post-write script error is retained with its write, not rolled back.
        error_source = scripts['CAS'].removesuffix(b'\n').removesuffix(b'return 0') + \
            b"error('sp335-after-write')\n"
        client.call('SCRIPT', 'LOAD', error_source)
        prepare(client, FIELDS)
        count = hsets(client)
        try:
            client.call('EVAL', error_source, 1, KEY, *cas_arguments())
        except RedisError as error:
            assert 'sp335-after-write' in str(error)
        else:
            raise AssertionError('expected the explicit post-write error')
        assert client.call('HGET', KEY, b'revision') == b'2' and hsets(client) - count == 1
    result = {'source': record, 'redis_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'cases': results, 'negative': negative, 'post_write_error_keeps_write': True,
              'scope': 'finite functional/compilation evidence; no durability or general Lua proof'}
    (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--redis-server', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = run(args.redis_server.resolve(), args.output)
    print(json.dumps({'cases': len(result['cases']), 'negative': len(result['negative']),
                      'post_write_error_keeps_write': result['post_write_error_keeps_write']}))
