"""Controlled real-file checks for the C6 reader; no VM, SDK or clone claims."""
import copy
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tests/system'))
import lifecycle_unload_cow as cow
import lifecycle_unload_runtime as runtime


class RootOwned:
    """Only UID is controlled; all content, inode and other metadata are real."""
    st_uid = 0

    def __init__(self, original):
        self.original = original

    def __getattr__(self, name):
        return getattr(self.original, name)


def read_controlled_file(path, limit, *, mode):
    # Ordinary tests run without root. Exercise the EXISTING full protected
    # reader with only the already-covered root-owner precondition supplied.
    # Keep real nofollow open/read/fstat, lengths, SHA and all other identities.
    original_lstat, original_fstat = Path.lstat, os.fstat
    with patch.object(Path, 'lstat', lambda value: RootOwned(original_lstat(value))), \
            patch.object(os, 'fstat', lambda descriptor: RootOwned(original_fstat(descriptor))):
        return runtime.checked_file(path, limit, mode=mode)


def write_private(path, contents):
    path.write_bytes(contents)
    path.chmod(0o400)
    return read_controlled_file(path, len(contents), mode=0o400)


@contextmanager
def fixture():
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve()
        store = root / 's'
        bundle = store / ('capture-' + 'a' * 64)
        store.mkdir(mode=0o700)
        bundle.mkdir(mode=0o700)
        materials = []
        for name, contents in zip(cow.ARTIFACTS,
                (b'original state', bytes(range(256)) * 300, b'original disk'), strict=True):
            materials.append(dict(name=name, **write_private(bundle / name, contents)))
        manifest = write_private(bundle / 'manifest.json', json.dumps(dict(
            binding={'original': True}, files=[dict(relative_path=row['name'],
                length=row['bytes'], digest=row['sha256']) for row in materials])).encode())
        completion_path = store / ('completed-' + 'a' * 64)
        completion = write_private(completion_path, json.dumps(dict(
            manifest_digest=manifest['sha256'])).encode())
        physical = dict(materials=materials, manifest=manifest, completion=completion,
            material_paths=[str(bundle), str(completion_path)], facts=dict(
                manifest_digest=manifest['sha256'], material_bytes=sum(row['bytes'] for row in materials),
                capture=11, source_incarnation=7))
        witness = dict(nonce='b' * 64, pid=31, start_time_ticks=47, counter=1)
        running = dict(session='original-session', material_use_pin=True,
            running_backing=copy.deepcopy({name: physical[name] for name in cow.BACKING_FIELDS}))
        continuity = dict(ram_nonce_sha256=hashlib.sha256(witness['nonce'].encode()).hexdigest(),
            guest_pid=witness['pid'], guest_start_time_ticks=witness['start_time_ticks'], counter=2,
            new_authorization=True, old_authorizations_rejected=True)
        with patch.object(cow, 'checked_file', side_effect=read_controlled_file):
            yield SimpleNamespace(root=root, store=store, bundle=bundle, physical=physical,
                witness=witness, running=running, continuity=continuity,
                deadline=time.monotonic() + 60)


def probe(cell):
    return cow.SealedSourceProbe(cell.physical, session='original-session',
        witness_before=cell.witness, deadline=cell.deadline)


