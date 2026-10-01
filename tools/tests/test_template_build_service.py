import io
import json
from pathlib import Path
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from unittest.mock import Mock, patch

from tools.template_build_store import BuildError, BuildStore, MIB, public_image
from tools.template_build_service import Backend, BuildHTTP, Supervisor
from tools.template_build_client import BuildClient, LOG_RESPONSE_BYTES


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
        return {"state": "candidate", "ready": False}

    def cleanup(self, row):
        if self.cleanup_known:
            self.vms.discard(row["id"])
        return self.cleanup_known

    def publish(self, store, row):
        assert row["id"] not in self.vms
        self.publications.add(row["id"])
        if self.lost_publish:
            self.lost_publish = False
            raise TimeoutError()
        return {"state": "ready", "template": {"templateId": "tpl-" + row["id"], "revision": "r1", "imageDigest": "sha256:" + "a" * 64}}


class BuildTests(unittest.TestCase):
    def test_private_create_uses_remaining_original_deadline_without_replacement(self):
        backend = object.__new__(Backend)
        backend.request = Mock()
        row = {'id': 'a' * 32, 'deadline': 1500,
               'request': json.dumps({'timeoutSeconds': 600, 'allowInternetAccess': True})}
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

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.root.chmod(0o700)
        self.now = 1000
        self.store = BuildStore(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        self.addCleanup(lambda: self.store.close())

    def job(self, owner=1, token=None):
        job = self.store.submit(owner, {"kind": "dockerfile"}, token)["buildID"]
        self.store.upload(job, owner, iter([b"opaque tar"]), 10)
        return job

    def publication(self):
        job = self.job()
        Supervisor(self.store, FakeBackend()).tick()
        self.assertEqual(self.store.get(job)['state'], 'Publishing')
        registry = self.root / 'registry'
        (registry / 'inbox').mkdir(parents=True, mode=0o700)
        backend = object.__new__(Backend)
        backend.config = {'runtime': {'template_registry': {'root': str(registry)},
                                      'kernel': str(self.root / 'kernel')}}
        backend.request = Mock(side_effect=BuildError(404, 'NotFound'))
        return job, backend, registry

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

    def test_staging_crossing_deadline_discards_only_unadmitted_candidate(self):
        job, backend, registry = self.publication()
        unrelated = registry / 'inbox' / ('f' * 32)
        unrelated.mkdir(mode=0o700)
        (unrelated / 'keep').write_bytes(b'unrelated')
        staged = registry / 'inbox' / job
        def stage(*args, **kwargs):
            self.assertEqual(kwargs['candidate_id'], job)
            staged.mkdir(mode=0o700)
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
            self.store = BuildStore(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
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

    def client(self, key="owner"):
        server = BuildHTTP(("127.0.0.1", 0), self.store,
                           [{"tenant": 1, "secret": "owner"}, {"tenant": 2, "secret": "other"}])
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
        first = self.store.submit(1, {"kind": "oci", "image": "ubuntu:24.04"}, "request")
        self.assertEqual(first, self.store.submit(1, {"kind": "oci", "image": "ubuntu:24.04"}, "request"))
        with self.assertRaisesRegex(BuildError, "IdempotencyConflict"):
            self.store.submit(1, {"kind": "oci", "image": "debian:12"}, "request")
        with self.assertRaisesRegex(BuildError, "NotFound"):
            self.store.query(first["buildID"], 2)
        for image in ("ubuntu\nRUN echo bad", "https://registry/repo", "localhost/x", "127.0.0.1/a", "user:password@registry/a", "registry.local/a", "registry:5000/a"):
            with self.subTest(image=image), self.assertRaises(BuildError):
                public_image(image)
        self.assertEqual(public_image("ubuntu:24.04"), "docker.io/library/ubuntu:24.04")
        self.assertEqual(public_image("ghcr.io/company/image@sha256:" + "a" * 64),
                         "ghcr.io/company/image@sha256:" + "a" * 64)
        self.assertEqual(first, self.store.submit(1, {"kind": "oci", "image": "docker.io/library/ubuntu:24.04"}, "request"))
        for fields in ({"diskMiB": 10000000}, {"cpuCount": True}, {"minFreeMiB": 1024}):
            with self.assertRaises(BuildError):
                self.store.submit(1, {"kind": "dockerfile", "resources": fields})
        self.job()
        with self.assertRaisesRegex(BuildError, "BuildBudgetExceeded"):
            self.job()

    def test_upload_interruption_no_queue_no_partial_and_retry(self):
        job = self.store.submit(1, {"kind": "dockerfile"})["buildID"]
        with self.assertRaisesRegex(BuildError, "Incomplete"):
            self.store.upload(job, 1, iter([b"short"]), 10)
        self.assertEqual(self.store.query(job, 1)["state"], "AwaitingInput")
        self.assertFalse((self.root / job / "source.partial").exists())
        self.store.upload(job, 1, iter([b"validinput"]), 10)
        with self.assertRaises(BuildError):
            self.store.upload(job, 1, iter([b"different!"]), 10)

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
        self.store = BuildStore(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
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
            BuildStore(self.root, limits())
        self.now += 301
        backend = FakeBackend()
        Supervisor(self.store, backend).tick()
        self.assertEqual(self.store.query(job, 1)["state"], "Failed")
        self.assertEqual(backend.builds, [])
        self.assertEqual(self.store.get(job)["reserved"], 0)

    def test_full_controller_owner_range_is_not_narrowed_by_sqlite(self):
        owner = 2**64 - 1
        job = self.store.submit(owner, {"kind": "oci", "image": "ubuntu:24.04"})["buildID"]
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
        history = limits()["max_log_bytes"] + 256 * 1024
        self.store.limits["workspace_bytes"] = cost + 2 * history - 1
        with self.assertRaisesRegex(BuildError, "BuildBudgetExceeded"):
            self.store.submit(2, {"kind": "dockerfile"})
        self.assertEqual(self.store.query(job, 1)["state"], "Ready")

    def test_candidate_checkpoint_restart_can_publish_without_rerun(self):
        job = self.job()
        backend = FakeBackend()
        self.store.change(job, "Building")
        status = backend.build(self.store, self.store.get(job))
        self.store.change(job, "Cleaning", candidate=status)
        self.store.close()
        self.store = BuildStore(self.root, limits(), clock=lambda: self.now, free_bytes=lambda: 1000000 * MIB)
        supervisor = Supervisor(self.store, backend)
        supervisor.tick()
        supervisor.tick()
        self.assertEqual(backend.builds, [job])
        self.assertEqual(self.store.query(job, 1)["state"], "Ready")

    def test_http_contract_upload_owner_and_no_host_path_input(self):
        server = BuildHTTP(("127.0.0.1", 0), self.store, [{"tenant": 1, "secret": "owner"}, {"tenant": 2, "secret": "other"}])
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            url = "http://127.0.0.1:" + str(server.server_port)
            def fetch(method, path, body=None, key="owner"):
                request = urllib.request.Request(url + path, data=body, method=method, headers={"X-API-Key": key})
                try:
                    with urllib.request.urlopen(request, timeout=5) as response:
                        return response.status, json.load(response)
                except urllib.error.HTTPError as error:
                    return error.code, json.load(error)
            code, result = fetch("POST", "/sandproof/template-builds", json.dumps({"kind": "dockerfile"}).encode())
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
