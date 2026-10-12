import io
import json
import os
from pathlib import Path
import sqlite3
import socket
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from unittest.mock import Mock, patch

from tools.template_build_store import BuildError, BuildStore, MIB, public_image
from tools.tests.template_build_fixture import new_store, PRIVATE_TEST_KEY, PrivateBuildClient as BuildClient
from tools.template_build_service import Backend, BuildHTTP, Handler, Supervisor
from tools.template_build_client import LOG_RESPONSE_BYTES, saved_intent
from tools.template_stage import stage_owner


def limits():
    return {"max_cpu": 2, "max_memory_mib": 2048, "max_disk_mib": 2048, "timeout_seconds": 300,
            "max_jobs": 12, "max_jobs_per_tenant": 10, "max_pending": 3, "max_pending_per_tenant": 2,
            "workspace_bytes": 100000 * MIB, "workspace_bytes_per_tenant": 50000 * MIB,
            "max_input_bytes": 65536, "max_expanded_bytes": 128 * MIB, "max_entries": 100,
            "max_log_bytes": 65536, "builder_disk_bytes": 1024 * MIB,
            "runtime_cleanup_reserve_bytes": 256 * MIB, "artifact_margin_bytes": 64 * MIB}


class PowerLoss(BaseException):
    pass


class FakeBackend:
    def __init__(self):
        self.builds, self.vms, self.publications = [], set(), set()
        self.cleanup_known, self.power_loss, self.fail, self.lost_publish = True, False, False, False

    def build(self, store, row):
        self.builds.append(row["id"])
        self.vms.add(row["id"])
        if self.power_loss:
            raise PowerLoss()
        if self.fail:
            raise BuildError(422, "WorkerFailed:RuntimeError")
        directory = store.directory(row["id"])
        (directory / "candidate.ext4").write_bytes(b"opaque")
        (directory / "build.log").write_bytes(b"first\nsecond\n")
        (directory / "candidate.ext4").chmod(0o600)
        (directory / "build.log").chmod(0o600)
        return {"state": "candidate", "ready": False}

    def cleanup(self, row, store):
        if self.cleanup_known:
            self.vms.discard(row["id"])
            store.private_checkpoint(row["id"], builder_settled={'requestWindow': None, 'sandboxID': None}, private_ack=1)
        return self.cleanup_known

    def publish(self, store, row):
        assert row["id"] not in self.vms
        self.publications.add(row["id"])
        if self.lost_publish:
            self.lost_publish = False
            raise TimeoutError()
        return {"state": "ready", "template": {"templateId": "tpl-" + row["id"], "revision": "r1", "imageDigest": "sha256:" + "a" * 64}}


