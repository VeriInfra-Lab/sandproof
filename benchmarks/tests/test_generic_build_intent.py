"""Real HTTP/SQLite retry-domain regression; no worker, SDK create, or VM."""
import asyncio
import hashlib
import http.client
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest

from harbor.models.task.config import EnvironmentConfig
from harbor.models.trial.paths import TrialPaths

from benchmarks.generic_environment import GenericEnvironment
from tools.benchmark_tasks import InvalidInput
from tools.template_build_service import BuildHTTP, Handler
from tools.template_build_store import BuildStore, MIB


class RecordedHandler(Handler):
    def operation(self):
        code, result = super().operation()
        if self.command == 'POST' and self.path == '/sandproof/template-builds':
            row = self.server.store.get(result['buildID'])
            self.server.posts.append((row['id'], row['owner'], row['token']))
            self.server.accepted.set()
        if self.command == 'PUT':
            job = self.path.split('/')[3]
            row = self.server.store.get(job)
            context = self.server.store.directory(job) / 'source.tar'
            self.server.uploads.append((job, row['owner'], hashlib.sha256(context.read_bytes()).hexdigest()))
            # Stop polling after observing the real uploaded bytes. No worker
            # or runtime is dispatched, and the reservation remains retained.
            self.server.store.change(job, 'Failed', failure='SyntheticNoWorker')
            result = self.server.store.query(job, row['owner'])
            self.server.uploaded.set()
        return code, result

    def do_POST(self):
        if self.server.drop_first_post and self.path == '/sandproof/template-builds':
            self.server.drop_first_post = False
            self.operation()  # Real durable admission precedes the lost reply.
            self.close_connection = True
            self.request.shutdown(socket.SHUT_RDWR)
            return
        self.respond()


class GenericBuildIntentTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.root.chmod(0o700)
        directory = self.root / 'sample/environment'
        directory.mkdir(parents=True)
        dockerfile = b'FROM ubuntu:24.04\n# original synthetic environment\n'
        (directory / 'Dockerfile').write_bytes(dockerfile)
        (self.root / 'trial').mkdir(mode=0o700)
        self.entry = {'name': 'sample', 'source_path': 'sample', 'files_sha256': 'a' * 64,
            'requirements': {'cpus': 1, 'memory_mib': 256, 'disk_mib': 1024, 'build_timeout_sec': 30.0},
            'files': [{'path': 'environment/Dockerfile', 'role': 'environment', 'mode': '100644',
                       'bytes': len(dockerfile), 'sha256': hashlib.sha256(dockerfile).hexdigest()}]}
        self.first = self.server('store-a', drop=True)
        self.second = self.server('store-b')

    def server(self, name, *, drop=False):
        root = self.root / name
        root.mkdir(mode=0o700)
        limits = {'max_cpu': 2, 'max_memory_mib': 2048, 'max_disk_mib': 2048,
            'timeout_seconds': 30, 'max_jobs': 12, 'max_jobs_per_tenant': 10,
            'max_pending': 3, 'max_pending_per_tenant': 2,
            'workspace_bytes': 100000 * MIB, 'workspace_bytes_per_tenant': 50000 * MIB,
            'max_input_bytes': 65536, 'max_expanded_bytes': 128 * MIB, 'max_entries': 100,
            'max_log_bytes': 65536, 'builder_disk_bytes': 1024 * MIB,
            'runtime_cleanup_reserve_bytes': 256 * MIB, 'artifact_margin_bytes': 64 * MIB}
        store = BuildStore(root, limits, free_bytes=lambda: 1000000 * MIB)
        self.addCleanup(store.close)
        server = BuildHTTP(('127.0.0.1', 0), store,
            [{'tenant': 1, 'secret': 'synthetic-owner-1'}, {'tenant': 2, 'secret': 'synthetic-owner-2'}])
        server.RequestHandlerClass = RecordedHandler
        server.drop_first_post, server.posts, server.uploads = drop, [], []
        server.accepted, server.uploaded = threading.Event(), threading.Event()
        thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': 0.01}, daemon=True)
        thread.start()
        def close():
            server.shutdown()
            thread.join(2)
            server.server_close()
            self.assertFalse(thread.is_alive())
        self.addCleanup(close)
        return server

    def environment(self, server, key='synthetic-owner-1'):
        # Exercise the same constructor/start path as the normal runner, while
        # explicitly reusing its trial to invoke the adapter's recovery branch.
        return GenericEnvironment(entry=self.entry, source=self.root, source_kind='dockerfile',
            build_url='http://127.0.0.1:' + str(server.server_port), build_mode='legacy-unbounded',
            sdk_options={'api_key': key, 'retries': 0},
            environment_dir=self.root / 'sample/environment', environment_name='sample',
            session_id='sample', trial_paths=TrialPaths(self.root / 'trial'),
            task_env_config=EnvironmentConfig(cpus=1, memory_mb=256, storage_mb=1024,
                                             build_timeout_sec=30))

    def lost_submission(self):
        original = self.environment(self.first)
        with self.assertRaises(http.client.RemoteDisconnected):
            asyncio.run(original.start())
        self.assertTrue(self.first.accepted.wait(1))
        self.assertEqual(len(self.first.posts), 1)
        self.assertEqual(self.first.uploads, [])
        self.assertEqual(self.second.posts, [])
        path = self.root / 'trial/build-intent.json'
        saved = path.read_bytes()
        intent = json.loads(saved)
        self.assertEqual({key: intent[key] for key in original.client.intent_binding()},
                         original.client.intent_binding())
        self.assertNotIn('synthetic-owner-1', saved.decode())
        context = (self.root / 'trial/context.tar').read_bytes()
        return path, saved, intent, context

    def test_lost_reply_reentry_in_original_domain_reuses_one_identity(self):
        path, saved, intent, context = self.lost_submission()
        restored = self.environment(self.first)
        with self.assertRaisesRegex(RuntimeError, 'template build: SyntheticNoWorker'):
            asyncio.run(restored.start())
        self.assertTrue(self.first.uploaded.wait(1))
        original_id = self.first.posts[0][0]
        self.assertEqual(self.first.posts, [(original_id, 1, intent['idempotency_key'])] * 2)
        self.assertEqual(self.first.uploads, [(original_id, 1, hashlib.sha256(context).hexdigest())])
        self.assertEqual(restored.build_id, original_id)
        self.assertEqual(len(self.first.store.rows()), 1)
        self.assertEqual(self.second.store.rows(), [])
        self.assertEqual(path.read_bytes(), saved)

    def rejected_reentry(self, destination, key):
        path, saved, intent, context = self.lost_submission()
        original = self.first.store.rows()[0]
        restored = self.environment(destination, key)
        with self.assertRaisesRegex(InvalidInput, 'original build intent conflicts'):
            asyncio.run(restored.start())
        self.assertFalse(restored.build_dispatched)
        self.assertIsNone(restored.build_id)
        self.assertEqual(self.first.posts, [(original['id'], 1, intent['idempotency_key'])])
        self.assertEqual(self.first.uploads, [])
        self.assertEqual(self.first.store.rows(), [original])
        self.assertEqual(self.second.posts, [])
        self.assertEqual(self.second.uploads, [])
        self.assertEqual(self.second.store.rows(), [])
        self.assertEqual(path.read_bytes(), saved)
        self.assertEqual((self.root / 'trial/context.tar').read_bytes(), context)

    def test_lost_reply_reentry_with_changed_origin_never_posts_or_uploads(self):
        self.rejected_reentry(self.second, 'synthetic-owner-1')

    def test_lost_reply_reentry_with_changed_credential_never_posts_or_uploads(self):
        self.rejected_reentry(self.first, 'synthetic-owner-2')
