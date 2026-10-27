"""Reject incomplete representation inventories before invoking proof tools."""
from pathlib import Path
import importlib.util
import json
import shutil
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('envelope_formal', HERE / 'formal.py')
formal = importlib.util.module_from_spec(spec)
spec.loader.exec_module(formal)


class ResultMapBuild(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name)
        self.prefix = 'verification/aeneas/stage1-protocol/schema-result/'
        self.files = {self.prefix + name: ('generated fixture: ' + name).encode() for name in (
            'SandproofProtocolResult/Funs.lean', 'SandproofProtocolResult/Types.lean',
            'translation.json')}
        self.reviewed = {}
        for name, content in self.files.items():
            path = self.project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            self.reviewed[name] = formal.sha(path)
            path.unlink()
        self.commands = []
        self.mode = 'correct'

    def command(self, project, output, label, argv):
        self.assertEqual(project, self.project)
        self.commands.append(label)
        if label == 'map-extract':
            self.assertEqual(argv, ['original-wrapper', 'bash',
                str(formal.HERE.parent / 'schema-leaves/extract-map.sh')])
            if self.mode == 'command-failed':
                raise ValueError('original map command failed')
            raw = project / 'artifacts/schema-map.llbc'
            raw.parent.mkdir()
            raw.write_text(json.dumps(dict(has_errors=self.mode == 'incomplete',
                translated=dict(target_information=[dict(
                    key='other-target' if self.mode == 'wrong-target' else 'reviewed-target')]))))
            for name, content in self.files.items():
                if self.mode == 'missing' and name.endswith('/Funs.lean'):
                    continue
                (project / name).write_bytes(content + (b' stale' if self.mode == 'stale' else b''))
            if self.mode == 'extra':
                (project / self.prefix / 'Unexpected.lean').write_text('-- unexpected provider\n')
        elif label == 'build':
            self.assertEqual(argv, ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.EnvelopeSchema'])
            for name, digest in self.reviewed.items():
                self.assertEqual(formal.sha(project / name), digest)
        else:
            self.fail('unexpected command: ' + label)

    def build(self):
        return formal.build_schema(self.project, self.project, ['original-wrapper'],
                                   self.command, self.reviewed, 'reviewed-target')

    def test_original_map_is_admitted_before_build(self):
        result = self.build()
        self.assertEqual(self.commands, ['map-extract', 'build'])
        self.assertEqual(result['generated_sha256'], self.reviewed)
        self.assertEqual(result['llbc_sha256'], formal.sha(self.project / 'artifacts/schema-map.llbc'))

    def test_missing_provider_stops_before_build(self):
        self.mode = 'missing'
        with self.assertRaisesRegex(ValueError, 'generation differs'):
            self.build()
        self.assertEqual(self.commands, ['map-extract'])

    def test_stale_or_extra_provider_stops_before_build(self):
        for mode in ('stale', 'extra'):
            with self.subTest(mode=mode):
                self.setUp()
                self.mode = mode
                with self.assertRaisesRegex(ValueError, 'generation differs'):
                    self.build()
                self.assertEqual(self.commands, ['map-extract'])

    def test_incomplete_or_wrong_target_ir_stops_before_build(self):
        for mode in ('incomplete', 'wrong-target'):
            with self.subTest(mode=mode):
                self.setUp()
                self.mode = mode
                with self.assertRaisesRegex(ValueError, 'wrong-target'):
                    self.build()
                self.assertEqual(self.commands, ['map-extract'])

    def test_failed_extractor_stops_before_build(self):
        self.mode = 'command-failed'
        with self.assertRaisesRegex(ValueError, 'map command failed'):
            self.build()
        self.assertEqual(self.commands, ['map-extract'])


class Admission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.data = json.loads((HERE / 'inventory.json').read_text())
        for name in self.data['inputs']:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(formal.ROOT / name, target)
        self.save()

    def save(self):
        (self.root / formal.HERE / 'inventory.json').write_text(json.dumps(self.data))

    def test_current_reviewed_candidate_is_admitted(self):
        self.assertEqual(len(formal.inventory(self.root)['audits']), 24)

    def test_changed_source_is_rejected(self):
        source = self.root / 'sandbox-protocol/src/lifecycle/request.rs'
        source.write_text(source.read_text() + '\n// changed\n')
        with self.assertRaisesRegex(ValueError, 'changed'):
            formal.inventory(self.root)

    def test_missing_struct_is_rejected(self):
        del self.data['structs']['RecordedLifecycleIntent']
        self.save()
        with self.assertRaisesRegex(ValueError, 'structure'):
            formal.inventory(self.root)

    def test_missing_branch_is_rejected(self):
        del self.data['operations']['Destroy']
        self.save()
        with self.assertRaisesRegex(ValueError, 'branch'):
            formal.inventory(self.root)

    def test_field_reordering_is_rejected(self):
        self.data['structs']['OperationKey']['fields_in_declaration_order'].reverse()
        self.save()
        with self.assertRaisesRegex(ValueError, 'coverage'):
            formal.inventory(self.root)

    def test_signed_field_cannot_be_unsigned(self):
        self.data['operations']['Renew'][1][1] = 'u64'
        self.save()
        with self.assertRaisesRegex(ValueError, 'operation fields'):
            formal.inventory(self.root)

    def test_missing_audit_is_rejected(self):
        self.data['audits'].pop()
        self.save()
        with self.assertRaisesRegex(ValueError, 'audit'):
            formal.inventory(self.root)

    def test_adoption_cannot_be_inferred(self):
        self.data['a6_instance_adopted'] = True
        self.save()
        with self.assertRaisesRegex(ValueError, 'unadopted'):
            formal.inventory(self.root)

    def test_prepare_race_cannot_leave_success_flag(self):
        inputs = dict(self.data['inputs'])
        with patch.object(formal, 'ROOT', self.root), patch.object(
            formal, 'snapshot', side_effect=[inputs, dict(inputs, changed='changed')]
        ), patch.object(formal.subprocess, 'check_output', return_value='reviewed-base\n'):
            result = formal.run(self.root / 'output', prepare_only=True)
        self.assertFalse(result['complete'])
        self.assertFalse(result.get('prepared', False))
        self.assertIn('changed during', result['error'])


if __name__ == '__main__':
    unittest.main()