class BuildTests(unittest.TestCase):
    def test_http_submission_requires_original_window_and_key(self):
        client = self.client()
        request = {'kind': 'oci', 'image': 'ubuntu:24.04', 'timeoutSeconds': 30}
        with self.assertRaisesRegex(RuntimeError, 'RequestWindowRequired'):
            client.request('POST', '/sandproof/template-builds', request, token='original')
        window = client.request('POST', '/sandproof/template-build-windows', {'durationMs': 30000})['requestWindow']
        with self.assertRaisesRegex(RuntimeError, 'IdempotencyKeyRequired'):
            client.request('POST', '/sandproof/template-builds', request | {'requestWindow': window})
        self.assertEqual(self.store.rows(), [])

    def test_client_cannot_choose_a_deployment_mirror(self):
        with self.assertRaises(BuildError):
            self.submit(1, {'kind': 'oci', 'image': 'ubuntu:24.04',
                                 'docker_hub_mirror': 'docker.1ms.run'}, 'mirror')
        self.assertEqual(self.store.db.execute('SELECT COUNT(*) FROM jobs').fetchone()[0], 0)

    def test_private_create_uses_remaining_original_deadline_without_replacement(self):
        backend = object.__new__(Backend)
        backend.request = Mock()
        row = {'id': 'a' * 32, 'deadline': 1500, 'owner': 1,
               'request': json.dumps({'timeoutSeconds': 600, 'allowInternetAccess': True}),
               'private_window': json.dumps('bw-original')}
        with patch('tools.template_build_service.time.time', return_value=1000):
            backend.vm(row, create=True)
        self.assertEqual(backend.request.call_args.kwargs['timeout'], 500)
        self.assertEqual(backend.request.call_args.args[2], '/sandproof/internal-build-vms/' + row['id'])
        with patch('tools.template_build_service.time.time', return_value=1500):
            with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'):
                backend.vm(row, create=True)
        self.assertEqual(backend.request.call_count, 1)
        backend.vm(row)
        self.assertEqual(backend.request.call_args.args[1], 'GET')
        self.assertEqual(backend.request.call_args.kwargs['timeout'], 60)

    def test_private_vm_request_without_original_window_never_dispatches(self):
        backend = object.__new__(Backend)
        backend.request = Mock()
        row = {'id': 'a' * 32, 'deadline': 1500,
               'request': json.dumps({'timeoutSeconds': 600, 'allowInternetAccess': True})}
        with patch('tools.template_build_service.time.time', return_value=1000):
            for create in (True, False):
                with self.subTest(create=create), self.assertRaisesRegex(BuildError, 'PrivateRequestWindowRequired'):
                    backend.vm(row, create=create)
        backend.request.assert_not_called()

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.auxiliary = Path(temporary.name).resolve()
        self.root = self.auxiliary / 'builds'
        self.root.mkdir(mode=0o700)
        self.requests = {}
        self.windows = {}
        self.next_request = 0
        self.now = 1000
        self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        self.addCleanup(lambda: self.store.close())

    def submit(self, owner, request, token=None):
        # The fixture preserves one exact window/key for retries; product code
        # never invents a replacement window or fills in a missing key.
        if token is None:
            token = 'fixture-' + str(self.next_request)
            self.next_request += 1
        original = (owner, token)
        if original not in self.requests:
            if owner not in self.windows:
                self.windows[owner] = self.store.open_window(owner, 600000)['requestWindow']
            self.requests[original] = self.windows[owner]
        return self.store.submit(owner, request, token, window=self.requests[original])

    def job(self, owner=1, token=None):
        job = self.submit(owner, {"kind": "dockerfile"}, token)["buildID"]
        self.store.upload(job, owner, iter([b"opaque tar"]), 10)
        return job

    def publication(self):
        job = self.job()
        Supervisor(self.store, FakeBackend()).tick()
        self.assertEqual(self.store.get(job)['state'], 'Publishing')
        registry = self.auxiliary / 'registry'
        (registry / 'inbox').mkdir(parents=True, mode=0o700)
        backend = object.__new__(Backend)
        backend.config = {'registry': {'root': str(registry)}, 'registry_owner_uid': os.geteuid(),
                          'runtime': {'kernel': str(self.auxiliary / 'kernel')}}
        backend.request = Mock(side_effect=BuildError(404, 'NotFound'))
        return job, backend, registry

    def owned_stage(self, path, job):
        path.mkdir(mode=0o700)
        stage_owner(path, {'id': job, 'nonce': self.store.get(job)['fs_nonce']})

    def test_candidate_expiring_during_unknown_cleanup_never_starts_publication(self):
        job = self.job()
        backend = FakeBackend()
        backend.cleanup_known = False
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        self.now = self.store.get(job)['deadline']
        supervisor.tick()
        self.assertEqual(self.store.get(job)['state'], 'CleanupUnknown')
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertTrue((self.root / job / 'candidate.ext4').exists())
        backend.cleanup_known = True
        supervisor.tick()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.assertEqual(self.store.get(job)['failure'], 'BuildDeadlineExceeded')
        self.assertEqual(self.store.get(job)['reserved'], 0)
        self.assertEqual(backend.publications, set())
        self.assertFalse(backend.vms)

    def test_expired_absent_publication_never_stages_or_posts(self):
        job, backend, _ = self.publication()
        self.now = self.store.get(job)['deadline']
        with patch('tools.template_build_service.stage') as stage:
            with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'):
                backend.publish(self.store, self.store.get(job))
        stage.assert_not_called()
        backend.request.assert_called_once_with('runtime', 'GET',
            '/sandproof/template-publications/' + job, owner=1)

    def test_expired_unadmitted_handoff_removes_only_its_pending_identity(self):
        job, backend, registry = self.publication()
        pending = registry / 'inbox' / ('.stage-' + job)
        self.owned_stage(pending, job)
        (self.root / job / 'candidate.ext4').rename(pending / 'rootfs.ext4')
        unrelated = registry / 'inbox' / ('.stage-' + 'f' * 32)
        unrelated.mkdir(mode=0o700)
        (unrelated / 'keep').write_bytes(b'other candidate')
        self.now = self.store.get(job)['deadline']
        with patch('tools.template_build_service.stage') as stage:
            Supervisor(self.store, backend).tick()
        stage.assert_not_called()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.assertEqual(self.store.get(job)['failure'], 'BuildDeadlineExceeded')
        self.assertEqual(self.store.get(job)['reserved'], 0)
        self.assertFalse(pending.exists())
        self.assertEqual((unrelated / 'keep').read_bytes(), b'other candidate')

    def test_unknown_publication_does_not_discard_pending_handoff(self):
        job, backend, registry = self.publication()
        pending = registry / 'inbox' / ('.stage-' + job)
        self.owned_stage(pending, job)
        (pending / 'rootfs.ext4').write_bytes(b'keep until admission is known')
        backend.request.side_effect = TimeoutError('unknown publication')
        with self.assertRaises(TimeoutError):
            backend.discard_unpublished(self.store, self.store.get(job))
        self.assertTrue(pending.exists())

    def test_staging_crossing_deadline_discards_only_unadmitted_candidate(self):
        job, backend, registry = self.publication()
        unrelated = registry / 'inbox' / ('f' * 32)
        unrelated.mkdir(mode=0o700)
        (unrelated / 'keep').write_bytes(b'unrelated')
        staged = registry / 'inbox' / job
        def stage(*args, **kwargs):
            self.assertEqual(kwargs['candidate_id'], job)
            self.owned_stage(staged, job)
            (staged / 'rootfs.ext4').write_bytes(b'opaque')
            self.now = self.store.get(job)['deadline']
            return job
        with patch('tools.template_build_service.stage', side_effect=stage):
            Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.assertEqual(self.store.get(job)['failure'], 'BuildDeadlineExceeded')
        self.assertEqual(self.store.get(job)['reserved'], 0)
        self.assertFalse(staged.exists())
        self.assertFalse((self.root / job / 'candidate.ext4').exists())
        self.assertEqual((unrelated / 'keep').read_bytes(), b'unrelated')
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'GET'])

    def test_publication_post_uses_deadline_remaining_after_staging(self):
        job, backend, _ = self.publication()
        backend.request.side_effect = [BuildError(404, 'NotFound'), {'state': 'checking'}]
        def stage(*args, **kwargs):
            self.now += 20
            return job
        with patch('tools.template_build_service.stage', side_effect=stage):
            result = backend.publish(self.store, self.store.get(job))
        self.assertEqual(result, {'state': 'checking'})
        backend.request.assert_called_with('runtime', 'POST', '/sandproof/template-publications',
            {'candidateID': job}, owner=1, timeout=280)

    def test_lost_publication_post_recovers_existing_identity_after_deadline_and_restart(self):
        job, backend, _ = self.publication()
        accepted, ready = False, False
        def request(pool, method, path, body=None, **kwargs):
            nonlocal accepted
            if method == 'GET':
                self.assertEqual(path, '/sandproof/template-publications/' + job)
                if not accepted:
                    raise BuildError(404, 'NotFound')
                return {'state': 'ready', 'template': {'templateId': 'tpl-' + job}} if ready else {'state': 'checking'}
            self.assertFalse(accepted)
            self.assertLess(self.now, self.store.get(job)['deadline'])
            self.assertEqual(body, {'candidateID': job})
            accepted = True
            self.now = self.store.get(job)['deadline'] + 1
            raise TimeoutError('lost publication reply')
        backend.request.side_effect = request
        with patch('tools.template_build_service.stage', return_value=job) as stage:
            Supervisor(self.store, backend).tick()
            self.assertEqual(self.store.get(job)['state'], 'Publishing')
            self.assertGreater(self.store.get(job)['reserved'], 0)
            self.store.close()
            self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
            supervisor = Supervisor(self.store, backend)
            supervisor.tick()
            self.assertEqual(self.store.get(job)['state'], 'Publishing')
            self.assertGreater(self.store.get(job)['reserved'], 0)
            ready = True
            supervisor.tick()
            self.assertEqual(self.store.get(job)['state'], 'Ready')
            self.assertEqual(self.store.get(job)['reserved'], 0)
            stage.assert_called_once()
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'POST', 'GET', 'GET'])

    def test_expired_unknown_publication_keeps_reservation_and_never_posts(self):
        job, backend, _ = self.publication()
        self.now = self.store.get(job)['deadline']
        for error in (TimeoutError(), BuildError(502, 'Unknown'), BuildError(408, 'ControllerRejectedOrUnknown')):
            with self.subTest(error=type(error).__name__), patch('tools.template_build_service.stage') as stage:
                backend.request.reset_mock(side_effect=True)
                backend.request.side_effect = error
                Supervisor(self.store, backend).tick()
                self.assertEqual(self.store.get(job)['state'], 'Publishing')
                self.assertGreater(self.store.get(job)['reserved'], 0)
                self.assertTrue((self.root / job / 'candidate.ext4').exists())
                stage.assert_not_called()
                self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET'])

    def test_delayed_publication_admission_after_404_keeps_intent_across_restart(self):
        job, backend, registry = self.publication()
        staged = registry / 'inbox' / job
        (registry / 'artifacts').mkdir(mode=0o700)
        accepted, admit_after_get = False, False
        reserved = self.store.get(job)['reserved']
        def request(pool, method, path, body=None, **kwargs):
            nonlocal accepted
            if method == 'POST':
                # An independent connection must see committed intent before
                # runtime can receive the call, not merely a mutated row.
                with sqlite3.connect(self.root / 'builds.db') as db:
                    self.assertEqual(db.execute('SELECT publication_attempted FROM jobs WHERE id=?', (job,)).fetchone()[0], 1)
                raise TimeoutError('request queued before admission')
            if accepted:
                return {'state': 'ready', 'template': {'templateId': 'tpl-' + job}}
            if admit_after_get:
                # The observation returned absence before runtime got the
                # admission lock. Admission/sealing then races its delivery.
                staged.rename(registry / 'artifacts' / job)
                accepted = True
            raise BuildError(404, 'NotFound')
        backend.request.side_effect = request
        def stage(*args, **kwargs):
            self.owned_stage(staged, job)
            (staged / 'rootfs.ext4').write_bytes(b'opaque')
            return job
        with patch('tools.template_build_service.stage', side_effect=stage) as staging:
            supervisor = Supervisor(self.store, backend)
            supervisor.tick()
            supervisor.tick()  # 404 before expiry must not send a second POST.
            self.assertTrue(staged.exists())
            self.now = self.store.get(job)['deadline']
            self.store.close()
            self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
            supervisor = Supervisor(self.store, backend)
            supervisor.tick()  # 404 after expiry/restart is still not a fence.
            self.assertTrue(staged.exists())
            self.assertEqual(self.store.get(job)['reserved'], reserved)
            admit_after_get = True
            supervisor.tick()
            self.assertTrue(accepted)
            self.assertTrue((registry / 'artifacts' / job).exists())
            self.assertEqual(self.store.get(job)['state'], 'Publishing')
            self.assertEqual(self.store.get(job)['reserved'], reserved)
            self.assertTrue((self.root / job / 'candidate.ext4').exists())
            supervisor.tick()
            self.assertEqual(self.store.get(job)['state'], 'Ready')
            self.assertEqual(self.store.get(job)['reserved'], 0)
            staging.assert_called_once()
        self.assertEqual([call.args[1] for call in backend.request.call_args_list],
                         ['GET', 'POST', 'GET', 'GET', 'GET', 'GET'])

    def test_restart_after_publication_intent_before_post_is_observation_only(self):
        job, backend, _ = self.publication()
        begin = self.store.begin_publication
        def crash_after_intent(job):
            begin(job)
            raise PowerLoss()
        with patch('tools.template_build_service.stage', return_value=job), \
                patch.object(self.store, 'begin_publication', side_effect=crash_after_intent):
            with self.assertRaises(PowerLoss):
                Supervisor(self.store, backend).tick()
        self.store.close()
        self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        self.now = self.store.get(job)['deadline']
        with patch('tools.template_build_service.stage') as staging:
            Supervisor(self.store, backend).tick()
            staging.assert_not_called()
        self.assertEqual(self.store.get(job)['state'], 'Publishing')
        self.assertEqual(self.store.get(job)['publication_attempted'], 1)
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertTrue((self.root / job / 'candidate.ext4').exists())
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'GET'])

    def test_restart_before_publication_intent_can_discard_expired_staging(self):
        job, backend, registry = self.publication()
        staged = registry / 'inbox' / job
        def crash_during_stage(*args, **kwargs):
            self.owned_stage(staged, job)
            raise PowerLoss()
        with patch('tools.template_build_service.stage', side_effect=crash_during_stage):
            with self.assertRaises(PowerLoss):
                Supervisor(self.store, backend).tick()
        self.store.close()
        self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        self.assertEqual(self.store.get(job)['publication_attempted'], 0)
        self.now = self.store.get(job)['deadline']
        Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.assertEqual(self.store.get(job)['failure'], 'BuildDeadlineExceeded')
        self.assertEqual(self.store.get(job)['reserved'], 0)
        self.assertFalse(staged.exists())
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'GET', 'GET'])

    def test_original_post_refusal_can_clear_intent_and_discard(self):
        for code in (400, 409, 429):
            with self.subTest(code=code):
                job, backend, registry = self.publication()
                staged = registry / 'inbox' / job
                def stage(*args, **kwargs):
                    self.owned_stage(staged, job)
                    return job
                backend.request.side_effect = [BuildError(404, 'NotFound'),
                    BuildError(code, 'ControllerRejectedOrUnknown'), BuildError(404, 'NotFound')]
                with patch('tools.template_build_service.stage', side_effect=stage):
                    Supervisor(self.store, backend).tick()
                self.assertEqual(self.store.get(job)['state'], 'Failed')
                self.assertEqual(self.store.get(job)['failure'], 'PublicationRejected')
                self.assertEqual(self.store.get(job)['publication_attempted'], 0)
                self.assertEqual(self.store.get(job)['reserved'], 0)
                self.assertFalse(staged.exists())
                self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'POST', 'GET'])
                (registry / 'inbox').rmdir()

    def test_unknown_post_then_get_refusal_cannot_clear_intent(self):
        job, backend, registry = self.publication()
        staged = registry / 'inbox' / job
        self.owned_stage(staged, job)
        backend.request.side_effect = [BuildError(404, 'NotFound'), TimeoutError()]
        with patch('tools.template_build_service.stage', return_value=job):
            supervisor = Supervisor(self.store, backend)
            supervisor.tick()
        self.now = self.store.get(job)['deadline']
        for code in (400, 409, 429, 408):
            with self.subTest(code=code), patch('tools.template_build_service.stage') as staging:
                backend.request.reset_mock(side_effect=True)
                backend.request.side_effect = BuildError(code, 'ControllerRejectedOrUnknown')
                supervisor.tick()
                self.assertEqual(self.store.get(job)['state'], 'Publishing')
                self.assertEqual(self.store.get(job)['publication_attempted'], 1)
                self.assertGreater(self.store.get(job)['reserved'], 0)
                self.assertTrue(staged.exists())
                self.assertTrue((self.root / job / 'candidate.ext4').exists())
                staging.assert_not_called()
                self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET'])

    def test_post_408_remains_unknown_and_cannot_discard_stale_row(self):
        job, backend, _ = self.publication()
        stale = self.store.get(job)
        backend.request.side_effect = [BuildError(404, 'NotFound'), BuildError(408, 'BuildDeadlineExceeded')]
        with patch('tools.template_build_service.stage', return_value=job):
            Supervisor(self.store, backend).tick()
        self.assertEqual(stale['publication_attempted'], 0)
        self.assertFalse(backend.discard_unpublished(self.store, stale))
        self.assertEqual(self.store.get(job)['state'], 'Publishing')
        self.assertEqual(self.store.get(job)['publication_attempted'], 1)
        self.assertGreater(self.store.get(job)['reserved'], 0)
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'POST'])

    def test_deadline_expiring_during_intent_commit_never_posts(self):
        job, backend, _ = self.publication()
        begin = self.store.begin_publication
        def expire_during_intent(job):
            begin(job)
            self.now = self.store.get(job)['deadline']
        with patch('tools.template_build_service.stage', return_value=job), \
                patch.object(self.store, 'begin_publication', side_effect=expire_during_intent):
            Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.get(job)['state'], 'Failed')
        self.assertEqual(self.store.get(job)['failure'], 'BuildDeadlineExceeded')
        self.assertEqual(self.store.get(job)['publication_attempted'], 0)
        self.assertEqual(self.store.get(job)['reserved'], 0)
        self.assertEqual([call.args[1] for call in backend.request.call_args_list], ['GET', 'GET'])

    def client(self, key="owner"):
        server = BuildHTTP(("127.0.0.1", 0), self.store, PRIVATE_TEST_KEY)
        self.server = server
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        def close():
            server.shutdown()
            server.server_close()
            thread.join()
        self.addCleanup(close)
        return BuildClient("http://127.0.0.1:" + str(server.server_port), key)

    def test_client_reads_full_raw_log_page_with_bounded_json_expansion(self):
        job = self.job()
        client = self.client()
        path = "/sandproof/template-builds/" + job + "/logs"
        for payload in (b"x" * 65536, b"\x00" * 65536, b"\xff" * 65536,
                        "\U0001f600".encode() * 16384):
            with self.subTest(prefix=payload[:4]):
                (self.root / job / "build.log").write_bytes(payload)
                page = client.request("GET", path + "?offset=0&limit=65536")
                self.assertEqual(page["text"], payload.decode("utf-8", errors="replace"))
                self.assertEqual((page["offset"], page["nextOffset"]), (0, 65536))
                self.assertEqual(client.request("GET", path + "?offset=65536&limit=1")["text"], "")
        # The generic runner's smaller page can also exceed 64 KiB on wire.
        (self.root / job / "build.log").write_bytes(b"\x00" * 16384)
        page = client.request("GET", path + "?offset=0&limit=16384")
        self.assertEqual(page["text"], "\x00" * 16384)
        self.assertEqual(page["nextOffset"], 16384)

    def test_client_log_allowance_preserves_owner_and_response_limits(self):
        job = self.job()
        client = self.client()
        path = "/sandproof/template-builds/" + job
        (self.root / job / "build.log").write_bytes(b"private build material")
        with self.assertRaisesRegex(RuntimeError, "NotFound"):
            self.client("other").request("GET", path + "/logs")
        with self.assertRaisesRegex(RuntimeError, "InvalidLogRange"):
            client.request("GET", path + "/logs?limit=65537")
        with patch.object(self.store, "query", return_value={"text": "x" * 65536}) as query:
            with self.assertRaisesRegex(RuntimeError, "oversized build response"):
                client.request("GET", path)
            query.assert_called_once()
        with patch.object(self.store, "logs", return_value={"text": "x" * LOG_RESPONSE_BYTES}) as logs:
            with self.assertRaisesRegex(RuntimeError, "oversized build response"):
                client.request("GET", path + "/logs")
            logs.assert_called_once()

    def test_request_identity_owner_and_budget_boundaries(self):
        first = self.submit(1, {"kind": "oci", "image": "ubuntu:24.04"}, "request")
        self.assertEqual(first, self.submit(1, {"kind": "oci", "image": "ubuntu:24.04"}, "request"))
        with self.assertRaisesRegex(BuildError, "IdempotencyConflict"):
            self.submit(1, {"kind": "oci", "image": "debian:12"}, "request")
        with self.assertRaisesRegex(BuildError, "NotFound"):
            self.store.query(first["buildID"], 2)
        for image in ("ubuntu\nRUN echo bad", "https://registry/repo", "localhost/x", "127.0.0.1/a", "user:password@registry/a", "registry.local/a", "registry:5000/a"):
            with self.subTest(image=image), self.assertRaises(BuildError):
                public_image(image)
        self.assertEqual(public_image("ubuntu:24.04"), "docker.io/library/ubuntu:24.04")
        self.assertEqual(public_image("ghcr.io/company/image@sha256:" + "a" * 64),
                         "ghcr.io/company/image@sha256:" + "a" * 64)
        self.assertEqual(first, self.submit(1, {"kind": "oci", "image": "docker.io/library/ubuntu:24.04"}, "request"))
        for fields in ({"diskMiB": 10000000}, {"cpuCount": True}, {"minFreeMiB": 1024}):
            with self.assertRaises(BuildError):
                self.submit(1, {"kind": "dockerfile", "resources": fields})
        self.job()
        with self.assertRaisesRegex(BuildError, "BuildBudgetExceeded"):
            self.job()

    def test_upload_interruption_no_queue_no_partial_and_retry(self):
        job = self.submit(1, {"kind": "dockerfile"})["buildID"]
        with self.assertRaisesRegex(BuildError, "Incomplete"):
            self.store.upload(job, 1, iter([b"short"]), 10)
        self.assertEqual(self.store.query(job, 1)["state"], "AwaitingInput")
        self.assertFalse((self.root / job / "source.partial").exists())
        self.store.upload(job, 1, iter([b"validinput"]), 10)
        with self.assertRaises(BuildError):
            self.store.upload(job, 1, iter([b"different!"]), 10)

    def test_upload_rejects_late_final_block_and_eof(self):
        for boundary in ("block", "eof"):
            with self.subTest(boundary=boundary):
                job = self.submit(1, {"kind": "dockerfile", "timeoutSeconds": 1})["buildID"]
                deadline = self.store.get(job)["deadline"]
                def blocks():
                    if boundary == "block":
                        self.now = deadline
                    yield b"input"
                    self.now = deadline
                with self.assertRaisesRegex(BuildError, "BuildDeadlineExceeded") as failure:
                    self.store.upload(job, 1, blocks(), 5)
                self.assertEqual(failure.exception.code, 408)
                self.assertEqual(self.store.get(job)["state"], "AwaitingInput")
                self.assertNotIn(job, self.store.uploads)
                self.assertFalse((self.store.directory(job) / "source.tar").exists())
                self.assertFalse((self.store.directory(job) / "source.partial").exists())
                Supervisor(self.store, FakeBackend()).tick()
                self.assertEqual(self.store.get(job)["state"], "Failed")
                self.assertEqual(self.store.get(job)["reserved"], 0)

    def test_upload_original_deadline_covers_file_and_directory_fsync(self):
        for boundary in ("file", "directory"):
            with self.subTest(boundary=boundary):
                job = self.submit(1, {"kind": "dockerfile", "timeoutSeconds": 1})["buildID"]
                deadline = self.store.get(job)["deadline"]
                import os
                import stat
                actual_fsync = os.fsync
                def delayed(fd):
                    actual_fsync(fd)
                    is_directory = stat.S_ISDIR(os.fstat(fd).st_mode)
                    if is_directory == (boundary == "directory"):
                        self.now = deadline
                with patch("tools.template_build_store.os.fsync", side_effect=delayed):
                    with self.assertRaisesRegex(BuildError, "BuildDeadlineExceeded"):
                        self.store.upload(job, 1, [b"input"], 5)
                self.assertEqual(self.store.get(job)["state"], "AwaitingInput")
                self.assertNotIn(job, self.store.uploads)
                self.assertFalse((self.store.directory(job) / "source.tar").exists())
                self.assertFalse((self.store.directory(job) / "source.partial").exists())
                Supervisor(self.store, FakeBackend()).tick()

    def test_upload_checks_original_deadline_immediately_before_commit(self):
        job = self.submit(1, {"kind": "dockerfile", "timeoutSeconds": 1})["buildID"]
        def expire(point):
            if point == "before_commit":
                self.now = self.store.get(job)["deadline"]
        self.store.fault = expire
        with self.assertRaisesRegex(BuildError, "BuildDeadlineExceeded"):
            self.store.upload(job, 1, [b"input"], 5)
        self.store.fault = lambda _point: None
        self.assertEqual(self.store.get(job)["state"], "AwaitingInput")
        self.assertNotIn(job, self.store.uploads)
        self.assertFalse((self.store.directory(job) / "source.tar").exists())

    def test_upload_lost_committed_reply_preserves_original_input(self):
        job = self.submit(1, {"kind": "dockerfile"})["buildID"]
        def lost(point):
            if point == "after_commit":
                raise OSError("lost commit reply")
        self.store.fault = lost
        with self.assertRaises(OSError):
            self.store.upload(job, 1, [b"input"], 5)
        self.store.fault = lambda _point: None
        self.assertEqual(self.store.get(job)["state"], "Queued")
        self.assertEqual((self.store.directory(job) / "source.tar").read_bytes(), b"input")
        self.assertNotIn(job, self.store.uploads)

    def test_upload_deadline_cleanup_keeps_replaced_partial_pinned(self):
        job = self.submit(1, {"kind": "dockerfile", "timeoutSeconds": 1})["buildID"]
        partial = self.store.directory(job) / "source.partial"
        displaced = self.auxiliary / "original-upload-inode"
        def blocks():
            yield b"input"
            # Isolate the replacement-inode fault from an unrelated unknown
            # entry in the strictly inventoried public workspace root.
            partial.rename(displaced)
            partial.write_bytes(b"foreign")
            partial.chmod(0o600)
            self.now = self.store.get(job)["deadline"]
        with self.assertRaisesRegex(BuildError, "InputCleanupIdentityUnknown"):
            self.store.upload(job, 1, blocks(), 5)
        self.assertNotIn(job, self.store.uploads)
        self.assertEqual(self.store.get(job)["state"], "CleanupUnknown")
        self.assertGreater(self.store.get(job)["reserved"], 0)
        Supervisor(self.store, FakeBackend()).tick()
        self.assertEqual(partial.read_bytes(), b"foreign")
        self.assertEqual(displaced.read_bytes(), b"input")
        self.assertGreater(self.store.get(job)["reserved"], 0)

    def test_http_slow_fragments_return_to_original_deadline(self):
        job = self.submit(1, {"kind": "dockerfile", "timeoutSeconds": 1})["buildID"]
        deadline = self.store.get(job)["deadline"]
        first_fragment = threading.Event()
        reads = []
        testcase = self
        class FragmentHandler(Handler):
            def setup(self):
                super().setup()
                incoming = self.rfile
                class Fragments:
                    def read1(self, size):
                        block = incoming.read1(size)
                        if block:
                            reads.append(block)
                            testcase.now = deadline - .25 if len(reads) == 1 else deadline
                            first_fragment.set()
                        return block
                    def __getattr__(self, name):
                        return getattr(incoming, name)
                self.rfile = Fragments()
        server = BuildHTTP(("127.0.0.1", 0), self.store, PRIVATE_TEST_KEY)
        server.RequestHandlerClass = FragmentHandler
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with socket.create_connection(server.server_address, timeout=2) as connection:
                connection.sendall(("PUT /sandproof/template-builds/" + job + "/context HTTP/1.0\r\n"
                    f"x-sandproof-build-key: {PRIVATE_TEST_KEY}\r\nx-sandproof-build-owner: 1\r\nContent-Length: 2\r\n\r\n").encode() + b"a")
                self.assertTrue(first_fragment.wait(1), "buffered read hid the first fragment")
                connection.sendall(b"b")
                from http.client import HTTPResponse
                with HTTPResponse(connection) as response:
                    response.begin()
                    self.assertEqual(response.status, 408)
                    self.assertEqual(json.load(response), {"error": "BuildDeadlineExceeded"})
            self.assertEqual(reads, [b"a", b"b"])
            self.assertEqual(self.store.get(job)["state"], "AwaitingInput")
            self.assertNotIn(job, self.store.uploads)
            self.assertFalse((self.store.directory(job) / "source.tar").exists())
            self.assertFalse((self.store.directory(job) / "source.partial").exists())
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_http_blocked_upload_wait_uses_original_remaining_deadline(self):
        job = self.submit(1, {"kind": "dockerfile"})["buildID"]
        self.store.clock = time.monotonic
        deadline = time.monotonic() + .2
        self.store.db.execute("UPDATE jobs SET deadline=? WHERE id=?", (deadline, job))
        self.store.db.commit()
        server = BuildHTTP(("127.0.0.1", 0), self.store, PRIVATE_TEST_KEY)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with socket.create_connection(server.server_address, timeout=2) as connection:
                connection.sendall(("PUT /sandproof/template-builds/" + job + "/context HTTP/1.0\r\n"
                    f"x-sandproof-build-key: {PRIVATE_TEST_KEY}\r\nx-sandproof-build-owner: 1\r\nContent-Length: 2\r\n\r\n").encode() + b"a")
                from http.client import HTTPResponse
                with HTTPResponse(connection) as response:
                    response.begin()
                    self.assertEqual(response.status, 408)
                    self.assertEqual(json.load(response), {"error": "BuildDeadlineExceeded"})
            self.assertEqual(self.store.get(job)["state"], "AwaitingInput")
            self.assertNotIn(job, self.store.uploads)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_success_reclaim_before_publish_and_bounded_log_offsets(self):
        job = self.job()
        backend = FakeBackend()
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        self.assertEqual(self.store.query(job, 1)["state"], "Publishing")
        self.assertFalse(backend.vms)
        supervisor.tick()
        view = self.store.query(job, 1)
        self.assertEqual(view["state"], "Ready")
        self.assertEqual(view["template"]["templateId"], "tpl-" + job)
        self.assertEqual(self.store.get(job)["reserved"], 0)
        self.assertFalse((self.root / job / "candidate.ext4").exists())
        self.assertEqual(self.store.logs(job, 1, 0, 6)["text"], "first\n")
        self.assertEqual(self.store.logs(job, 1, 6, 65536)["text"], "second\n")
        with self.assertRaises(BuildError):
            self.store.logs(job, 2, 0, 1)

    def test_real_journal_reopen_after_started_vm_lost_receipt_no_rebuild(self):
        job = self.job()
        backend = FakeBackend()
        backend.power_loss = True
        with self.assertRaises(PowerLoss):
            Supervisor(self.store, backend).tick()
        self.store.close()
        self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        backend.power_loss = False
        Supervisor(self.store, backend).tick()
        self.assertEqual(backend.builds, [job])
        self.assertFalse(backend.vms)
        self.assertEqual(self.store.query(job, 1)["failure"], "InterruptedBuild")
        self.assertEqual(self.store.query(job, 1)["state"], "Failed")

    def test_unknown_cleanup_retains_reservation_then_reclaims_exact_job(self):
        job = self.job()
        backend = FakeBackend()
        backend.fail, backend.cleanup_known = True, False
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        self.assertEqual(self.store.query(job, 1)["state"], "CleanupUnknown")
        self.assertGreater(self.store.get(job)["reserved"], 0)
        backend.cleanup_known = True
        supervisor.tick()
        self.assertEqual(self.store.query(job, 1)["state"], "Failed")
        self.assertEqual(self.store.get(job)["reserved"], 0)
        self.assertEqual(backend.builds, [job])

    def test_lost_publication_retries_same_candidate_not_build(self):
        job = self.job()
        backend = FakeBackend()
        backend.lost_publish = True
        supervisor = Supervisor(self.store, backend)
        for _ in range(3):
            supervisor.tick()
        self.assertEqual(backend.builds, [job])
        self.assertEqual(backend.publications, {job})
        self.assertEqual(self.store.query(job, 1)["state"], "Ready")

    def test_original_deadline_and_single_process_lease(self):
        job = self.job()
        with self.assertRaisesRegex(BuildError, "AlreadyOpen"):
            new_store(self.root, limits())
        self.now += 301
        backend = FakeBackend()
        Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.query(job, 1)["state"], "Failed")
        self.assertEqual(backend.builds, [])
        self.assertEqual(self.store.get(job)["reserved"], 0)

    def test_full_controller_owner_range_is_not_narrowed_by_sqlite(self):
        owner = 2**64 - 1
        job = self.submit(owner, {"kind": "oci", "image": "ubuntu:24.04"})["buildID"]
        self.assertEqual(self.store.get(job, owner)["owner"], owner)
        with self.assertRaises(BuildError):
            self.store.query(job, owner - 1)

    def test_retained_logs_still_charge_workspace_after_vm_budget_release(self):
        job = self.job()
        backend = FakeBackend()
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        supervisor.tick()
        cost = (limits()["builder_disk_bytes"] + limits()["max_input_bytes"]
                + 2 * 1024 * MIB + limits()["artifact_margin_bytes"])
        history = limits()["max_log_bytes"] + self.store.bounded["max_record_bytes"]
        self.store.limits["workspace_bytes"] = cost + 2 * history - 1
        with self.assertRaisesRegex(BuildError, "BuildBudgetExceeded"):
            self.submit(2, {"kind": "dockerfile"})
        self.assertEqual(self.store.query(job, 1)["state"], "Ready")

    def test_candidate_checkpoint_restart_can_publish_without_rerun(self):
        job = self.job()
        backend = FakeBackend()
        self.store.change(job, "Building")
        status = backend.build(self.store, self.store.get(job))
        self.store.change(job, "Cleaning", candidate=status)
        self.store.close()
        self.store = new_store(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        supervisor.tick()
        self.assertEqual(backend.builds, [job])
        self.assertEqual(self.store.query(job, 1)["state"], "Ready")

    def test_http_contract_upload_owner_and_no_host_path_input(self):
        server = BuildHTTP(("127.0.0.1", 0), self.store, PRIVATE_TEST_KEY)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = "http://127.0.0.1:" + str(server.server_port)
            def fetch(method, path, body=None, key="owner", token=None):
                headers = BuildClient(url, key).headers()
                if token is not None:
                    headers["Idempotency-Key"] = token
                request = urllib.request.Request(url + path, data=body, method=method, headers=headers)
                try:
                    with urllib.request.urlopen(request, timeout=5) as response:
                        return response.status, json.load(response)
                except urllib.error.HTTPError as error:
                    return error.code, json.load(error)
            code, opened = fetch("POST", "/sandproof/template-build-windows", json.dumps({"durationMs": 300000}).encode())
            self.assertEqual(code, 201)
            code, result = fetch("POST", "/sandproof/template-builds", json.dumps({"kind": "dockerfile", "requestWindow": opened["requestWindow"]}).encode(), token="original")
            self.assertEqual(code, 202)
            path = "/sandproof/template-builds/" + result["buildID"]
            self.assertEqual(fetch("PUT", path + "/context", b"opaque archive")[0], 200)
            self.assertEqual(fetch("GET", path, key="other")[0], 404)
            self.assertEqual(fetch("GET", path, key="wrong")[0], 401)
            self.assertEqual(fetch("POST", "/sandproof/template-builds", b'{"kind":"dockerfile","hostPath":"/etc"}')[0], 400)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
