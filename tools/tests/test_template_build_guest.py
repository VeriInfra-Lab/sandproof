"""Protected supervisor inputs and actual bounded file reads; no VM startup."""
import asyncio
import copy
import hashlib
import json
import os
from pathlib import Path
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from tools import template_build_guest as guest
from tools.template_build_config import validate_capacity, MIB
from tools.template_build_service import Backend, configuration
from tools.template_build_store import BuildError, BuildStore
from tools.template_build_upload import PinnedReader, upload
from tools.template_guest import load_profile, load_runtime_profile
from tools.tests.test_template_guest import GuestFixture
from tools.tests.test_bounded_acceptance import service_fixture, inputs


class ProtectedBuildGuestTests(GuestFixture):
    def test_runtime_payload_omits_archive_but_host_audit_still_requires_it(self):
        expected = load_profile(self.profile_path)
        (self.release / 'source.tar').unlink()
        self.assertEqual(load_runtime_profile(self.profile_path), expected)
        with self.assertRaises(FileNotFoundError): load_profile(self.profile_path)
        binary = self.release / 'sandproof-guest-worker'
        binary.chmod(0o600)
        binary.write_bytes(b'changed')
        with self.assertRaises(ValueError): load_runtime_profile(self.profile_path)

    def test_same_profile_path_with_new_pin_changes_original_configuration_binding(self):
        root = self.work / 'supervisor'
        root.mkdir(mode=0o700)
        path, _, _, _ = service_fixture(root)
        config = json.loads(path.read_bytes())
        config['rust_guest_profile'] = str(self.profile_path)
        del config['tools']['envd']
        path.write_text(json.dumps(config))
        first = configuration(path, require_physical_lease=False)
        self.assertEqual(first['rust_guest'], guest.snapshot(self.profile_path))
        self.config['runtime']['worker_threads'] += 1
        self.save_profile()
        second = configuration(path, require_physical_lease=False)
        self.assertNotEqual(first['binding'], second['binding'])
        # Exercise the original SQLite configuration guard without running a
        # reducer or activating a physical pool; there is no second journal.
        from tools.tests.template_build_fixture import bounded_config, synthetic_helper
        from tools.tests.test_template_build_service import limits
        journal = self.work / 'journal'
        journal.mkdir(mode=0o700)
        args = dict(bounded=bounded_config(synthetic_helper()))
        store = BuildStore(journal, limits(), binding=first['binding'], **args)
        store.close()
        before = (journal / 'builds.db').read_bytes()
        with self.assertRaises(ValueError):
            BuildStore(journal, limits(), binding=second['binding'], **args)
        self.assertEqual((journal / 'builds.db').read_bytes(), before)
        store = BuildStore(journal, limits(), binding=first['binding'], **args)
        store.close()
        with self.assertRaisesRegex(ValueError, 'changed since'): guest.recheck(first)
        self.assertEqual(guest.recheck(second), second['rust_guest'])
        config['tools']['envd'] = config['tools']['busybox']
        path.write_text(json.dumps(config))
        with self.assertRaises(ValueError): configuration(path, require_physical_lease=False)

    def test_materials_have_fixed_paths_independent_pins_and_accounted_capacity(self):
        binding = guest.snapshot(self.profile_path)
        files, profile = guest.materials(binding)
        self.assertEqual(len(files), 4)
        self.assertNotIn('source.tar', [path.name for path, _, _ in files])
        translated = json.loads(profile)
        self.assertEqual(translated['release'], '/input/rust-guest/release')
        self.assertEqual(translated['configuration'], '/input/rust-guest/config.json')
        self.assertEqual(translated['manifest_sha256'], self.profile['manifest_sha256'])
        largest, occupied = guest.capacity(binding)
        self.assertGreaterEqual(largest, max(expected['bytes'] for _, _, expected in files))
        self.assertGreater(occupied, sum(expected['bytes'] for _, _, expected in files))
        changed = copy.deepcopy(binding['identity'])
        changed['versions']['pid_format_version'] = True
        original = copy.deepcopy(changed)
        original['versions']['pid_format_version'] = 1
        self.assertFalse(guest.equal(original, changed))

    def test_capacity_rejects_guest_material_beyond_existing_builder_disk(self):
        runtime, builder, limits, base, registry = inputs()
        disk = base['resources']['disk_bytes']
        required = limits['max_disk_mib'] * MIB + limits['max_expanded_bytes'] + 2 * limits['max_input_bytes'] + 64 * MIB
        arguments = dict(registry=registry, kernel_bytes=64 * MIB, largest_tool_bytes=128 * MIB)
        self.assertEqual(validate_capacity(runtime, builder, limits, base, **arguments,
                                          guest_input_bytes=disk - required), disk)
        with self.assertRaisesRegex(ValueError, 'builder disk'):
            validate_capacity(runtime, builder, limits, base, **arguments, guest_input_bytes=disk - required + 1)


