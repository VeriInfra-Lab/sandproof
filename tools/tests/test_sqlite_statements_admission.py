"""Admission mutations only; these are not Rust or Lean semantic proofs."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest

from tools import check_sqlite_statements as gate

from tools.tests.proof_input_fixture import reference_root

ROOT = reference_root()


class ActualStatementAdmissionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.correct = gate.read_json(ROOT / 'verification/aeneas/llbc/sandproof_sqlite_statements.llbc')
        cls.wrong = gate.read_json(ROOT / 'verification/aeneas/llbc/sandproof_sqlite_statements_wrong_count.llbc')

    def test_frozen_reference_ast_and_all_audits_admitted(self):
        registry = gate.check_pair(ROOT, self.correct, self.wrong)
        self.assertEqual(len(registry['interfaces']), 9)
        self.assertEqual(len(gate.expected_audits(ROOT)), 38)
        self.assertEqual(len(gate.expected_audits(ROOT, True)), 7)

    def test_all_original_tool_bodies_and_drop_fields_are_checked(self):
        for variant in ['correct', 'wrong']:
            data = copy.deepcopy(getattr(self, variant))
            target = next(f for f in data['translated']['fun_decls'] if f and f['def_id'] == 12)
            target['body'] = 'Opaque'
            with self.subTest(variant=variant), self.assertRaises(ValueError):
                gate.check_pair(ROOT, data if variant == 'correct' else self.correct,
                                data if variant == 'wrong' else self.wrong)

    def test_drop_unwind_and_normal_kind_mutations_are_rejected(self):
        def nodes(value):
            if isinstance(value, dict):
                yield value
                for child in value.values():
                    yield from nodes(child)
            elif isinstance(value, list):
                for child in value:
                    yield from nodes(child)
        for mutation in ['kind', 'on_unwind']:
            data = copy.deepcopy(self.correct)
            drop = next(n['Drop'] for n in nodes(data) if 'Drop' in n)
            if mutation == 'kind':
                drop['kind'] = 'Unconditional'
            else:
                drop['on_unwind'] = {'statements': []}
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                gate.check_pair(ROOT, data, self.wrong)

    def test_disabled_compiler_checks_or_missing_actual_roots_are_rejected(self):
        for flag in ['skip_borrowck', 'no_typecheck', 'no_normalize', 'no_reorder_decls', 'start_from']:
            data = copy.deepcopy(self.correct)
            data['translated']['options'][flag] = [] if flag == 'start_from' else True
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                gate.check_pair(ROOT, data, self.wrong)

    def test_short_name_map_order_only_is_canonicalized(self):
        data = copy.deepcopy(self.correct)
        data['translated']['short_names'] = [{'key': {'fixture': 1}, 'value': 'a'}, {'key': {'fixture': 2}, 'value': 'b'}]
        first = gate.full_digest(data)
        data['translated']['short_names'].reverse()
        self.assertEqual(gate.full_digest(data), first)
        with self.assertRaises(ValueError):
            gate.check_pair(ROOT, data, self.wrong)

    def test_duplicate_short_names_and_reordered_ordered_ast_are_rejected(self):
        duplicate = copy.deepcopy(self.correct)
        duplicate['translated']['short_names'] = [{'key': {'fixture': 1}, 'value': 'a'}, {'key': {'fixture': 1}, 'value': 'b'}]
        with self.assertRaises(ValueError):
            gate.full_digest(duplicate)
        ordered = copy.deepcopy(self.correct)
        ordered['translated']['fun_decls'].reverse()
        with self.assertRaises(ValueError):
            gate.check_pair(ROOT, ordered, self.wrong)

    def test_retained_wrong_body_is_the_actual_mutated_original_generation(self):
        good = (ROOT / gate.LIBRARY / 'SandproofSqliteStatements/Funs.lean').read_text()
        bad = (ROOT / gate.ASSETS / 'negative/Funs.lean').read_text()
        self.assertEqual(good.count('if val1 != 1#usize'), 1)
        self.assertEqual(bad, good.replace('if val1 != 1#usize', 'if val1 > 1#usize'))
        self.assertEqual(gate.sha(ROOT / gate.MODELS / 'Contract.lean'),
                         '4bb7e6b50dea7b680a555bc0229ead7ecb3a8e1c0815fcb632ed151222b655bf')

    def test_entry_modules_import_without_running_external_tools(self):
        assets = ROOT / gate.ASSETS
        sys.path.insert(0, str(assets))
        self.addCleanup(lambda: sys.path.remove(str(assets)))
        for name in ['proof', 'replay']:
            with self.subTest(name=name):
                spec = importlib.util.spec_from_file_location('sqlite_statement_' + name, assets / (name + '.py'))
                module = importlib.util.module_from_spec(spec)
                spec.loader.exec_module(module)
                self.assertTrue(callable(module.run))


class PhysicalProofAdmissionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='sqlite-physical-admission-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.name = Path('SandboxModel/Audit.lean')
        path = self.root / self.name
        path.parent.mkdir()
        path.write_text('theorem fixture : True := True.intro\n')
        (self.root / 'verification/aeneas').mkdir(parents=True)
        for n in ['lakefile.toml', 'lake-manifest.json', 'lean-toolchain']:
            (self.root / n).write_text('fixed\n')
        self.versions = {'server_version': 'fixed-server', 'client_version': 'fixed-client', 'mcp_version': 'fixed-mcp'}
        self.expected = {'Fixture.root'}
        self.record = dict(request=dict(tool='lean_diagnostic_messages', arguments={'file_path': str(self.name)}, expected_sha256=gate.sha(path)),
            stale=False, revision=dict(file_sha256=gate.sha(path), sources_sha256=gate.proof_sources_digest(self.root),
                toolchain='fixed', lakefile_sha256=gate.sha(self.root / 'lakefile.toml'),
                manifest_sha256=gate.sha(self.root / 'lake-manifest.json'), **self.versions),
            result={'result': dict(success=True, partial=False, timed_out=False,
                failed_dependencies=[], still_elaborating_lines=None,
                items=[dict(severity='info',message='Checked Fixture.root: 3 standard logical dependencies')])})

    def check(self, record):
        return gate.check_record(self.root, record, self.expected, self.name, self.versions)

    def test_complete_exact_physical_receipt_is_accepted(self):
        self.assertEqual(self.check(self.record), 1)

    def test_stale_versions_hashes_and_filtered_requests_are_rejected(self):
        mutations = [lambda r: r.update(stale=True),
            lambda r: r['request'].update(tool='lean_run_code'),
            lambda r: r['request']['arguments'].update(severity='error')]
        for key in ['file_sha256', 'sources_sha256', 'server_version', 'client_version', 'mcp_version', 'toolchain', 'lakefile_sha256', 'manifest_sha256']:
            mutations.append(lambda r, key=key: r['revision'].update({key: 'wrong'}))
        for index, mutation in enumerate(mutations):
            record = copy.deepcopy(self.record)
            mutation(record)
            with self.subTest(index=index), self.assertRaises(ValueError):
                self.check(record)

    def test_incomplete_failure_warning_and_audit_omission_are_rejected(self):
        mutations = [lambda x: x.update(success=False), lambda x: x.update(partial=True),
            lambda x: x.update(timed_out=True), lambda x: x.update(failed_dependencies=['missing']),
            lambda x: x.update(still_elaborating_lines=[1]), lambda x: x['items'].clear(),
            lambda x: x['items'].append(copy.deepcopy(x['items'][0])),
            lambda x: x['items'][0].update(severity='warning'),
            lambda x: x['items'][0].update(message='Checked Fixture.root: 4 standard logical dependencies'),
            lambda x: x['items'][0].update(lean_tags=['unsolvedGoals'])]
        for index, mutation in enumerate(mutations):
            record = copy.deepcopy(self.record)
            mutation(record['result']['result'])
            with self.subTest(index=index), self.assertRaises(ValueError):
                self.check(record)

    def test_changed_actual_physical_file_is_rejected(self):
        (self.root / self.name).write_text('changed\n')
        with self.assertRaises(ValueError):
            self.check(self.record)

    def test_outer_service_error_is_rejected(self):
        record = copy.deepcopy(self.record)
        record['result']['isError'] = True
        with self.assertRaises(ValueError):
            self.check(record)


if __name__ == '__main__':
    unittest.main()
