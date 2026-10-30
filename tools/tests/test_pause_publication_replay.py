"""Synthetic formal-driver rejection checks; never Rust extraction or Lean proof."""
import copy
import hashlib
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest import mock

from tools import check_pause_publication_components as component

checks = component.load('test_pause_publication_inventory', component.HERE/'inventory.py')
replay = component.load('test_pause_publication_replay', component.HERE/'replay.py')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class PublicationInventoryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.proof = str(checks.BASE/'Checks.lean')
        self.target = checks.PREFIX+'synthetic_property'
        self.wrong = str(checks.HERE/'model/IgnoreStop.lean')
        self.wrong_target = checks.PREFIX+'IgnoreStop.synthetic_rejection'
        self.manifest = dict(schema=1, namespace=checks.PREFIX.rstrip('.'),
            proofs={self.proof: [self.target]}, required_theorems=[self.target],
            wrong=dict(file=self.wrong, namespace=checks.PREFIX+'IgnoreStop',
                       audits=[self.wrong_target]))
        self.write(self.proof, 'namespace '+checks.PREFIX.rstrip('.')+'\n'
                   'theorem synthetic_property : True := by trivial\nend\n')
        self.write(str(checks.BASE/'Audit.lean'), '#audit_controller_proof '+self.target+'\n')
        self.write(self.wrong, 'namespace '+checks.PREFIX+'IgnoreStop\n'
                   'theorem synthetic_rejection : True := by trivial\nend\n'
                   '#audit_controller_proof '+self.wrong_target+'\n')
        for module, root in checks.LIBRARIES:
            for leaf in ('Types', 'FunsExternal', 'Funs'):
                self.write(root+'/'+module+'/'+leaf+'.lean', '-- synthetic fixture only\n')
        self.save()

    def write(self, relative, text):
        path = self.root/relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def save(self):
        self.write(str(checks.HERE/'inventory.json'), json.dumps(self.manifest))

    def test_complete_inventory_comes_from_exact_final_manifest(self):
        files, manifest = checks.inventory(self.root)
        self.assertEqual(manifest, self.manifest)
        self.assertEqual(files[str(checks.BASE/'Audit.lean')], [self.target])
        self.assertEqual(files[self.wrong], [self.wrong_target])
        self.assertIn(self.proof, files)

    def test_missing_final_manifest_refuses_before_any_proof(self):
        (self.root/checks.HERE/'inventory.json').unlink()
        with self.assertRaisesRegex(ValueError, 'required source'):
            checks.inventory(self.root)

    def test_unlisted_file_and_missing_or_duplicate_theorem_are_rejected(self):
        extra = checks.BASE/'Omitted.lean'
        self.write(str(extra), 'theorem omitted : True := by trivial\n')
        with self.assertRaisesRegex(ValueError, 'every publication proof file'):
            checks.inventory(self.root)
        (self.root/extra).unlink()
        original = (self.root/self.proof).read_text()
        for text in ('namespace '+checks.PREFIX.rstrip('.')+'\nend\n',
                     original+'theorem synthetic_property : True := by trivial\n'):
            self.write(self.proof, text)
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, 'declaration inventory'):
                checks.inventory(self.root)

    def test_discovery_cannot_replace_a_missing_contractual_obligation(self):
        # Even reducing the discovered declarations and audit together cannot
        # erase an obligation still required by the final explicit contract.
        self.manifest['proofs'][self.proof] = []
        self.save()
        self.write(self.proof, 'namespace '+checks.PREFIX.rstrip('.')+'\nend\n')
        self.write(str(checks.BASE/'Audit.lean'), '')
        with self.assertRaisesRegex(ValueError, 'missing required publication theorem'):
            checks.inventory(self.root)

    def test_missing_extra_or_reordered_audits_are_rejected(self):
        path = str(checks.BASE/'Audit.lean')
        for text in ('', ('#audit_controller_proof '+self.target+'\n')*2,
                     '#audit_controller_proof '+self.target+'\n#audit_controller_proof Other.target\n'):
            self.write(path, text)
            with self.subTest(audit=text), self.assertRaisesRegex(ValueError, 'complete declared closure'):
                checks.inventory(self.root)

    def test_wrong_rejection_and_actual_generated_file_cannot_be_omitted(self):
        original = (self.root/self.wrong).read_text()
        self.write(self.wrong, original.replace('#audit_controller_proof '+self.wrong_target, ''))
        with self.assertRaisesRegex(ValueError, 'Wrong-Stop'):
            checks.inventory(self.root)
        self.write(self.wrong, original)
        module, root = checks.LIBRARIES[1]
        (self.root/root/module/'Funs.lean').unlink()
        with self.assertRaisesRegex(ValueError, 'required source'):
            checks.inventory(self.root)

    def test_reviewed_literals_are_read_without_importing_cli_code(self):
        path = self.root/'original.py'
        path.write_text('raise RuntimeError("must not import")\nTOOLS = {"tool": "sha"}\n')
        self.assertEqual(checks.literal_constant(path, 'TOOLS'), {'tool': 'sha'})
        for text in ('TOOLS = computed()\n', 'TOOLS = {}\nTOOLS = {}\n', 'OTHER = {}\n'):
            path.write_text(text)
            with self.subTest(text=text), self.assertRaises(ValueError):
                checks.literal_constant(path, 'TOOLS')


class PublicationGenerationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.module = 'CaptureCaller'
        self.namespace = 'CaptureCallerRust.sandbox_controller_core'
        self.generated = self.root/'lean'/self.module
        self.generated.mkdir(parents=True)
        self.functions = [self.function('sandbox_controller_core::'+rust, self.namespace+'.'+lean)
                          for rust, lean in checks.REQUIRED_FUNCTIONS]
        for name in checks.CAPTURE_METHODS:
            self.functions.append(self.function('sandbox_controller_core::capture::{CaptureHandoff}::'+name,
                self.namespace+'.capture.CaptureHandoff.'+name, 'controller-core/src/capture.rs'))
        self.externals = {'Reviewed.one', 'Reviewed.two'}
        self.llbc = {'has_errors': False}
        self.template = '\n'.join('axiom '+name+' : True' for name in sorted(self.externals))
        self.save()

    def function(self, rust, lean, file='controller-core/src/publication.rs'):
        return dict(rust_name=rust, lean_name=lean, is_opaque=False, is_local=True,
                    lean_file=self.module+'/Funs.lean', source={'file': file})

    def save(self):
        (self.root/'capture_handoff.llbc').write_text(json.dumps(self.llbc))
        (self.root/'lean/translation.json').write_text(json.dumps({'functions': self.functions}))
        (self.generated/'Funs.lean').write_text('\n'.join(
            'def '+row['lean_name'].removeprefix(self.namespace+'.')+' := ()' for row in self.functions))
        (self.generated/'FunsExternal_Template.lean').write_text(self.template)

    def verify(self):
        return checks.verify_generation(self.root, self.module, self.namespace, self.externals)

    def test_all_actual_local_functions_and_loop_bodies_are_required(self):
        selected = self.verify()
        self.assertIn(self.namespace+'.publication.indexed_vms_closed_loop.body', selected)
        self.assertIn(self.namespace+'.publication.indexed_slots_closed_loop', selected)
        original = copy.deepcopy(self.functions)
        for suffix in ('.publication.handle_publish_paused', '.publication.indexed_vms_closed_loop.body'):
            self.functions = [row for row in original if not row['lean_name'].endswith(suffix)]
            self.save()
            with self.subTest(suffix=suffix), self.assertRaisesRegex(ValueError, 'actual local definition'):
                self.verify()

    def test_opaque_external_duplicate_or_missing_final_body_cannot_be_a_root(self):
        original = copy.deepcopy(self.functions)
        for field, value in (('is_local', False), ('is_opaque', True), ('lean_file', 'Wrong/Funs.lean')):
            self.functions = copy.deepcopy(original); self.functions[0][field] = value; self.save()
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify()
        self.functions = original+[copy.deepcopy(original[0])]; self.save()
        with self.assertRaises(ValueError):
            self.verify()
        self.functions = original; self.save()
        (self.generated/'Funs.lean').write_text('-- metadata alone is not a real body\n')
        with self.assertRaisesRegex(ValueError, 'actual local definition'):
            self.verify()

    def test_failed_extraction_or_changed_reviewed_external_set_is_rejected(self):
        self.llbc['has_errors'] = True; self.save()
        with self.assertRaisesRegex(ValueError, 'Charon errors'):
            self.verify()
        self.llbc['has_errors'] = False
        for template in ('axiom Reviewed.one : True',
                         self.template+'\naxiom Reviewed.new : True',
                         self.template+'\naxiom Reviewed.one : True'):
            self.template = template; self.save()
            with self.subTest(template=template), self.assertRaisesRegex(ValueError, 'external'):
                self.verify()


class PublicationServiceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root/'Proof.lean').write_text('-- synthetic service fixture; not Lean-checked\n')
        self.files = {'Proof.lean': ['Synthetic.target']}
        digest = sha(self.root/'Proof.lean')
        self.row = dict(request=dict(tool='lean_diagnostic_messages',
            arguments=dict(file_path='Proof.lean'), expected_sha256=digest),
            revision=dict(file_sha256=digest), stale=False,
            result=dict(success=True, items=[dict(severity='info',
                message='Checked Synthetic.target: 0 standard logical dependencies')]))

    def check(self, rows):
        return checks.check_service(self.root, self.files, '\n'.join(map(json.dumps, rows)), sha)

    def test_complete_actual_file_diagnostics_require_matching_audits(self):
        self.assertEqual(self.check([self.row]), [self.row])

    def test_missing_duplicate_partial_stale_or_wrong_fingerprint_is_rejected(self):
        for rows in ([], [self.row, self.row]):
            with self.subTest(rows=len(rows)), self.assertRaises(ValueError):
                self.check(rows)
        variants = []
        row = copy.deepcopy(self.row); row['stale'] = True; variants.append(row)
        row = copy.deepcopy(self.row); row['revision']['file_sha256'] = 'changed'; variants.append(row)
        row = copy.deepcopy(self.row); row['request']['expected_sha256'] = 'changed'; variants.append(row)
        for field in ('partial', 'timed_out', 'failed_dependencies'):
            row = copy.deepcopy(self.row); row['result'][field] = True; variants.append(row)
        for row in variants:
            with self.subTest(row=row), self.assertRaises(ValueError):
                self.check([row])

    def test_warning_missing_or_unapproved_audit_is_rejected(self):
        for items in ([], [dict(severity='warning', message='warning')],
                      [dict(severity='info', message='Checked Other.target: 0 standard logical dependencies')],
                      [dict(severity='info', message='Checked Synthetic.target: 4 standard logical dependencies')]):
            row = copy.deepcopy(self.row); row['result']['items'] = items
            with self.subTest(items=items), self.assertRaises(ValueError):
                self.check([row])

    def test_wrong_host_role_cannot_start_formal_tools(self):
        args = SimpleNamespace(output=self.root/'attempt', head='synthetic', archive_sha256='synthetic')
        with mock.patch.dict(os.environ, {'SANDPROOF_ACCEPTANCE_ROLE': 'native'}), \
                mock.patch.object(replay.subprocess, 'run') as command:
            result = replay.run(args)
        self.assertFalse(result['complete'])
        self.assertFalse(result['whole_V4'])
        self.assertIn('formal host role', result['error'])
        command.assert_not_called()


if __name__ == '__main__':
    unittest.main()