class UploadReaderTests(GuestFixture):
    def reader(self, *, pin=None):
        path = self.release / 'sandproof-guest-service'
        return PinnedReader(path, pin or self.manifest['artifacts']['service'], 100, lambda: self.now)

    def setUp(self):
        super().setUp()
        self.now = 0

    def test_exact_stream_hash_and_eof_with_bounded_reads(self):
        with self.reader() as reader:
            data = bytearray()
            while block := reader.read():
                self.assertLessEqual(len(block), 65536)
                data.extend(block)
            self.assertTrue(reader.complete)
        self.assertTrue(reader.closed)
        self.assertEqual(hashlib.sha256(data).hexdigest(), self.manifest['artifacts']['service']['sha256'])

    def test_changed_pin_truncation_growth_and_path_replacement_fail(self):
        expected = self.manifest['artifacts']['service']
        for mutation in ('pin', 'truncated', 'grew', 'replacement'):
            with self.subTest(mutation=mutation):
                path = self.release / 'sandproof-guest-service'
                old = path.read_bytes()
                pin = expected | {'sha256': '0' * 64} if mutation == 'pin' else expected
                with self.reader(pin=pin) as reader:
                    if mutation != 'pin':
                        path.chmod(0o600)
                        if mutation == 'replacement':
                            path.unlink()
                        path.write_bytes(old[:-1] if mutation == 'truncated' else old + b'x' if mutation == 'grew' else old)
                        path.chmod(0o500)
                    with self.assertRaises(ValueError):
                        while reader.read(): pass
                path.chmod(0o600)
                path.write_bytes(old)
                path.chmod(0o500)

    def test_deadline_expiry_at_read_and_eof_cannot_complete(self):
        for read_first in (False, True):
            self.now = 0
            with self.reader() as reader:
                if read_first: reader.read()
                self.now = 100
                with self.assertRaisesRegex(BuildError, 'BuildDeadlineExceeded'): reader.read()
                self.assertFalse(reader.complete)

    def test_links_and_nonregular_sources_are_rejected_before_read(self):
        path = self.release / 'sandproof-guest-service'
        link = self.work / 'linked'
        os.link(path, link)
        with self.assertRaises(ValueError): self.reader()
        link.unlink()
        path.unlink()
        path.symlink_to(self.config_path)
        with self.assertRaises(OSError): self.reader()
        path.unlink()
        os.mkfifo(path, 0o600)
        with self.assertRaises(ValueError): self.reader()

    def test_cancelled_upload_keeps_fd_until_reader_exits_even_after_runner_join_timeout(self):
        started, release, runner_timeout, finished = (threading.Event() for _ in range(4))
        seen, errors = [], []
        async def stalled(_sandbox, _version, _destination, data, _deadline, _clock):
            seen.append(data)
            def read():
                started.set()
                if not release.wait(timeout=5): raise AssertionError('reader not released')
                return data.read(65536)
            async with asyncio.timeout(0.05):
                await asyncio.to_thread(read)
        def invoke():
            try:
                upload(None, None, '/unused', self.release / 'sandproof-guest-service', 100, lambda: 0,
                       expected=self.manifest['artifacts']['service'])
            except BaseException as error:
                errors.append(error)
            finally:
                finished.set()
        with patch('tools.template_build_upload._write', side_effect=stalled), \
                patch('asyncio.constants.THREAD_JOIN_TIMEOUT', 0.01), \
                patch('asyncio.base_events.warnings.warn', side_effect=lambda *_a, **_k: runner_timeout.set()):
            thread = threading.Thread(target=invoke)
            thread.start()
            try:
                self.assertTrue(started.wait(timeout=2))
                self.assertTrue(runner_timeout.wait(timeout=2))
                self.assertFalse(finished.wait(timeout=0.1))
                self.assertFalse(seen[0].closed)
                self.assertGreater(os.fstat(seen[0].fd).st_size, 0)
            finally:
                release.set()
                thread.join(timeout=5)
        self.assertFalse(thread.is_alive())
        self.assertTrue(finished.is_set())
        self.assertTrue(seen[0].closed)
        self.assertEqual(len(errors), 1)
        self.assertIsInstance(errors[0], TimeoutError)


