"""Delay one original publication through a loopback HTTP barrier (#189).

Called by the isolated self-service probe with both real KVM pools alive and
the background supervisor stopped. Only delivery is delayed: BuildStore,
Supervisor, staging, runtime admission and its check VM are production code.
The build clock advances explicitly; runtime and VM clocks remain real.
"""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import secrets
import sqlite3
import threading
import time
import urllib.error
import urllib.request

from tools.template_build_service import Backend, Supervisor, configuration
from tools.template_build_store import BuildStore


def exercise(config_path, context, request):
    config = configuration(config_path)
    if config.get('bounded_builds'):
        raise ValueError('controlled build clock probe requires the explicit legacy fixture; not bounded-retention acceptance')
    now = [time.time()]
    store = None
    gate, received, dispatched = threading.Event(), threading.Event(), threading.Event()
    posts, failures = [], []
    origin = 'http://' + config['runtime']['bind']
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    class Barrier(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def relay(self):
            try:
                size = int(self.headers.get('Content-Length', '0'))
                assert 0 <= size <= 65536
                body = self.rfile.read(size) if size else None
                if self.command == 'POST':
                    assert self.path == '/sandproof/template-publications'
                    assert not posts, 'must not repeat the original publication'
                    posts.append(json.loads(body)['candidateID'])
                    received.set()
                    assert gate.wait(30), 'delivery barrier was never released'
                else:
                    assert self.command == 'GET' and self.path.startswith('/sandproof/template-publications/')
                upstream = urllib.request.Request(origin + self.path, data=body, method=self.command,
                    headers={'X-API-Key': self.headers['X-API-Key'], 'Content-Type': 'application/json'})
                try:
                    response = opener.open(upstream, timeout=120)
                except urllib.error.HTTPError as error:
                    response = error
                with response:
                    data, code = response.read(65537), response.code
                assert len(data) <= 65536
                if self.command == 'POST':
                    assert code == 200, 'delayed original POST was not admitted'
                    dispatched.set()
                try:
                    self.send_response(code)
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # Its original client has deliberately timed out.
            except BaseException as error:
                failures.append(type(error).__name__)

        do_GET = relay
        do_POST = relay

    proxy = ThreadingHTTPServer(('127.0.0.1', 0), Barrier)
    proxy.daemon_threads = True
    thread = threading.Thread(target=proxy.serve_forever)
    thread.start()

    class DelayedBackend(Backend):
        def request(self, pool, method, path, body=None, **kwargs):
            if pool == 'runtime' and method == 'POST':
                kwargs['timeout'] = min(kwargs.get('timeout', 60), 0.2)
            return super().request(pool, method, path, body, **kwargs)

    proxied = copy.deepcopy(config)
    proxied['runtime']['bind'] = f'127.0.0.1:{proxy.server_port}'
    backend = DelayedBackend(proxied)

    def reopen():
        return BuildStore(config['root'], config['limits'], clock=lambda: now[0],
                          binding=config['binding'], bounded=config.get('bounded_builds'),
                          physical_profile=config.get('physical_storage_profile'))

    try:
        store = reopen()
        job = store.submit(1, request, secrets.token_hex(16))['buildID']
        with context.open('rb') as source:
            store.upload(job, 1, iter(lambda: source.read(65536), b''), context.stat().st_size)
        supervisor = Supervisor(store, backend)
        supervisor.tick()  # Real builder, download and exact VM reclamation.
        assert store.get(job)['state'] == 'Publishing'
        supervisor.tick()  # Real HTTP POST reaches the barrier, reply times out.
        assert received.is_set() and posts == [job] and not dispatched.is_set()
        charged = store.get(job)['reserved']
        assert charged > 0 and store.get(job)['publication_attempted'] == 1
        with sqlite3.connect(Path(config['root']) / 'builds.db') as independent:
            assert independent.execute('SELECT publication_attempted FROM jobs WHERE id=?', (job,)).fetchone() == (1,)
        now[0] = store.get(job)['deadline'] + 1
        supervisor.tick()  # Real runtime 404 after expiry must not delete input.
        assert store.get(job)['state'] == 'Publishing' and store.get(job)['reserved'] == charged
        staged = Path(config['runtime']['template_registry']['root']) / 'inbox' / job
        assert staged.is_dir()
        store.close()
        store = reopen()  # Actual SQLite close/reopen before a second 404.
        supervisor = Supervisor(store, backend)
        supervisor.tick()
        assert store.get(job)['state'] == 'Publishing' and store.get(job)['reserved'] == charged
        assert staged.is_dir() and posts == [job]
        gate.set()  # Only now may the original request reach real runtime.
        assert dispatched.wait(120) and not failures
        deadline = time.monotonic() + 120
        while store.get(job)['state'] == 'Publishing' and time.monotonic() < deadline:
            supervisor.tick()  # Observation only, never sends a second POST.
            time.sleep(0.1)
        result = store.query(job, 1)
        assert result['state'] == 'Ready' and store.get(job)['reserved'] == 0
        assert posts == [job] and not failures
        return {'scenario': 'delayed_original_publication_after_expiry_and_restart', 'build': job,
                'template': result['template']['templateId'], 'result': 'passed',
                'original_post_count': 1, 'expired_404_preserved_reservation': True,
                'restart_404_preserved_reservation': True, 'real_runtime_check_completed': True}
    finally:
        gate.set()
        if received.is_set():
            dispatched.wait(120)  # Never abandon the original queued side effect.
        proxy.shutdown()
        thread.join(timeout=5)
        proxy.server_close()
        if store is not None:
            store.close()
