"""Private prepared-CI identity checks; no images, VMs or host manipulation."""
import copy
from dataclasses import FrozenInstanceError
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.ci_fixtures import (
    FORBIDDEN_PATHS, MAX_MANIFEST_BYTES, MIB, guest_preflight_command, load_ci_fixtures,
)
from benchmarks.ci_verifier import PYTHON, REQUIREMENTS_SHA256
from tools.benchmark_tasks import InvalidInput, PIN, canonical


def sha(data):
    return hashlib.sha256(data).hexdigest()


class CIFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(dir=str(Path(tempfile.gettempdir()).resolve()))
        self.root = Path(self.temporary.name)
        self.root.chmod(0o700)
        self.kernel = self.root / 'kernel'
        self.kernel.write_bytes(b'bounded kernel fixture')
        self.kernel.chmod(0o444)
        self.lock = dict(dataset='terminal-bench@2.0',
                         source=dict(repository=PIN.repository, commit=PIN.commit), tasks=[])
        self.manifest = dict(schema_version=1, dataset=self.lock['dataset'], source_commit=PIN.commit,
                             verifier_python=PYTHON, verifier_requirements_sha256=REQUIREMENTS_SHA256,
                             fixtures=[])
        self.names = ['first-task', 'second-task']
        for index, name in enumerate(self.names):
            image = self.root / (name + '.image')
            data = bytes([index + 1]) * MIB
            image.write_bytes(data)
            image.chmod(0o444)
            shape = dict(cpus=index + 1, memory_mib=256, disk_mib=1)
            entry = dict(name=name, files_sha256=sha(name.encode()),
                         upstream_image_reference='public/' + name + ':fixed',
                         requirements=shape)
            self.lock['tasks'].append(entry)
            self.manifest['fixtures'].append(dict(name=name, files_sha256=entry['files_sha256'],
                original_oci=dict(reference=entry['upstream_image_reference'], digest='sha256:' + sha(data)),
                kernel=dict(path=str(self.kernel), sha256=sha(self.kernel.read_bytes())),
                rootfs=dict(path=str(image), sha256=sha(data)), resources=shape,
                startup=dict(env_vars={'FIXTURE': name}, user='root', workdir='/app', argv=['/bin/bash']),
                absent_paths=list(FORBIDDEN_PATHS)))
        self.path = self.root / 'manifest.json'

    def tearDown(self):
        self.temporary.cleanup()

    def save(self, manifest=None):
        data = json.dumps(self.manifest if manifest is None else manifest).encode()
        self.path.write_bytes(data)
        self.path.chmod(0o600)
        return sha(data)

    def load(self, *, names=None, lock=None, digest=None):
        return load_ci_fixtures(self.path, self.save() if digest is None else digest,
                               self.lock if lock is None else lock,
                               self.names if names is None else names)

    def test_selected_descriptor_retains_exact_shape_startup_and_private_identity(self):
        values = self.load()
        self.assertEqual(list(values), self.names)
        for name, fixture in values.items():
            expected = next(r for r in self.manifest['fixtures'] if r['name'] == name)
            template = fixture.template_descriptor()
            binding = dict(name=name, files_sha256=fixture.files_sha256,
                           rootfs_sha256=fixture.rootfs_sha256, startup=fixture.startup)
            self.assertEqual(template['template_id'], 'tpl-' + sha(canonical(binding))[:32])
            self.assertEqual(fixture.guest_preflight_command(), guest_preflight_command())
            self.assertEqual((template['revision'], template['owner'], template['envd_version']), ('r1', 1, '0.9.0'))
            self.assertEqual(template['startup'], expected['startup'])
            self.assertEqual(template['rootfs']['size_bytes'], MIB)
            self.assertEqual(template['resources'], dict(cpu_count=expected['resources']['cpus'],
                memory_mib=256, disk_bytes=MIB, host_memory_bytes=512 * MIB))
            self.assertEqual(fixture.verify_artifacts()['fixture_sha256'], sha(self.path.read_bytes()))
            evidence = fixture.evidence()
            self.assertEqual(evidence['verifier_requirements_sha256'], REQUIREMENTS_SHA256)
            self.assertFalse(evidence['original_environment'])
            self.assertFalse(evidence['original_verifier'])
            self.assertFalse(evidence['original_oracle_verifier'])
            self.assertTrue(evidence['original_test_assertions'])
            self.assertTrue(evidence['original_oracle'])
            self.assertFalse(evidence['public_network'])
            self.assertEqual(evidence['guest_preflight'], 'required_not_run')
            self.assertNotIn(str(self.root), json.dumps(evidence))
            # Returned configuration snapshots cannot alter an approved fixture.
            template['startup']['env_vars']['FIXTURE'] = 'modified'
            fixture.resources['cpus'] = 99
            self.assertEqual(fixture.startup, expected['startup'])
            self.assertEqual(fixture.resources, expected['resources'])
            with self.assertRaises(FrozenInstanceError):
                fixture.name = 'changed'

    def test_serial_subset_uses_only_its_approved_fixture(self):
        values = self.load(names=[self.names[1]])
        self.assertEqual(list(values), [self.names[1]])
        for names in ([], ['unknown'], ['first-task', 'first-task'], 'first-task', [None]):
            with self.subTest(names=names), self.assertRaises(InvalidInput):
                self.load(names=names)

    def test_manifest_operator_pin_and_duplicate_json_fields_are_required(self):
        self.save()
        for digest in ('0' * 64, 'G' * 64, None):
            with self.subTest(digest=digest), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, digest, self.lock, self.names)
        for data in (b'{"schema_version":1,"schema_version":1}', b'not-json', b'x' * (MAX_MANIFEST_BYTES + 1)):
            self.path.write_bytes(data)
            with self.subTest(data_length=len(data)), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, sha(data), self.lock, self.names)

    def test_unreviewed_dataset_dependencies_and_manifest_fields_are_rejected(self):
        changes = [dict(schema_version=True), dict(dataset='terminal-bench@2.1'), dict(source_commit='0' * 40),
                   dict(verifier_python='/usr/bin/python3'), dict(verifier_requirements_sha256='0' * 64),
                   dict(allow_internet=True)]
        for change in changes:
            value = copy.deepcopy(self.manifest)
            value.update(change)
            with self.subTest(change=change), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, self.save(value), self.lock, self.names)
        lock = copy.deepcopy(self.lock)
        lock['source']['commit'] = '0' * 40
        with self.assertRaises(InvalidInput):
            self.load(lock=lock)

    def test_two_fixture_scope_task_inventory_and_original_oci_are_bound(self):
        mutations = [lambda x: x['fixtures'].pop(), lambda x: x['fixtures'].append(copy.deepcopy(x['fixtures'][0])),
                     lambda x: x['fixtures'][1].update(name='first-task'),
                     lambda x: x['fixtures'][0].update(name='not-locked'),
                     lambda x: x['fixtures'][0].update(files_sha256='0' * 64),
                     lambda x: x['fixtures'][0]['original_oci'].update(reference='other/image:mutable'),
                     lambda x: x['fixtures'][0]['original_oci'].update(digest='mutable-tag'),
                     lambda x: x['fixtures'][0].update(absent_paths=['/solution', '/tests'])]
        for index, mutation in enumerate(mutations):
            value = copy.deepcopy(self.manifest)
            mutation(value)
            with self.subTest(index=index), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, self.save(value), self.lock, self.names)

    def test_resources_and_total_rootfs_bytes_cannot_drift(self):
        for key, value in (('cpus', True), ('cpus', 255), ('memory_mib', 0), ('disk_mib', 2)):
            manifest = copy.deepcopy(self.manifest)
            manifest['fixtures'][0]['resources'][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, self.save(manifest), self.lock, self.names)
        image = Path(self.manifest['fixtures'][0]['rootfs']['path'])
        image.chmod(0o600)
        image.write_bytes(b'wrong total disk size')
        self.manifest['fixtures'][0]['rootfs']['sha256'] = sha(image.read_bytes())
        with self.assertRaises(InvalidInput):
            self.load()

    def test_startup_defaults_are_structured_bounded_and_not_shell_commands(self):
        cases = [dict(user='0:0'), dict(user='root; touch /tmp/x'), dict(workdir='relative'),
                 dict(workdir='/app/../tests'), dict(workdir='/app\\escape'),
                 dict(argv=['']), dict(argv=['ok\0bad']), dict(argv=['x' * 8193]),
                 dict(env_vars={'BAD-NAME': 'x'}), dict(env_vars={'GOOD': '\0'}),
                 dict(env_vars={'GOOD': 'x' * (32 * 1024)}), dict(extra='field')]
        for change in cases:
            manifest = copy.deepcopy(self.manifest)
            manifest['fixtures'][0]['startup'].update(change)
            with self.subTest(change=change), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, self.save(manifest), self.lock, self.names)

    def test_private_manifest_and_nonwritable_materials_reject_links_permissions_and_special_files(self):
        pin = self.save()
        self.path.chmod(0o644)
        with self.assertRaises(InvalidInput):
            load_ci_fixtures(self.path, pin, self.lock, self.names)
        self.path.chmod(0o600)
        for target in (self.path, self.kernel):
            link = self.root / 'link'
            link.symlink_to(target)
            if target == self.path:
                with self.assertRaises(InvalidInput):
                    load_ci_fixtures(link, pin, self.lock, self.names)
            else:
                self.manifest['fixtures'][0]['kernel']['path'] = str(link)
                with self.assertRaises(InvalidInput):
                    self.load()
            link.unlink()
        self.manifest['fixtures'][0]['kernel']['path'] = str(self.kernel)
        self.kernel.chmod(0o666)
        with self.assertRaises(InvalidInput):
            self.load()
        self.kernel.chmod(0o444)
        linked = self.root / 'hardlink'
        os.link(self.kernel, linked)
        with self.assertRaises(InvalidInput):
            self.load()
        linked.unlink()
        fifo = self.root / 'fifo'
        os.mkfifo(fifo)
        self.manifest['fixtures'][0]['kernel']['path'] = str(fifo)
        with self.assertRaises(InvalidInput):
            self.load()

    def test_unsafe_ancestry_relative_path_and_artifact_fingerprint_are_rejected(self):
        self.root.chmod(0o777)
        with self.assertRaises(InvalidInput):
            self.load()
        self.root.chmod(0o700)
        for path in ('relative', str(self.root / '..' / self.kernel.name), None):
            manifest = copy.deepcopy(self.manifest)
            manifest['fixtures'][0]['kernel']['path'] = path
            with self.subTest(path=path), self.assertRaises(InvalidInput):
                load_ci_fixtures(self.path, self.save(manifest), self.lock, self.names)
        self.manifest['fixtures'][0]['kernel']['sha256'] = '0' * 64
        with self.assertRaises(InvalidInput):
            self.load()

    def test_recheck_rejects_same_bytes_replacement_and_in_place_changes(self):
        for material in ('manifest', 'kernel', 'rootfs'):
            fixture = self.load(names=['first-task'])['first-task']
            path = {'manifest': self.path, 'kernel': self.kernel, 'rootfs': fixture.rootfs}[material]
            old = path.read_bytes()
            replacement = self.root / 'replacement'
            replacement.write_bytes(old)
            replacement.chmod(path.stat().st_mode & 0o777)
            replacement.replace(path)
            with self.subTest(material=material), self.assertRaises(InvalidInput):
                fixture.verify_artifacts()
        fixture = self.load(names=['first-task'])['first-task']
        fixture.kernel.chmod(0o600)
        fixture.kernel.write_bytes(b'changed kernel bytes')
        with self.assertRaises(InvalidInput):
            fixture.template_descriptor()

    def test_manifest_growth_and_replacement_during_read_fail_closed(self):
        self.save()
        original_read = os.read
        changed = False

        def grow(fd, amount):
            nonlocal changed
            if not changed:
                changed = True
                with self.path.open('ab') as outgoing:
                    outgoing.write(b' ' * (MAX_MANIFEST_BYTES + 1))
            return original_read(fd, amount)

        with patch('benchmarks.ci_fixtures.os.read', side_effect=grow), self.assertRaises(InvalidInput):
            load_ci_fixtures(self.path, sha(self.path.read_bytes()), self.lock, self.names)

    def test_guest_preflight_is_fixed_and_cannot_claim_execution(self):
        command = guest_preflight_command()
        self.assertIn('test -x ' + PYTHON, command)
        for path in FORBIDDEN_PATHS:
            self.assertIn('if test -e ' + path + ' || test -L ' + path + '; then exit 1; fi', command)
        for forbidden in ('curl ', 'apt ', 'pip ', 'pytest ', 'rm ', 'PRIVATE KEY'):
            self.assertNotIn(forbidden, command)

    def test_guest_preflight_shell_rejects_preseeded_phase_files_and_dangling_links(self):
        for blocked in (None, *FORBIDDEN_PATHS, 'dangling'):
            with self.subTest(blocked=blocked), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                python = root / PYTHON.lstrip('/')
                python.parent.mkdir(parents=True)
                python.write_bytes(b'fixture executable; this command only checks its mode')
                python.chmod(0o700)
                if blocked is not None:
                    path = root / (FORBIDDEN_PATHS[0] if blocked == 'dangling' else blocked).lstrip('/')
                    path.parent.mkdir(parents=True, exist_ok=True)
                    if blocked == 'dangling':
                        path.symlink_to(root / 'missing')
                    elif blocked == '/logs/verifier/reward.txt':
                        path.write_bytes(b'1\n')
                    else:
                        path.mkdir()
                command = guest_preflight_command()
                for path in (*FORBIDDEN_PATHS, PYTHON):
                    command = command.replace(path, shlex.quote(str(root / path.lstrip('/'))))
                result = subprocess.run(['/bin/sh', '-c', command], capture_output=True, timeout=5)
                self.assertEqual(result.returncode == 0, blocked is None, result.stderr)
                if blocked == 'dangling':
                    self.assertTrue((root / 'solution').is_symlink())


if __name__ == '__main__':
    unittest.main()