class GuestPublicationTests(GuestFixture):
    def setUp(self):
        super().setUp()
        self.registry = self.work / 'registry'
        for path in (self.registry, self.registry / 'inbox', self.registry / 'artifacts'):
            path.mkdir(mode=0o700)
        self.job = 'a' * 32
        self.directory = self.work / self.job
        self.directory.mkdir(mode=0o700)
        self.disk = self.directory / 'candidate.ext4'
        self.disk.write_bytes(b'opaque candidate')
        self.disk.chmod(0o600)
        self.kernel = self.work / 'kernel'
        self.kernel.write_bytes(b'synthetic kernel')
        self.status = dict(state='candidate', ready=False, startup_applied=False,
            disk_bytes=self.disk.stat().st_size, min_free_bytes=1,
            image_digest='sha256:' + 'a' * 64, rootfs_sha256=hashlib.sha256(self.disk.read_bytes()).hexdigest(),
            runtime_defaults=dict(env_vars={}, user='root', workdir='/', argv=[]),
            guest=guest.snapshot(self.profile_path)['identity'])
        self.row = dict(id=self.job, owner=1, deadline=100, staging_cleanup=None, publication_attempted=False,
            fs_nonce='b' * 32, candidate=json.dumps(self.status),
            request=json.dumps({'resources': {'cpuCount': 1, 'memoryMiB': 256}}))
        self.store = SimpleNamespace(get=lambda _job: self.row, clock=lambda: 0,
            directory=lambda _job: self.directory, begin_publication=Mock(), clear_publication_attempt=Mock())
        self.backend = Backend.__new__(Backend)
        self.backend.config = dict(registry={'root': str(self.registry)}, registry_owner_uid=os.geteuid(),
            runtime={'kernel': str(self.kernel)}, rust_guest_profile=str(self.profile_path),
            rust_guest=guest.snapshot(self.profile_path))
        self.backend.request = Mock()

    def test_staging_handoff_restart_and_unknown_post_keep_original_identity(self):
        self.backend.request.side_effect = [BuildError(404, 'NotFound'), BuildError(503, 'Unknown')]
        with self.assertRaisesRegex(BuildError, 'Unknown'): self.backend.publish(self.store, self.row)
        self.store.begin_publication.assert_called_once_with(self.job)
        self.store.clear_publication_attempt.assert_not_called()
        self.assertFalse(self.disk.exists())
        manifest = self.registry / 'inbox' / self.job / 'manifest.json'
        saved = manifest.read_bytes()
        self.assertEqual(json.loads(saved)['binding']['guest'], self.status['guest'])
        # Model crash after rename before begin_publication: repeat original
        # staging with no source filename, then publish the same candidate ID.
        self.backend.request.side_effect = [BuildError(404, 'NotFound'), {'state': 'pending'}]
        self.assertEqual(self.backend.publish(self.store, self.row), {'state': 'pending'})
        self.assertEqual(manifest.read_bytes(), saved)
        self.assertEqual(self.backend.request.call_args.args[3], {'candidateID': self.job})
        self.row['publication_attempted'] = True
        self.backend.request.side_effect = [BuildError(404, 'NotFound')]
        with self.assertRaisesRegex(BuildError, 'PublicationOutcomeUnknown'):
            self.backend.publish(self.store, self.row)
        # A known original receipt remains observable even if material was
        # removed; it does not permit a new submission with changed bytes.
        self.profile_path.unlink()
        self.backend.request.side_effect = [{'state': 'ready', 'template': {'revision': 'original'}}]
        self.assertEqual(self.backend.publish(self.store, self.row)['template']['revision'], 'original')

    def test_unpublished_candidate_or_changed_configuration_refuses_before_staging(self):
        for change in ('status', 'profile'):
            self.backend.request.reset_mock(side_effect=True)
            self.backend.request.side_effect = [BuildError(404, 'NotFound')]
            if change == 'status':
                self.row['candidate'] = json.dumps(self.status | {'guest': None})
            else:
                self.row['candidate'] = json.dumps(self.status)
                self.config['runtime']['worker_threads'] += 1
                self.save_profile()
            with self.subTest(change=change), self.assertRaisesRegex(BuildError, 'CandidateStagingRejected'):
                self.backend.publish(self.store, self.row)
            self.store.begin_publication.assert_not_called()
            self.assertEqual(list((self.registry / 'inbox').iterdir()), [])
            self.assertTrue(self.disk.exists())