class SealedSourceProbeTests(unittest.TestCase):
    def test_actual_source_reread_after_separate_target_write_closes_original_descriptors(self):
        with fixture() as cell, probe(cell) as observation:
            descriptors = [entry[1] for entry in observation._directories]
            target = cell.root / 'restored-disk.img'
            target.write_bytes(b'changed restored target; this is not a real CoW or VM test')
            result = observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)
            self.assertTrue(result['source_unchanged'])
            self.assertEqual(result['manifest_digest'], cell.physical['facts']['manifest_digest'])
            self.assertEqual([entry['name'] for entry in result['files']],
                ['manifest.json', *cow.ARTIFACTS, 'completed-' + 'a' * 64])
            self.assertEqual(result['guest']['counter'], 2)
            for descriptor in descriptors:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)
            self.assertEqual((cell.bundle / 'disk-0.img').read_bytes(), b'original disk')
            self.assertEqual(set(cell.bundle.iterdir()),
                {cell.bundle / name for name in ('manifest.json', *cow.ARTIFACTS)})

    def test_each_source_file_mutation_is_rejected_and_never_removed(self):
        for name in ('manifest.json', *cow.ARTIFACTS, 'completion'):
            with self.subTest(name=name), fixture() as cell, probe(cell) as observation:
                path = (Path(cell.physical['material_paths'][1]) if name == 'completion'
                        else cell.bundle / name)
                path.chmod(0o600)
                path.write_bytes(b'different length; deterministic source stamp change')
                path.chmod(0o400)
                with self.assertRaisesRegex(ValueError, 'original sealed source file changed'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)
                self.assertTrue(path.exists())
                self.assertTrue(cell.bundle.exists())

    def test_same_bytes_in_replacement_bundle_cannot_replace_original_directory(self):
        with fixture() as cell, probe(cell) as observation:
            contents = {path.name: path.read_bytes() for path in cell.bundle.iterdir()}
            original = cell.store / 'held-original'
            cell.bundle.rename(original)
            cell.bundle.mkdir(mode=0o700)
            for name, data in contents.items():
                write_private(cell.bundle / name, data)
            with self.assertRaisesRegex(ValueError, 'original source directory changed'):
                observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)
            self.assertTrue(original.exists())
            self.assertTrue(cell.bundle.exists())

    def test_full_reader_rejects_a_wrong_expected_digest_even_when_size_and_inode_match(self):
        with fixture() as cell:
            cell.physical['materials'][0]['sha256'][0] ^= 1
            cell.running['running_backing'] = copy.deepcopy(
                {name: cell.physical[name] for name in cow.BACKING_FIELDS})
            with probe(cell) as observation:
                with self.assertRaisesRegex(ValueError, 'source bytes differ from original adopted manifest'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)

    def test_file_changed_after_its_own_hash_is_rejected_by_final_whole_anchor_check(self):
        with fixture() as cell, probe(cell) as observation:
            def read_then_change_earlier_file(path, limit, *, mode):
                result = read_controlled_file(path, limit, mode=mode)
                if path.name == 'disk-0.img':
                    earlier = cell.bundle / 'state.bin'
                    earlier.chmod(0o600)
                    earlier.write_bytes(b'late change after state hash')
                    earlier.chmod(0o400)
                return result
            with patch.object(cow, 'checked_file', side_effect=read_then_change_earlier_file):
                with self.assertRaisesRegex(ValueError, 'original sealed source file changed'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)

    def test_resume_must_keep_the_same_session_manifest_source_and_actual_backing_pin(self):
        for fault in ('session', 'pin', 'manifest', 'bundle', 'material_inode'):
            with self.subTest(fault=fault), fixture() as cell, probe(cell) as observation:
                if fault == 'session':
                    cell.running['session'] = 'other-session'
                elif fault == 'pin':
                    cell.running['material_use_pin'] = False
                elif fault == 'manifest':
                    cell.running['running_backing']['manifest']['sha256'][0] ^= 1
                elif fault == 'bundle':
                    cell.running['running_backing']['material_paths'][0] += '-other'
                else:
                    cell.running['running_backing']['materials'][0]['inode'] += 1
                with self.assertRaisesRegex(ValueError, 'original restored source and backing pin required'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)

    def test_ram_process_and_actual_advance_cannot_be_replaced_by_unchanged_or_other_witness(self):
        for name, wrong in (('counter', 1), ('counter', True), ('guest_pid', 32),
                ('guest_start_time_ticks', 48), ('ram_nonce_sha256', 'c' * 64),
                ('new_authorization', False), ('old_authorizations_rejected', False)):
            with self.subTest(name=name, wrong=wrong), fixture() as cell, probe(cell) as observation:
                cell.continuity[name] = wrong
                with self.assertRaisesRegex(ValueError, 'original process RAM/disk advance evidence required'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline)

    def test_original_adopted_manifest_digest_and_complete_table_are_required_at_anchor(self):
        for fault in ('manifest', 'missing_material', 'extra_material', 'material_total', 'key'):
            with self.subTest(fault=fault), fixture() as cell:
                if fault == 'manifest':
                    cell.physical['facts']['manifest_digest'] = [0] * 32
                elif fault == 'missing_material':
                    cell.physical['materials'].pop()
                elif fault == 'extra_material':
                    cell.physical['materials'].append(copy.deepcopy(cell.physical['materials'][0]))
                elif fault == 'material_total':
                    cell.physical['facts']['material_bytes'] += 1
                else:
                    cell.physical['material_paths'][1] += '-other'
                with self.assertRaises(ValueError):
                    probe(cell)

    def test_original_deadline_is_not_extended_and_context_closes_on_preverify_failure(self):
        with fixture() as cell:
            with self.assertRaisesRegex(RuntimeError, 'original workload failed'):
                with probe(cell) as observation:
                    descriptors = [entry[1] for entry in observation._directories]
                    raise RuntimeError('original workload failed')
            for descriptor in descriptors:
                with self.assertRaises(OSError):
                    os.fstat(descriptor)
            with probe(cell) as observation:
                with self.assertRaises(TimeoutError):
                    observation.verify_after_guest_write(cell.running, cell.continuity, time.monotonic() - 1)
            with probe(cell) as observation:
                with self.assertRaisesRegex(ValueError, 'deadline cannot be extended'):
                    observation.verify_after_guest_write(cell.running, cell.continuity, cell.deadline + 1)
            self.assertTrue(cell.bundle.exists())


if __name__ == '__main__':
    unittest.main()
