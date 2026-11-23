"""Fixtures for the evidence consumer. These do not run either translator."""
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import check_revision_pair as checker
import revision_capture as revision


def declaration(name, identifier, **shape):
    return dict(def_id=identifier, item_meta=dict(opacity='Transparent',
        name=[{'Ident': [part, 0]} for part in name.split('::')]), **shape)


def query_arms(*statements):
    return ''.join(
        '  let result ←\n'
        '    rusqlite.Connection.query_row\n'
        f'      c (toStr\n      "{statement}") params callback\n'
        '  let next ← consume result\n'
        for statement in statements
    )


class RevisionPairTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.archive = b'development archive fixture'
        self.identity = dict(driver_head=checker.DRIVER, head='1' * 40, tree='2' * 40,
            source_archive_sha256=hashlib.sha256(self.archive).hexdigest())
        self.cfg = dict(roots=[revision.PARENT_ROOT], include=[revision.CANDIDATE], opaque=[],
                        namespace='ParentFixture', subdir='ParentFixture')
        self.target = 'x86_64-unknown-linux-gnu'
        self.base = self.make_capture('base-revision', 1)
        self.parent = self.make_capture('parent', 7)

    @staticmethod
    def write_json(path, value):
        path.write_text(json.dumps(value), encoding='utf-8')

    def make_capture(self, mode, identifier):
        directory = self.root / mode
        directory.mkdir()
        cfg = revision.configuration(self.cfg, mode)
        generated = directory / 'generated' / cfg['subdir']
        generated.mkdir(parents=True)
        (generated / 'Types.lean').write_text('  native_revision : Option U64\n', encoding='utf-8')
        funs = query_arms(*([revision.REVISION_SQL] * 6)) if mode == 'base-revision' else ''
        (generated / 'Funs.lean').write_text(funs, encoding='utf-8')
        (directory / 'source.tar').write_bytes(self.archive)
        ir = directory / 'actual.llbc'
        original = declaration(revision.BASE_ROOT, 2, body={'Structured': {'body': {'Str': revision.REVISION_SQL}}})
        call = {'Call': {'call': {'func': {'Regular': {'kind': {'Fun': 2}}}}}}
        parent = declaration(revision.PARENT_ROOT, 3, body={'Structured': {'body': call}})
        options = dict(skip_borrowck=False, no_typecheck=False, remove_unused_clauses=False,
                       monomorphize=False, exclude=[], start_from=cfg['roots'],
                       include=cfg['include'], opaque=cfg['opaque'])
        fields = [dict(name='entry'), dict(name='native_revision')]
        self.write_json(ir, dict(has_errors=False, translated=dict(options=options,
            target_information=[dict(key=self.target)], fun_decls=[original, parent],
            type_decls=[declaration(revision.CANDIDATE, identifier, kind={'Struct': fields})])))
        recorded_dir = Path('/original/run') / mode
        selectors = [arg for flag, key in (('--start-from', 'roots'), ('--include', 'include'), ('--opaque', 'opaque'))
                     for value in cfg[key] for arg in (flag, value)]
        commands = [
            ['/fixed/charon', 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
             *selectors, '--dest-file', str(recorded_dir / ir.name), '--', '--manifest-path',
             'controller-service/Cargo.toml', '--lib', '--locked', '--offline', '--target', self.target],
            ['/fixed/aeneas', '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
             '-emit-json', '-namespace', cfg['namespace'], '-subdir', cfg['subdir'], '-dest',
             str(recorded_dir / 'generated'), str(recorded_dir / ir.name)],
        ]
        rows = []
        for label, argv in zip(('charon', 'aeneas'), commands, strict=True):
            log = directory / (label + '.log')
            log.write_text('fixture', encoding='utf-8')
            rows.append(dict(label=label, argv=argv, exit_code=0, log_sha256=checker.digest(log)))
        self.write_json(directory / 'result.json', dict(self.identity, development_only=True,
            native_acceptance=False, complete=True, commands=rows, ir_sha256=checker.digest(ir),
            generated=checker.generated_inventory(directory / 'generated'),
            revision_admission=revision.admit(ir, directory / 'generated', cfg, self.target, mode)))
        return directory

    def check(self):
        return checker.check_pair(self.base, self.parent, self.identity, self.cfg, self.target)

    def edit_record(self, directory, edit):
        record = directory / 'result.json'
        value = json.loads(record.read_bytes())
        edit(value)
        self.write_json(record, value)

    def test_two_views_read_back_one_source_without_sharing_generated_ids(self):
        pair = self.check()
        self.assertIs(pair['development_pair_checked'], True)
        self.assertIs(pair['native_acceptance'], False)
        self.assertIs(pair['public_acceptance'], False)
        self.assertEqual(pair['source'], self.identity)
        self.assertEqual(pair['views']['base-revision']['admission']['generated_revision_queries'], 6)
        self.assertNotEqual(pair['views']['base-revision']['admission']['ready_candidate_id'],
                            pair['views']['parent']['admission']['ready_candidate_id'])
        with self.assertRaisesRegex(ValueError, 'distinct original capture directories'):
            checker.check_pair(self.base, self.base, self.identity, self.cfg, self.target)

    def test_pair_rejects_claimed_identity_and_archive_drift(self):
        for key in self.identity:
            with self.subTest(key=key):
                self.edit_record(self.parent, lambda record: record.update({key: 'other'}))
                with self.assertRaisesRegex(ValueError, 'driver/source identity'):
                    self.check()
                self.edit_record(self.parent, lambda record: record.update({key: self.identity[key]}))
        (self.parent / 'source.tar').write_bytes(b'other bytes')
        with self.assertRaisesRegex(ValueError, 'archive differs from Git'):
            self.check()

    def test_same_status_does_not_accept_mixed_commands_or_changed_logs(self):
        self.edit_record(self.parent, lambda record: record['commands'][1]['argv'].__setitem__(
            -1, '/original/run/base-revision/actual.llbc'))
        with self.assertRaisesRegex(ValueError, 'another IR'):
            self.check()
        self.edit_record(self.parent, lambda record: record['commands'][1]['argv'].__setitem__(
            -1, '/original/run/parent/actual.llbc'))
        (self.parent / 'charon.log').write_text('wrong original log', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'log changed'):
            self.check()

    def test_rehashed_generated_files_still_need_original_sql_and_parent_call(self):
        funs = next((self.base / 'generated').rglob('Funs.lean'))
        original = funs.read_text(encoding='utf-8')
        # Five correct branches and a decoy literal cannot hide a wrong sixth one.
        wrong = query_arms(*([revision.REVISION_SQL] * 5), 'SELECT no_revision FROM sandboxes')
        funs.write_text(wrong + f'-- "{revision.REVISION_SQL}"\n', encoding='utf-8')
        self.edit_record(self.base, lambda result: result.update(
            generated=checker.generated_inventory(self.base / 'generated')))
        with self.assertRaisesRegex(ValueError, 'revision SQL differs'):
            self.check()
        funs.write_text(original, encoding='utf-8')
        self.edit_record(self.base, lambda result: result.update(
            generated=checker.generated_inventory(self.base / 'generated')))
        ir = self.parent / 'actual.llbc'
        raw = json.loads(ir.read_bytes())
        raw['translated']['fun_decls'][1]['body']['Structured']['body'] = {}
        self.write_json(ir, raw)
        self.edit_record(self.parent, lambda result: result.update(ir_sha256=checker.digest(ir)))
        with self.assertRaisesRegex(ValueError, 'no longer calls its original base_snapshot once'):
            self.check()

    def test_rehashed_base_still_requires_every_generated_query_and_the_original_literal(self):
        funs = next((self.base / 'generated').rglob('Funs.lean'))
        original = funs.read_text(encoding='utf-8')
        funs.write_text(query_arms(*([revision.REVISION_SQL] * 5)), encoding='utf-8')
        self.edit_record(self.base, lambda result: result.update(
            generated=checker.generated_inventory(self.base / 'generated')))
        with self.assertRaisesRegex(ValueError, 'revision query count differs'):
            self.check()
        funs.write_text(original, encoding='utf-8')
        ir = self.base / 'actual.llbc'
        raw = json.loads(ir.read_bytes())
        raw['translated']['fun_decls'][0]['body']['Structured']['body']['Str'] = 'SELECT bad'
        self.write_json(ir, raw)
        self.edit_record(self.base, lambda result: result.update(
            ir_sha256=checker.digest(ir),
            generated=checker.generated_inventory(self.base / 'generated')))
        with self.assertRaisesRegex(ValueError, 'original base_snapshot revision SQL differs'):
            self.check()

    def test_unlisted_output_is_not_part_of_the_record(self):
        (self.parent / 'generated' / 'foreign.lean').write_text('fixture', encoding='utf-8')
        with self.assertRaisesRegex(ValueError, 'complete generated file inventory changed'):
            self.check()

    def test_source_archive_is_independently_rebuilt_from_git(self):
        repository = self.root / 'git'
        repository.mkdir()
        with patch.object(checker, 'ROOT', repository):
            checker.git('init', '-q')
            checker.git('config', 'tar.umask', 'user')
            source = repository / revision.REVISION_SOURCE
            source.parent.mkdir(parents=True)
            source.write_bytes(b'reviewed source fixture')
            checker.git('add', '.')
            subprocess.run(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                'commit', '-q', '--no-gpg-sign', '-m', 'original source'], cwd=repository, check=True)
            head = checker.git('rev-parse', 'HEAD').decode().strip()
            with patch.object(revision, 'REVISION_SOURCE_SHA256', checker.digest(source)):
                observed = checker.source_identity(head)
            raw = subprocess.check_output(['git', 'archive', head], cwd=repository, umask=0o077)
            with tarfile.open(fileobj=io.BytesIO(raw), mode='r:') as archive:
                self.assertEqual(archive.getmember(revision.REVISION_SOURCE.as_posix()).mode, 0o600)
            self.assertEqual(observed['source_archive_sha256'], hashlib.sha256(raw).hexdigest())
            self.assertEqual(observed['tree'], checker.git('rev-parse', 'HEAD^{tree}').decode().strip())


if __name__ == '__main__':
    unittest.main()
