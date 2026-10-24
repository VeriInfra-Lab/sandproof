"""Test-only, finite double-mTLS relay for deterministic receipt loss.

No production hook, arbitrary URL, VM command or success synthesizer. Both
legs verify the fixed operator CA/SAN and exact leaf. Only original internal
paths are forwarded. This probe deliberately bounds complete short test bodies;
it is not the product streaming implementation or a general network proxy.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import base64
import copy
import hashlib
import http.client
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import socket
from socketserver import ThreadingMixIn
import ssl
import stat
import threading

from tools.check_kvm_host import unique_keys
from tools.system_acceptance import private_path

BODY_BYTES = 2 * 1024**2
MATERIAL_BYTES = 512 * 1024
LIFECYCLE = '/internal/lifecycle'
CREATE_OBSERVATION = '/internal/lifecycle/create-observation'
CONTROL_PATHS = frozenset((LIFECYCLE, '/internal/lifecycle/result-confirmation',
                          '/internal/lifecycle/window-close',
                          '/internal/lifecycle/create-observation',
                          '/internal/lifecycle/terminal-observation',
                          '/internal/lifecycle/renewal-confirmation'))
STATUS_PATH = '/internal/node-status'
DATA_PREFIX = '/internal/data/'
PATHS = CONTROL_PATHS | frozenset((STATUS_PATH, '/internal/probe',
                                  '/internal/template-cache', '/internal/template-validation-evidence'))
HOP_HEADERS = frozenset(('host', 'connection', 'transfer-encoding', 'content-length',
                         'proxy-connection', 'keep-alive', 'upgrade', 'trailer'))


def allowed_path(path):
    return (type(path) is str and path.startswith('/') and not path.startswith('//')
            and len(path) <= 4096 and ('?' not in path or path.startswith(DATA_PREFIX))
            and (path in PATHS or path.startswith(DATA_PREFIX)))


def read_body(source, headers, *, limit=BODY_BYTES):
    """Finite test HTTP framing. Ambiguous/truncated/trailer bodies are refused."""
    lengths = headers.get_all('Content-Length', [])
    transfers = headers.get_all('Transfer-Encoding', [])
    if len(lengths) > 1 or len(transfers) > 1 or (lengths and transfers):
        raise ValueError('ambiguous test framing')
    if transfers:
        if transfers != ['chunked']:
            raise ValueError('unsupported test framing')
        output = bytearray()
        while True:
            line = source.readline(128)
            if not line.endswith(b'\r\n') or len(line) >= 128 or b';' in line:
                raise ValueError('invalid test chunk')
            try:
                size = int(line[:-2], 16)
            except ValueError as error:
                raise ValueError('invalid test chunk') from error
            if size < 0 or size > limit - len(output):
                raise ValueError('test body exceeds budget')
            if size == 0:
                if source.readline(3) != b'\r\n':
                    raise ValueError('test trailers refused')
                return bytes(output)
            data = source.read(size + 2)
            if len(data) != size + 2 or data[-2:] != b'\r\n':
                raise ValueError('truncated test chunk')
            output.extend(data[:-2])
    if not lengths:
        return b''
    if not lengths[0].isascii() or not lengths[0].isdecimal():
        raise ValueError('invalid test length')
    size = int(lengths[0])
    if size > limit:
        raise ValueError('test body exceeds budget')
    data = source.read(size)
    if len(data) != size:
        raise ValueError('truncated test body')
    return data


def _fields(value, names):
    if type(value) is not dict or set(value) != set(names.split()):
        raise ValueError('current lifecycle fields required')
    return value


def _u64(value, *, positive=False):
    if type(value) is not int or not int(positive) <= value < 2**64:
        raise ValueError('typed lifecycle counter required')
    return value


def _opaque(value):
    if (type(value) is not str or not 0 < len(value) <= 128
            or any(not (c.isascii() and (c.isalnum() or c in '-_.')) for c in value)):
        raise ValueError('bounded lifecycle identifier required')


def _bytes(value, *, size=None):
    if (type(value) is not list or (size is not None and len(value) != size)
            or any(type(v) is not int or not 0 <= v <= 255 for v in value)):
        raise ValueError('lifecycle byte array required')


def accepted_request(value, node):
    """Finite test correlation of current envelopes; never admission authority."""
    _fields(value, 'request accepted_version center retention')
    version = _u64(value['accepted_version'], positive=True)
    center = _fields(value['center'], 'deployment center')
    for v in center.values():
        _opaque(v)
    retention = _fields(value['retention'], 'slot window')
    if not 1 <= _u64(retention['slot']) <= 4096:
        raise ValueError('original retained position required')
    _u64(retention['window'], positive=True)
    request = _fields(value['request'], 'binding operation')
    binding = _fields(request['binding'], 'protocol_version node session key expected')
    actual_node = _fields(binding['node'], 'deployment_id node_id node_generation')
    _opaque(actual_node['deployment_id'])
    _opaque(actual_node['node_id'])
    _u64(actual_node['node_generation'], positive=True)
    if (type(binding['protocol_version']) is not int or binding['protocol_version'] != 3
            or actual_node != node or center['deployment'] != actual_node['deployment_id']):
        raise ValueError('current protocol and exact original node required')
    _opaque(binding['session'])
    key = _fields(binding['key'], 'deployment owner window sequence')
    _opaque(key['deployment'])
    _opaque(key['owner'])
    _u64(key['window'], positive=True)
    _u64(key['sequence'], positive=True)
    if key['deployment'] != actual_node['deployment_id']:
        raise ValueError('request deployment changed')
    expected = binding['expected']
    operation = request['operation']
    if type(operation) is not dict:
        raise ValueError('current operation required')
    kind = operation.get('kind')
    shapes = {'create': 'kind create target_incarnation',
              'destroy': 'kind', 'unload': 'kind source_incarnation capture',
              'resume': 'kind source_incarnation capture target_incarnation manifest_digest',
              'renew': 'kind incarnation end_at_ms extend_only'}
    if type(kind) is not str or kind not in shapes:
        raise ValueError('current operation kind required')
    _fields(operation, shapes[kind])
    if kind == 'create':
        _fields(expected, 'state')
        if expected['state'] != 'absent' or version != 1:
            raise ValueError('initial Create acceptance required')
        create = _fields(operation['create'], 'template_revision template_digest resources options startup_deadline_ms timeout_ms max_lifetime_ms')
        _opaque(create['template_revision'])
        _bytes(create['template_digest'], size=32)
        _bytes(create['options'])
        if not 0 < len(create['options']) <= 65536:
            raise ValueError('bounded full Create options required')
        resources = _fields(create['resources'], 'instances cpu memory_bytes disk_bytes')
        for v in resources.values():
            _u64(v, positive=True)
        for name in ('startup_deadline_ms', 'timeout_ms', 'max_lifetime_ms'):
            _u64(create[name], positive=True)
        if (resources['instances'] != 1 or create['timeout_ms'] > create['max_lifetime_ms']
                or create['startup_deadline_ms'] + create['max_lifetime_ms'] >= 2**64):
            raise ValueError('original Create bounds required')
    else:
        _fields(expected, 'state version')
        if expected['state'] != 'version' or _u64(expected['version']) + 1 != version:
            raise ValueError('accepted successor required')
    for name in ('target_incarnation', 'source_incarnation', 'capture', 'incarnation'):
        if name in operation:
            _u64(operation[name], positive=True)
    if kind == 'resume':
        _bytes(operation['manifest_digest'], size=32)
        if operation['target_incarnation'] <= operation['source_incarnation']:
            raise ValueError('new resume incarnation required')
    if kind == 'renew' and (type(operation['end_at_ms']) is not int
            or not -(2**63) <= operation['end_at_ms'] < 2**63
            or type(operation['extend_only']) is not bool):
        raise ValueError('typed normalized renewal required')
    return operation


class ReceiptBarrier:
    """A real Ready is held before any response bytes are sent centerward."""
    def __init__(self, node):
        self.node = node
        self.lock = threading.Lock()
        self.ready = threading.Event()
        self.release = threading.Event()
        self.release.set()
        self.armed = False
        self.captured = None
        self.discard = False
        self.forwarded = 0

    def arm(self):
        with self.lock:
            if self.armed or not self.release.is_set():
                raise RuntimeError('receipt barrier already active')
            self.armed, self.captured, self.discard, self.forwarded = True, None, False, 0
            self.ready.clear()
            self.release.clear()

    def observe(self, request, response):
        with self.lock:
            if not self.armed:
                return False
            operation = accepted_request(request, self.node)
            if operation['kind'] != 'create':
                return False
            _fields(response, 'request current_request historical_create_succeeded readiness')
            if type(response['historical_create_succeeded']) is not bool:
                raise ValueError('typed historical success required')
            accepted_request(response['request'], self.node)
            accepted_request(response['current_request'], self.node)
            if response['request'] != request:
                raise ValueError('observation changed full original request')
            readiness = response['readiness']
            if type(readiness) is not dict:
                raise ValueError('current readiness required')
            state = readiness.get('state')
            if state in ('pending', 'not_current', 'not_running', 'expired'):
                _fields(readiness, 'state')
                return False
            if state == 'refused':
                _fields(readiness, 'state reason')
                if readiness['reason'] != 'capacity':
                    raise ValueError('unknown refusal reason')
                return False
            _fields(readiness, 'state facts')
            if state != 'observed':
                raise ValueError('unknown current readiness')
            if response['current_request'] != request or not response['historical_create_succeeded']:
                raise ValueError('Ready must describe this exact current Create')
            facts = _fields(readiness['facts'], 'started_at_ms end_at_ms max_end_at_ms startup_deadline_ms observed_at_ms')
            for v in facts.values():
                _u64(v)
            create = operation['create']
            start = facts['started_at_ms']
            if (facts['startup_deadline_ms'] != create['startup_deadline_ms']
                    or start > create['startup_deadline_ms']
                    or facts['end_at_ms'] != start + create['timeout_ms']
                    or facts['max_end_at_ms'] != start + create['max_lifetime_ms']
                    or not start <= facts['observed_at_ms'] < facts['end_at_ms'] <= facts['max_end_at_ms']):
                raise ValueError('Ready changed original lifetime or current observation')
            if self.captured is None:
                self.captured = copy.deepcopy(response)
            else:
                # Real readonly queries can observe the same execution later;
                # no invented host sequence or frozen observation time exists.
                previous = copy.deepcopy(self.captured)
                previous['readiness']['facts']['observed_at_ms'] = facts['observed_at_ms']
                if previous != response:
                    raise ValueError('receipt loss produced another request or lifetime')
            self.ready.set()
            return True

    def unblock(self, *, discard):
        with self.lock:
            self.discard, self.armed = discard, False
            self.release.set()


def _pem(kind, data):
    encoded = base64.b64encode(data).decode('ascii')
    return ('-----BEGIN ' + kind + '-----\n' + '\n'.join(
        encoded[index:index+64] for index in range(0, len(encoded), 64))
        + '\n-----END ' + kind + '-----\n').encode('ascii')


class RelayMaterials:
    """Exact root-only temporary PEM copies inside original host-control budget."""
    def __init__(self, operator, control, receipt):
        operator, control = private_path(operator, directory=True), private_path(control, directory=True)
        data = {}
        for name in ('ca.der', 'center.der', 'center-key.der', 'host-runtime.der', 'host-runtime-key.der'):
            path = private_path(operator/name)
            before = path.stat()
            if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > 65536:
                raise ValueError('fixed bounded operator material required')
            with path.open('rb') as source:
                data[name] = source.read(65537)
            after = path.stat()
            if ((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
                    != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
                    or hashlib.sha256(data[name]).hexdigest() != receipt[name]):
                raise ValueError('operator TLS receipt changed')
        self.root = control/('dk-relay-' + os.urandom(16).hex())
        self.root.mkdir(mode=0o700)
        self.identity = (self.root.stat().st_dev, self.root.stat().st_ino)
        self.files = {}
        payloads = {name + '.pem': _pem('PRIVATE KEY' if '-key.' in name else 'CERTIFICATE', value)
                    for name, value in data.items()}
        if sum(map(len, payloads.values())) > MATERIAL_BYTES or len(payloads) + 1 > 8:
            raise ValueError('relay material exceeds original declared budget')
        for name, value in payloads.items():
            path = self.root/name
            fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
            with os.fdopen(fd, 'wb') as output:
                output.write(value)
                output.flush()
                os.fsync(output.fileno())
            self.files[path] = (path.stat().st_dev, path.stat().st_ino)
        self.center_pin = hashlib.sha256(data['center.der']).digest()
        self.host_pin = hashlib.sha256(data['host-runtime.der']).digest()
        self.server = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self.server.minimum_version = ssl.TLSVersion.TLSv1_2
        self.server.verify_mode = ssl.CERT_REQUIRED
        self.server.set_alpn_protocols(['http/1.1'])
        self.server.load_verify_locations(str(self.root/'ca.der.pem'))
        self.server.load_cert_chain(str(self.root/'host-runtime.der.pem'), str(self.root/'host-runtime-key.der.pem'))
        self.client = ssl.create_default_context(cafile=str(self.root/'ca.der.pem'))
        self.client.minimum_version = ssl.TLSVersion.TLSv1_2
        self.client.set_alpn_protocols(['http/1.1'])
        self.client.load_cert_chain(str(self.root/'center.der.pem'), str(self.root/'center-key.der.pem'))

    def remove(self):
        if (self.root.stat().st_dev, self.root.stat().st_ino) != self.identity or set(self.root.iterdir()) != set(self.files):
            raise ValueError('relay material identity changed; retain evidence')
        for path, identity in self.files.items():
            info = path.lstat()
            if ((info.st_dev, info.st_ino) != identity or not stat.S_ISREG(info.st_mode)
                    or info.st_uid != 0 or info.st_nlink != 1 or info.st_mode & 0o077):
                raise ValueError('relay material inode changed; retain evidence')
        for path in self.files:
            path.unlink()
        self.root.rmdir()


class _Backend(http.client.HTTPSConnection):
    def __init__(self, address, materials):
        super().__init__(address[0], address[1], context=materials.client, timeout=10)
        self.materials = materials

    def connect(self):
        raw = socket.create_connection((self.host, self.port), timeout=self.timeout)
        try:
            self.sock = self._context.wrap_socket(raw, server_hostname='host.test')
            if self.sock.selected_alpn_protocol() != 'http/1.1':
                raise ValueError('fixed host HTTP protocol mismatch')
            if hashlib.sha256(self.sock.getpeercert(binary_form=True)).digest() != self.materials.host_pin:
                raise ValueError('fixed host leaf mismatch')
        except BaseException:
            if self.sock is not None:
                self.sock.close()
            raw.close()
            raise


class _FiniteServer(ThreadingMixIn, HTTPServer):
    daemon_threads = False
    block_on_close = True
    def __init__(self, address, relay):
        self.relay = relay
        self.work = threading.BoundedSemaphore(8)
        super().__init__(address, _Handler)

    def get_request(self):
        raw, address = super().get_request()
        raw.settimeout(5)
        connection = None
        try:
            connection = self.relay.materials.server.wrap_socket(raw, server_side=True)
            if connection.selected_alpn_protocol() != 'http/1.1':
                raise ValueError('fixed center HTTP protocol mismatch')
            if hashlib.sha256(connection.getpeercert(binary_form=True)).digest() != self.relay.materials.center_pin:
                raise ValueError('fixed center leaf mismatch')
            connection.settimeout(15)
            return connection, address
        except BaseException:
            if connection is not None:
                connection.close()
            raw.close()
            raise OSError('fixed mutual authentication failed') from None

    def process_request(self, request, address):
        if not self.work.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.work.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.work.release()

    def handle_error(self, request, address):
        self.relay.failed.set()  # no request/key/body/exception text is printed


class _Handler(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args):
        pass

    def do_POST(self):
        self._forward()

    def do_GET(self):
        self._forward()

    def do_DELETE(self):
        self._forward()

    def do_PUT(self):
        self._forward()

    def _forward(self):
        relay = self.server.relay
        if not allowed_path(self.path) or len(self.headers) > 32:
            self._reply(400, [], b'')
            return
        body = read_body(self.rfile, self.headers)
        request = json.loads(body, object_pairs_hook=unique_keys) if self.path in (LIFECYCLE, CREATE_OBSERVATION) else None
        if self.path in CONTROL_PATHS and relay.partition.is_set():
            if request is not None:
                relay.note_partition(request, path=self.path)
            self._reply(503, [], b'')
            return
        backend = _Backend(relay.backend, relay.materials)
        try:
            headers = {name: value for name, value in self.headers.items() if name.lower() not in HOP_HEADERS}
            headers['Content-Length'] = str(len(body))
            headers['Connection'] = 'close'
            backend.request(self.command, self.path, body=body, headers=headers)
            response = backend.getresponse()
            data = response.read(BODY_BYTES + 1)
            if len(data) > BODY_BYTES:
                raise ValueError('test response exceeds finite budget')
            outgoing = [(name, value) for name, value in response.getheaders() if name.lower() not in HOP_HEADERS]
            held = (self.path == CREATE_OBSERVATION and response.status == 200
                    and relay.barrier.observe(request, json.loads(data, object_pairs_hook=unique_keys)))
            if held:
                if not relay.barrier.release.wait(timeout=60):
                    raise TimeoutError('receipt barrier not explicitly released')
                if relay.barrier.discard:
                    self.close_connection = True
                    return  # not a single response byte forwarded centerward
                relay.barrier.forwarded += 1
            self._reply(response.status, outgoing, data)
        except (OSError, http.client.HTTPException):
            # Original fault cases deliberately stop the fixed host while an
            # accepted call is in flight. A transport failure is unavailable,
            # never a fabricated lifecycle fact or a relay-parser failure.
            self._reply(503, [], b'')
        finally:
            backend.close()

    def _reply(self, status, headers, body):
        self.close_connection = True
        try:
            self.send_response(status)
            for name, value in headers:
                self.send_header(name, value)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(body)
        except OSError:
            # The exact center may have exited at the Ready barrier. The
            # bounded original reply was not delivered; do not count it as
            # successful or treat deliberate socket cancellation as bad data.
            self.close_connection = True


class InternalFaultRelay:
    def __init__(self, *, listen, backend, node, materials):
        for value in (listen, backend):
            if (type(value) is not tuple or len(value) != 2 or value[0] != '127.0.0.1'
                    or type(value[1]) is not int or not 0 < value[1] < 65536):
                raise ValueError('two fixed loopback operator endpoints required')
        if listen == backend:
            raise ValueError('relay cannot forward to itself')
        self.backend, self.materials = backend, materials
        self.barrier = ReceiptBarrier(node)
        self.partition = threading.Event()
        self.destroy_seen = threading.Event()
        self.failed = threading.Event()
        self.partition_requests = []
        self.lock = threading.Lock()
        self.server = _FiniteServer(listen, self)
        self.thread = threading.Thread(target=self.server.serve_forever, name='dk-fixed-mtls-relay')

    def note_partition(self, request, *, path):
        operation = accepted_request(request, self.barrier.node)
        if path not in (LIFECYCLE, CREATE_OBSERVATION):
            raise ValueError('partition evidence requires an original request path')
        with self.lock:
            if request not in self.partition_requests:
                if len(self.partition_requests) == 32:
                    raise ValueError('finite partition evidence budget exhausted')
                self.partition_requests.append(copy.deepcopy(request))
        if path == LIFECYCLE and operation == {'kind': 'destroy'}:
            self.destroy_seen.set()

    def start(self):
        self.thread.start()

    def stop(self):
        self.partition.clear()
        self.barrier.unblock(discard=True)
        self.server.shutdown()
        self.thread.join(timeout=10)
        if self.thread.is_alive():
            raise RuntimeError('test relay did not exit')
        self.server.server_close()  # joins actual finite worker threads

    def facts(self):
        fields = Path('/proc/self/stat').read_text().rsplit(')', 1)[1].split()
        return dict(kind='test_only_in_process_mtls_relay', pid=os.getpid(),
            start_time_ticks=int(fields[19]), cgroup=Path('/proc/self/cgroup').read_text().strip(),
            node=self.barrier.node, center_leaf_sha256=self.materials.center_pin.hex(),
            host_leaf_sha256=self.materials.host_pin.hex())
