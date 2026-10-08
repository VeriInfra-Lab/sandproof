"""Fixed context membership and independent sparse material publication."""
import copy
import errno
import hashlib
import json
import os
from pathlib import Path
import tarfile
import tempfile
import sys
import unittest
from unittest.mock import patch

from benchmarks.ci_fixtures import FORBIDDEN_PATHS, MIB, load_ci_fixtures
from benchmarks.ci_verifier import REQUIREMENTS_TEXT
from benchmarks.prepare_ci_fixtures import (
    MAX_UV_BYTES, RECIPE, archive_ci_context, make_ci_fixture_manifest, publish_ready_fixture,
)
from tools.benchmark_tasks import InvalidInput, PIN, canonical


def sha(data):
    return hashlib.sha256(data).hexdigest()


class PrepareCiFixturesTests(unittest.TestCase):
    def setUp(self):
        if sys.platform != 'linux':
            # The existing copier's fast sendfile signature is Linux-specific;
            # use its real opaque-byte/zero-span fallback on other platforms.
            # This does not mock copy success: all actual bytes/inodes are checked.
            replacement = patch('tools.template_stage.os.sendfile',
                                side_effect=OSError(errno.ENOSYS, 'Linux extent copier unavailable'))
            replacement.start()
            self.addCleanup(replacement.stop)
        self.temporary = tempfile.TemporaryDirectory(dir=str(Path(tempfile.gettempdir()).resolve()))
        self.root = Path(self.temporary.name)
        self.root.chmod(0o700)
        self.source = self.root / 'source'
        self.source.mkdir(mode=0o700)
        self.entry = dict(name='first-task', source_path='first-task',
                          upstream_image_reference='registry.example:5000/public/task:fixed',
                          requirements=dict(cpus=1, memory_mib=256, disk_mib=1))
        directory = self.source / self.entry['source_path']
        (directory / 'environment').mkdir(parents=True)
        (directory / 'solution').mkdir()
        (directory / 'tests').mkdir()
        self.original = directory / 'environment/Dockerfile'
        self.original.write_bytes(b'FROM ubuntu:24.04\nWORKDIR /app\n')
        (directory / 'solution/solve.sh').write_bytes(b'private standard answer must never be packaged')
        (directory / 'tests/test_outputs.py').write_bytes(b'private assertions must never be packaged')
        self.entry['files'] = [dict(path='environment/Dockerfile', role='environment', mode='100644',
                                    bytes=self.original.stat().st_size, sha256=sha(self.original.read_bytes()))]
        self.entry['files_sha256'] = sha(canonical(self.entry['files']))
        self.uv = self.root / 'trusted-uv'
        header = bytearray(64)
        header[:7] = b'\x7fELF\x02\x01\x01'
        header[18:20] = (62).to_bytes(2, 'little')
        self.uv.write_bytes(header + b'bounded synthetic Linux amd64 ELF; never executed')
        self.uv.chmod(0o700)
        self.digest = 'sha256:' + 'a' * 64
        self.output = self.root / 'context.tar'
        self.kernel = self.root / 'kernel'
        self.kernel.write_bytes(b'kernel fixture')
        self.kernel.chmod(0o444)
        self.disk = self.root / 'ready.ext4'
        with self.disk.open('wb') as outgoing:
            outgoing.write(b'opaque disk prefix')
            outgoing.truncate(MIB)
        self.disk.chmod(0o444)
        self.template = dict(templateId='tpl-' + 'b' * 32, revision='r1', cpuCount=1, memoryMib=256,
            diskBytes=MIB, imageDigest='sha256:' + 'c' * 64,
            rootfsSha256=sha(self.disk.read_bytes()), kernelSha256=sha(self.kernel.read_bytes()),
            startup=dict(env_vars={'PATH': '/app/bin:/usr/bin'}, user='root', workdir='/app', argv=['/bin/bash']))
        self.destination = self.root / 'published'

    def tearDown(self):
        self.temporary.cleanup()

    def archive(self, **kwargs):
        args = dict(entry=self.entry, source=self.source, output=self.output, uv_path=self.uv, base_digest=self.digest)
        args.update(kwargs)
        return archive_ci_context(**args)

    def publish(self, **kwargs):
        args = dict(entry=self.entry, template=self.template, ready_rootfs=self.disk, kernel=self.kernel,
                    dest=self.destination, original_digest=self.digest)
        args.update(kwargs)
        return publish_ready_fixture(**args)

    def test_archive_contains_only_reviewed_three_inputs_with_immutable_base(self):
        original = self.original.read_bytes()
        digest = self.archive()
        self.assertEqual(digest, sha(self.output.read_bytes()))
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o600)
        with tarfile.open(self.output) as archive:
            self.assertEqual(archive.getnames(), ['Dockerfile', 'ci-uv', 'verifier-requirements.txt'])
            dockerfile = archive.extractfile('Dockerfile').read()
            self.assertIn(('FROM registry.example:5000/public/task@' + self.digest + '\n').encode(), dockerfile)
            self.assertNotIn(b'ARG BASE_IMAGE', dockerfile)
            self.assertNotIn(b'${BASE_IMAGE}', dockerfile)
            self.assertEqual(archive.extractfile('ci-uv').read(), self.uv.read_bytes())
            self.assertEqual(archive.extractfile('verifier-requirements.txt').read(), REQUIREMENTS_TEXT.encode())
            self.assertEqual(archive.getmember('ci-uv').mode, 0o755)
            self.assertTrue(all(item.mtime == 0 for item in archive))
        self.assertEqual(self.original.read_bytes(), original)
        second = self.root / 'second.tar'
        self.assertEqual(self.archive(output=second), digest)

    def test_context_rejects_unpinned_base_changed_frozen_files_and_unsafe_source(self):
        for digest in ('mutable-tag', 'SHA256:' + 'a' * 64, 'sha256:' + 'g' * 64):
            with self.subTest(digest=digest), self.assertRaises(InvalidInput):
                self.archive(base_digest=digest)
        self.assertFalse(self.output.exists())
        for mutation in (dict(source_path='../escape'), dict(source_path=''), dict(files_sha256='0' * 64),
                         dict(upstream_image_reference='public/image;touch /tmp/x')):
            entry = copy.deepcopy(self.entry)
            entry.update(mutation)
            with self.subTest(mutation=mutation), self.assertRaises(InvalidInput):
                self.archive(entry=entry)
        self.original.write_bytes(b'changed original environment')
        with self.assertRaises(InvalidInput):
            self.archive()
        self.assertFalse(self.output.exists())

    def test_context_rejects_existing_output_links_unsafe_uv_and_architecture(self):
        sentinel = self.root / 'sentinel'
        sentinel.write_bytes(b'untouched')
        self.output.symlink_to(sentinel)
        with self.assertRaises(InvalidInput):
            self.archive()
        self.assertEqual(sentinel.read_bytes(), b'untouched')
        self.output.unlink()
        self.uv.chmod(0o777)
        with self.assertRaises(InvalidInput):
            self.archive()
        self.uv.chmod(0o700)
        self.uv.write_bytes(b'not a trusted Linux ELF executable')
        with self.assertRaises(InvalidInput):
            self.archive()
        self.assertFalse(self.output.exists())

    def test_oversized_uv_and_changed_recipe_placeholders_rejected(self):
        with self.uv.open('r+b') as outgoing:
            outgoing.truncate(MAX_UV_BYTES + 1)
        with self.assertRaises(InvalidInput):
            self.archive()
        self.uv.write_bytes(b'\x7fELF\x02\x01\x01' + bytes(57))
        bad_recipe = self.root / 'bad-Dockerfile'
        bad_recipe.write_bytes(RECIPE.read_bytes().replace(b'FROM ${BASE_IMAGE}', b'FROM ubuntu:24.04'))
        with patch('benchmarks.prepare_ci_fixtures.RECIPE', bad_recipe), self.assertRaises(InvalidInput):
            self.archive()
        self.assertFalse(self.output.exists())

    def test_reference_already_using_a_digest_must_match_pin(self):
        entry = copy.deepcopy(self.entry)
        entry['upstream_image_reference'] = 'public/task@sha256:' + 'b' * 64
        with self.assertRaises(InvalidInput):
            self.archive(entry=entry)

    def test_publication_copies_readonly_distinct_sparse_artifacts_and_exact_shape(self):
        item = self.publish()
        self.assertEqual(item['resources'], dict(cpus=1, memory_mib=256, disk_mib=1))
        self.assertEqual(item['original_oci'], dict(reference=self.entry['upstream_image_reference'], digest=self.digest))
        self.assertEqual(item['absent_paths'], list(FORBIDDEN_PATHS))
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o700)
        for source, label in ((self.disk, 'rootfs'), (self.kernel, 'kernel')):
            target = Path(item[label]['path'])
            self.assertEqual(target.read_bytes(), source.read_bytes())
            self.assertNotEqual((target.stat().st_dev, target.stat().st_ino), (source.stat().st_dev, source.stat().st_ino))
            self.assertEqual(target.stat().st_mode & 0o777, 0o444)
            self.assertEqual(target.stat().st_nlink, 1)
        # Mutating caller configuration cannot change the returned frozen defaults.
        self.template['startup']['env_vars']['PATH'] = 'changed'
        self.assertEqual(item['startup']['env_vars']['PATH'], '/app/bin:/usr/bin')

    def test_ready_digest_resource_startup_and_logical_size_rejection_precedes_creation(self):
        changes = [dict(rootfsSha256='0' * 64), dict(kernelSha256='0' * 64), dict(diskBytes=2 * MIB),
                   dict(memoryMib=512), dict(imageDigest='mutable'), dict(startup={})]
        for change in changes:
            template = copy.deepcopy(self.template)
            template.update(change)
            with self.subTest(change=change), self.assertRaises(InvalidInput):
                self.publish(template=template)
            self.assertFalse(self.destination.exists())
        self.disk.chmod(0o600)
        self.disk.write_bytes(b'wrong logical size')
        self.template['rootfsSha256'] = sha(self.disk.read_bytes())
        with self.assertRaises(InvalidInput):
            self.publish()
        self.assertFalse(self.destination.exists())

    def test_destination_collision_and_symlink_cannot_replace_other_material(self):
        self.destination.mkdir(mode=0o700)
        marker = self.destination / 'marker'
        marker.write_bytes(b'untouched')
        with self.assertRaises(InvalidInput):
            self.publish()
        self.assertEqual(marker.read_bytes(), b'untouched')
        alternative = self.root / 'destination-link'
        alternative.symlink_to(self.destination)
        with self.assertRaises(InvalidInput):
            self.publish(dest=alternative)

    def test_copy_failure_preserves_only_exact_partial_destination(self):
        with patch('benchmarks.prepare_ci_fixtures.copy_sparse', side_effect=OSError('controlled failure')), \
                self.assertRaises(InvalidInput):
            self.publish()
        self.assertTrue(self.destination.is_dir())
        self.assertEqual(list(self.destination.iterdir()), [])
        self.assertEqual(sha(self.disk.read_bytes()), self.template['rootfsSha256'])

    def test_source_mutation_during_copy_is_not_published_as_success(self):
        from tools.template_stage import copy_sparse

        def mutate(source, target):
            copy_sparse(source, target)
            source.chmod(0o600)
            with source.open('r+b') as incoming:
                incoming.write(b'changed')

        with patch('benchmarks.prepare_ci_fixtures.copy_sparse', side_effect=mutate), self.assertRaises(InvalidInput):
            self.publish()
        self.assertTrue(self.destination.is_dir())
        self.assertFalse((self.destination / 'kernel').exists())

    def test_manifest_envelope_roundtrips_through_strict_fixture_loader(self):
        first = self.publish()
        second_entry = copy.deepcopy(self.entry)
        second_entry.update(name='second-task', source_path='second-task', upstream_image_reference='public/second:fixed')
        second = self.publish(entry=second_entry, dest=self.root / 'second')
        lock = dict(dataset='terminal-bench@2.0', source=dict(repository=PIN.repository, commit=PIN.commit),
                    tasks=[self.entry, second_entry])
        manifest = make_ci_fixture_manifest([first, second], lock)
        path = self.root / 'manifest.json'
        data = canonical(manifest)
        path.write_bytes(data)
        path.chmod(0o600)
        values = load_ci_fixtures(path, sha(data), lock, ['first-task', 'second-task'])
        self.assertEqual(set(values), {'first-task', 'second-task'})
        self.assertFalse(values['first-task'].evidence()['original_environment'])
        with self.assertRaises(InvalidInput):
            make_ci_fixture_manifest([first, first], lock)
        with self.assertRaises(InvalidInput):
            make_ci_fixture_manifest([first], lock)
        changed = copy.deepcopy(second)
        changed['resources']['cpus'] = 3
        with self.assertRaises(InvalidInput):
            make_ci_fixture_manifest([first, changed], lock)
        self.assertEqual(json.loads(data)['fixtures'][0]['startup'], self.template['startup'])


if __name__ == '__main__':
    unittest.main()
