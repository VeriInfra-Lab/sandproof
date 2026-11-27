from contextlib import nullcontext
from dataclasses import asdict
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools import ci_promotion_acceptance as gate
from tools import formal_builds, native_task


def sha(digit):
    return digit * 40


class GitHub:
    app_id = gate.GATE_APP

    def __init__(self):
        self.rows = {}
        self.commits = {}
        self.posts = []

    def request(self, method, path, body=None):
        group, *rest = path.removeprefix(gate.PREFIX).split('/')
        if method == 'GET' and group == 'git':
            return self.commits[rest[1]]
        if method == 'GET' and group == 'commits':
            rs = self.rows.get(rest[0], [])
            return dict(total_count=len(rs), check_runs=rs)
        if method == 'POST' and group == 'check-runs':
            self.posts.append(body)
            return self.check(body['head_sha'], body['name'], **{
                k: body[k] for k in ('status', 'conclusion', 'output', 'external_id')})
        raise AssertionError((method, path))

    def check(self, head, name, **extra):
        rows = self.rows.setdefault(head, [])
        row = dict(id=sum(map(len, self.rows.values())) + 1, head_sha=head, name=name,
                   status='completed', conclusion='success', app=dict(id=self.app_id))
        row.update(extra)
        rows.append(row)
        return row


class PromotionTest(unittest.TestCase):
    def setUp(self):
        self.addCleanup(os.umask, os.umask(0o077))
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name).resolve()
        self.observer = self.path/'installed-observer'
        self.observer.write_text('the tests override only host observation\n')
        self.candidate = gate.Candidate(681, 'integration', sha('a'), sha('b'), sha('c'), sha('d'))
        self.api = GitHub()
        self.group = gate.QueueGroup('refs/heads/gh-readonly-queue/test/known', sha('e'),
                                    self.candidate.base)
        self.queue = dict(entry='queue-entry', pull=self.candidate.pull, source=self.candidate.source,
                          head=self.candidate.head, base=self.candidate.base, tree=self.candidate.tree)
        self.evidence = {'native': 'original-native', 'formal': 'original-formal'}
        self.actions_app = 15368
        for job in ('sdk-contract', 'harbor-inputs', 'pr-title', 'pr-policy'):
            self.ordinary(job)
            self.api.check(self.group.sha, job, app=dict(id=self.actions_app))
        self.e2e = self.api.check(self.candidate.head, gate.E2E, output=dict(text=json.dumps(dict(
            schema=1, source=self.candidate.source, target='test', head=self.candidate.head,
            base=self.candidate.base, tree=self.candidate.tree))))

    def ordinary(self, job, **overrides):
        metadata = dict(app=dict(id=self.actions_app), check_suite=dict(id=123842787037),
            pull_requests=[dict(number=self.candidate.pull, head=dict(sha=self.candidate.head),
                                base=dict(sha=self.candidate.base))])
        return self.api.check(self.candidate.head, job, **dict(metadata, **overrides))

    def admit(self, policy=lambda: None):
        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(gate.Candidate, 'observe'),
              patch.object(gate, 'read_stage', return_value=self.evidence)):
            gate.admit(self.candidate, self.path, self.path, api=self.api, source_client=None,
                       proxy=None, directory=self.path, policy=policy)

    def publish(self, policy=lambda: None):
        with (patch.object(gate.QueueGroup, 'observe', return_value=self.queue),
              patch.object(gate, 'read_stage', return_value=self.evidence)):
            gate.publish(self.candidate, self.path, self.path, group=self.group, api=self.api,
                         lookup=None, commit=None, directory=self.path, policy=policy)

    def test_bind_live_protected_refs_together(self):
        refs = self.candidate.refs()
        original = '\n'.join(f'{value}\t{ref}' for value, ref in zip(
            (self.candidate.head, self.candidate.base, self.candidate.merge), refs))
        def branch(ref, commit):
            return dict(ref=ref, sha=commit, repo=dict(full_name='VeriInfra-Lab/sandproof'))
        pull = Mock(return_value=dict(number=681, state='open', draft=False, mergeable=True,
            merge_commit_sha=self.candidate.merge, head=branch('integration', self.candidate.head),
            base=branch('test', self.candidate.base)))
        with (patch.object(gate, 'git', return_value=original) as remote,
              patch.object(gate, 'Transport', return_value=pull)):
            self.candidate.observe(source_client=None, proxy=None)
            self.assertEqual(remote.call_args.args[2:], refs)
            for index in range(3):
                remote.return_value = original.replace((sha('a'), sha('b'), sha('c'))[index], sha('e'))
                with self.assertRaisesRegex(ValueError, 'moved'):
                    self.candidate.observe(source_client=None, proxy=None)
            remote.return_value = original
            pull.return_value['draft'] = True
            with self.assertRaisesRegex(ValueError, 'no longer an open protected promotion'):
                self.candidate.observe(source_client=None, proxy=None)

    def test_no_gate_on_newer_pending_failed_or_wrong_app_ordinary(self):
        self.ordinary('sdk-contract', status='in_progress', conclusion=None)
        self.ordinary('sdk-contract', app=dict(id=100))
        with self.assertRaisesRegex(ValueError, 'ordinary checks'):
            self.admit()
        self.ordinary('sdk-contract', conclusion='failure')
        with self.assertRaisesRegex(ValueError, 'ordinary checks'):
            self.admit()
        self.assertEqual(self.api.posts, [])

    def test_unrelated_pr_or_mixed_attempts_do_not_supply_missing_ordinary_evidence(self):
        del self.api.rows[self.candidate.head][0]
        self.ordinary('sdk-contract', pull_requests=[])
        with self.assertRaisesRegex(ValueError, 'ordinary checks'):
            self.admit()
        self.ordinary('sdk-contract', check_suite=dict(id=999999))
        with self.assertRaisesRegex(ValueError, 'ordinary checks'):
            self.admit()
        self.assertEqual(self.api.posts, [])

    def test_no_gate_on_pending_failed_wrong_head_or_wrong_base_e2e(self):
        for key, value, reason in (('status', 'in_progress', 'existing E2E is pending'),
                ('conclusion', 'failure', 'existing E2E is pending'),
                ('head_sha', sha('e'), 'existing E2E is pending'),
                ('output', {'text': '{}'}, 'bound to a different source')):
            with self.subTest(key=key):
                original = self.e2e[key]
                self.e2e[key] = value
                with self.assertRaisesRegex(ValueError, reason):
                    self.admit()
                self.e2e[key] = original
        self.assertEqual(self.api.posts, [])

    def test_publish_live_queue_and_source_admission_idempotently(self):
        self.admit()
        self.admit()
        self.publish()
        self.publish()
        self.assertEqual([(row['name'], row['head_sha']) for row in self.api.posts],
                         [(gate.GATE, self.candidate.head), (gate.REFERENCE, self.candidate.head),
                          (gate.E2E, self.group.sha), (gate.GATE, self.group.sha)])
        self.assertEqual(self.api.posts[1]['external_id'], self.api.posts[3]['external_id'])
        self.assertEqual(json.loads(self.api.posts[1]['output']['summary'])['candidate'],
                         asdict(self.candidate))
        self.assertEqual(json.loads(self.api.posts[2]['output']['summary'])['e2e_check_id'], self.e2e['id'])
        self.assertEqual(len(json.loads(self.api.posts[3]['output']['summary'])['ordinary_check_ids']), 4)
        self.assertEqual(json.loads(self.api.posts[3]['output']['summary'])['queue_ordinary_check_ids'],
                         {row['name']: row['id'] for row in self.api.rows[self.group.sha]
                          if row['app']['id'] == self.actions_app})

    def test_moved_target_before_publication_raises_without_post(self):
        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(gate.Candidate, 'observe', side_effect=[None, ValueError('base moved')]),
              patch.object(gate, 'read_stage', return_value=self.evidence)):
            with self.assertRaisesRegex(ValueError, 'base moved'):
                gate.admit(self.candidate, self.path, self.path, api=self.api, source_client=None,
                           proxy=None, directory=self.path, policy=lambda: None)
        self.assertEqual(self.api.posts, [])

    def test_lost_check_post_never_repeats_without_readback(self):
        api = Mock(app_id=gate.GATE_APP)
        api.request.side_effect = [dict(total_count=0, check_runs=[]), gate.APIError('unavailable'),
                                   dict(total_count=0, check_runs=[])]
        args = (api, self.path, {'trusted': 'receipt'})
        with self.assertRaises(gate.APIError):
            gate.publish_check(*args, name=gate.GATE, sha=self.candidate.merge)
        with self.assertRaises(FileExistsError):
            gate.publish_check(*args, name=gate.GATE, sha=self.candidate.merge)
        self.assertEqual([call.args[0] for call in api.request.call_args_list], ['GET', 'POST', 'GET'])

    def test_only_exact_push_skips_workspace(self):
        self.admit()
        self.publish()
        actual_merge = sha('f')  # GitHub may give the final commit a new message/author.
        self.api.commits[actual_merge] = dict(sha=actual_merge, tree=dict(sha=self.candidate.tree),
            parents=[dict(sha=self.candidate.base), dict(sha=self.candidate.head)])
        event = dict(ref='refs/heads/test', before=self.candidate.base, after=actual_merge,
                     repository=dict(full_name='VeriInfra-Lab/sandproof'), forced=False, deleted=False)
        self.assertTrue(gate.equivalent_push(event, self.api))
        self.assertFalse(gate.equivalent_push(dict(event, before=sha('1')), self.api))
        self.assertFalse(gate.equivalent_push(dict(event, ref='refs/heads/main'), self.api))
        self.assertFalse(gate.equivalent_push(dict(event, forced=True), self.api))
        self.api.commits[actual_merge]['tree']['sha'] = sha('2')
        self.assertFalse(gate.equivalent_push(event, self.api))
        self.api.commits[actual_merge]['tree']['sha'] = self.candidate.tree
        self.api.check(self.group.sha, gate.GATE, conclusion='failure')
        self.assertFalse(gate.equivalent_push(event, self.api))

    def test_changed_upstream_attempt_during_publication_does_not_pass_gate(self):
        self.admit()
        checked = gate.promotion_inputs(self.api, self.candidate, self.actions_app)
        replacement = (dict(checked[0], **{'pr-title': 42}), checked[1])
        with patch.object(gate, 'promotion_inputs', side_effect=[checked, replacement]):
            with self.assertRaisesRegex(ValueError, 'ordinary checks or E2E changed'):
                self.publish()
        self.assertFalse(any(row['name'] == gate.GATE and row['head_sha'] == self.group.sha
                             for row in self.api.posts))

    def test_publication_rejects_wrong_tree_missing_admission_or_inactive_protection(self):
        blocked = Mock(side_effect=ValueError('queue inactive'))
        with self.assertRaisesRegex(ValueError, 'queue inactive'):
            self.admit(blocked)
        with self.assertRaisesRegex(ValueError, 'matching successful Stage1 queue admission'):
            self.publish()
        self.admit()
        self.queue['tree'] = sha('f')
        with self.assertRaisesRegex(ValueError, 'queue PR, base or tree differs'):
            self.publish()
        self.assertEqual([(row['name'], row['head_sha']) for row in self.api.posts],
                         [(gate.GATE, self.candidate.head)])

    def test_queue_requires_literal_success_from_latest_native_check_runs(self):
        self.admit()
        saved = self.api.rows[self.group.sha][:]
        cases = [
            ('absent', None), ('wrong app', {'app': {'id': 123}}),
            ('wrong SHA', {'head_sha': self.candidate.head}),
            ('skipped', {'conclusion': 'skipped'}), ('neutral', {'conclusion': 'neutral'}),
            ('cancelled', {'conclusion': 'cancelled'}), ('failed', {'conclusion': 'failure'}),
            ('running', {'status': 'in_progress', 'conclusion': None}),
        ]
        for label, update in cases:
            with self.subTest(label=label):
                if label in ('absent', 'wrong app', 'wrong SHA'):
                    fresh = [row for row in saved if row['name'] != 'pr-policy']
                else:
                    fresh = saved[:]
                self.api.rows[self.group.sha] = fresh
                if update is not None:
                    self.api.check(self.group.sha, 'pr-policy',
                                   **dict({'app': {'id': self.actions_app}}, **update))
                with self.assertRaisesRegex(ValueError, 'all four current Actions checks'):
                    self.publish()
        self.assertEqual([(row['name'], row['head_sha']) for row in self.api.posts],
                         [(gate.GATE, self.candidate.head)])

    def test_changed_queue_check_blocks_e2e_after_reference_and_gate_after_e2e(self):
        self.admit()
        original = gate.publish_check

        def publish_and_restart(*args, name, sha):
            original(*args, name=name, sha=sha)
            if name == pivot:
                self.api.check(self.group.sha, 'pr-title', app=dict(id=self.actions_app),
                               status='in_progress', conclusion=None)

        for pivot, message in ((gate.REFERENCE, 'changed before E2E publication'),
                               (gate.E2E, 'ordinary checks or E2E changed during publication')):
            with self.subTest(after=pivot):
                with patch.object(gate, 'publish_check', side_effect=publish_and_restart):
                    with self.assertRaisesRegex(ValueError, message):
                        self.publish()
                self.assertFalse(any(row['head_sha'] == self.group.sha and row['name'] == gate.GATE
                                     for row in self.api.posts))
                if pivot == gate.REFERENCE:
                    self.assertFalse(any(row['head_sha'] == self.group.sha for row in self.api.posts))
                self.api.rows[self.group.sha].pop()

    def formal_fixture(self, trailing='exit 0'):
        launch = formal_builds.execute

        def execute(output, command, *, cwd, env, pass_fds):
            self.assertEqual(command, ['bash', 'scripts/verify-stage1-protocol.sh'])
            self.assertNotIn('GH_TOKEN', env)
            self.assertEqual(env['CARGO_TARGET_DIR'], str(output/'formal-cargo-target'))
            self.assertEqual(pass_fds[0], 9)
            script = r'''
set -eu
phase="$TMPDIR/sandproof-stage1-protocol.abcdef/registered"
mkdir -p "$phase/execution/retained/cargo-target" "$phase/actual/source/.lake"
printf '{}\n' > "$phase/registered-result.json"
printf '{}\n' > "$phase/execution-inputs.json"
printf 'per-job build\n' > "$phase/execution/retained/cargo-target/keep"
printf 'Lean cache\n' > "$phase/actual/source/.lake/keep"
printf 'build\n' > "$CARGO_TARGET_DIR/private-build"
printf 'original formal executor log\n'
'''
            # The caller's fd transport is checked separately from the private subprocess fixture.
            return launch(output, ['bash', '-c', script + trailing], cwd=cwd, env=env)

        return execute

    @staticmethod
    def formal_receipt(native, formal, _candidate, _checkout):
        return dict(native=gate.digest(native/'complete.json'), formal=gate.digest(formal/'registered-result.json'),
                    context=gate.digest(formal/'execution-inputs.json'))

    def test_original_full_formal_consumes_exact_one_new_or_existing_native_seal(self):
        existing = self.path/'existing-native'
        checkout = self.path/'checkout'
        cache = self.path/'shared-native-target'
        checkout.mkdir(mode=0o700)
        cache.mkdir(mode=0o755)
        existing.mkdir(mode=0o700)
        (existing/'complete.json').write_text('{}')
        (existing/'complete.json').chmod(0o600)
        for prior in (None, existing):
            with self.subTest(existing=bool(prior)):
                output = self.path / ('reuse' if prior else 'fresh')
                output.mkdir(mode=0o700)

                def ordinary(command, *, log, checkout, env, pass_fds):
                    self.assertEqual(pass_fds, (9, 37))
                    self.assertNotIn('GH_TOKEN', env)
                    self.assertIn('tools/native_task.py', command)
                    self.assertIn('stage1-protocol', command)
                    self.assertEqual(env['TMPDIR'], '/mnt/xfs')
                    self.assertEqual(env['CARGO_TARGET_DIR'], str(cache))
                    (output/'native').mkdir(mode=0o700)
                    gate.write_once(output/'native'/'complete.json', {})

                with (patch.object(gate.Candidate, 'check_checkout'),
                      patch.object(gate, 'MOUNT', self.path),
                      patch.object(gate, 'git', return_value=self.candidate.head),
                      patch.object(gate, 'operator_lock', return_value=nullcontext(9)),
                      patch.object(gate, 'attest_evidence', side_effect=self.formal_receipt),
                      patch.object(gate, 'check_stage1_inputs') as preflight,
                      patch.object(gate, 'run_entry', side_effect=ordinary) as entries,
                      patch.object(formal_builds, 'execute', side_effect=self.formal_fixture()) as formal,
                      patch.object(native_task, 'observe'),
                      patch.dict(os.environ, dict(GH_TOKEN='test-only', TMPDIR='/mnt/xfs',
                                                 CARGO_TARGET_DIR=str(cache),
                                                 SANDPROOF_NATIVE_OBSERVER=str(self.observer)))):
                    result = gate.run_stage(self.candidate, checkout, output, native=prior, registration_fd=37)
                self.assertEqual(entries.call_count, 0 if prior else 1)
                self.assertEqual(preflight.call_count, 0 if prior else 1)
                formal.assert_called_once()
                self.assertEqual(formal.call_args.kwargs['pass_fds'], (9, 37))
                self.assertEqual(formal.call_args.kwargs['cwd'], checkout)
                self.assertEqual(formal.call_args.kwargs['env']['SANDPROOF_NATIVE_EVIDENCE'],
                                 str(prior or output/'native'))
                self.assertEqual(result['candidate'], asdict(self.candidate))
                self.assertTrue((output/'complete.json').is_file())
                self.assertFalse((output/'formal-cargo-target').exists())
                formal_output = Path(result['formal_path'])
                self.assertTrue((formal_output/'execution/retained/cargo-target/keep').is_file())
                self.assertTrue((formal_output/'actual/source/.lake/keep').is_file())

    def test_stale_stage1_registration_stops_before_the_native_gate(self):
        checkout, output, cache = (self.path / name for name in ('checkout', 'run', 'shared-target'))
        for path in (checkout, output, cache):
            path.mkdir(mode=0o700)
        failure = subprocess.CalledProcessError(1, ['register'])
        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(gate, 'MOUNT', self.path),
              patch.object(gate, 'git', return_value=self.candidate.head),
              patch.object(gate, 'operator_lock') as lock,
              patch.object(gate, 'run_entry', side_effect=(None, failure)) as execute,
              patch.dict(os.environ, dict(CARGO_TARGET_DIR=str(cache),
                                         SANDPROOF_NATIVE_OBSERVER=str(self.observer)))):
            with self.assertRaises(subprocess.CalledProcessError):
                gate.run_stage(self.candidate, checkout, output)
        self.assertEqual([call.kwargs['log'].name for call in execute.call_args_list],
                         ['stage1-freeze.log', 'stage1-registration.log'])
        lock.assert_not_called()
        self.assertFalse((output/'native').exists())
        self.assertFalse((output/'complete.json').exists())

    def test_native_target_alias_cannot_hide_builds_in_candidate_or_evidence(self):
        checkout = self.path/'checkout'
        checkout.mkdir(mode=0o700)
        for index in range(2):
            output = self.path/f'run-{index}'
            output.mkdir(mode=0o700)
            target = (checkout if index == 0 else output)/'target'
            target.mkdir(mode=0o700)
            alias = self.path/f'alias-{index}'
            alias.symlink_to(target)
            with (patch.object(gate.Candidate, 'check_checkout'),
                  patch.object(gate, 'MOUNT', self.path),
                  patch.object(gate, 'git', return_value=self.candidate.head),
                  patch.object(gate, 'run_entry') as execute,
                  patch.dict(os.environ, dict(CARGO_TARGET_DIR=str(alias),
                                             SANDPROOF_NATIVE_OBSERVER=str(self.observer)))):
                with self.assertRaisesRegex(ValueError, 'separate from source and evidence'):
                    gate.run_stage(self.candidate, checkout, output)
            execute.assert_not_called()

    def test_resume_uses_bound_native_seal_and_original_partial_formal_record(self):
        old = self.path/'interrupted'
        new = self.path/'resumed'
        native = old/'native'
        formal = old/'tmp'/'sandproof-stage1-protocol.old'/'registered'
        native.mkdir(parents=True, mode=0o700)
        formal.mkdir(parents=True, mode=0o700)
        old.chmod(0o700)
        new.mkdir(mode=0o700)
        seal = native/'complete.json'
        gate.write_once(seal, {'native': 'complete'})
        gate.write_once(old/'attempt.json', dict(schema=1, candidate=asdict(self.candidate),
                                                native_path=str(native)))
        gate.write_once(old/'native-seal.json', {'sha256': gate.digest(seal)})
        prior = formal/'registered-result.json'
        gate.write_once(prior, dict(profile='full', shared_seal_sha256=gate.digest(seal),
                                    complete=False))

        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(gate, 'MOUNT', self.path),
              patch.object(gate, 'operator_lock', return_value=nullcontext(9)),
              patch.object(gate, 'git', return_value=self.candidate.head),
              patch.object(gate, 'attest_evidence', side_effect=self.formal_receipt),
              patch.object(gate, 'run_entry') as ordinary,
              patch.object(native_task, 'observe'),
              patch.dict(os.environ, SANDPROOF_NATIVE_OBSERVER=str(self.observer)),
              patch.object(formal_builds, 'execute', side_effect=self.formal_fixture()) as command):
            gate.run_stage(self.candidate, self.path, new, resume_from=old)
            self.assertEqual(command.call_count, 1)
            ordinary.assert_not_called()
            env = command.call_args.kwargs['env']
            self.assertEqual(env['SANDPROOF_STAGE1_RESUME_FROM'], str(prior))
            self.assertEqual(env['SANDPROOF_NATIVE_EVIDENCE'], str(native))
            rejected = gate.Candidate(682, 'integration', self.candidate.head,
                                      self.candidate.base, self.candidate.merge, self.candidate.tree)
            with self.assertRaisesRegex(ValueError, 'same PR'):
                gate.previous_attempt(rejected, old)
            seal.write_text('{}')
            with self.assertRaisesRegex(ValueError, 'original native seal changed'):
                gate.previous_attempt(self.candidate, old)

    def test_pending_formal_closeout_uses_original_registration_and_does_not_relaunch(self):
        output, native, registry = (self.path/name for name in ('output', 'native', 'registry'))
        for directory in (output, native, registry):
            directory.mkdir()
        gate.write_once(native/'complete.json', {})
        original = output.stat()
        gate.write_once(registry/'attempt-00000001.json', dict(schema=1, candidate=asdict(self.candidate),
            output=str(output), output_identity=[original.st_dev, original.st_ino]))
        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(gate, 'MOUNT', self.path),
              patch.object(gate, 'git', return_value=self.candidate.head),
              patch.object(gate, 'operator_lock', return_value=nullcontext(9)),
              patch.object(gate, 'registered_promotion', return_value=nullcontext((registry, 37))) as mutex,
              patch.object(gate, 'attest_evidence', side_effect=self.formal_receipt),
              patch.object(gate, 'run_entry') as ordinary,
              patch.dict(os.environ, SANDPROOF_NATIVE_OBSERVER=str(self.observer)),
              patch.object(formal_builds, 'execute', side_effect=self.formal_fixture()) as formal):
            with patch.object(native_task, 'observe', side_effect=ValueError('live target reference')):
                with self.assertRaises(subprocess.CalledProcessError) as error:
                    gate.run_stage(self.candidate, self.path, output, native=native, registration_fd=37)
            self.assertEqual(error.exception.returncode, 75)
            self.assertTrue((output/'formal-ready.json').is_file())
            self.assertTrue((output/'formal-cargo-target/private-build').is_file())
            self.assertFalse((output/'complete.json').exists())
            with self.assertRaisesRegex(ValueError, 'close out the original'):
                gate.previous_attempt(self.candidate, output)
            with patch.object(native_task, 'observe'):
                result = gate.closeout_registered(self.candidate, self.path, output)
            self.assertEqual(result, gate.json_file(output/'complete.json'))
            self.assertFalse((output/'formal-cargo-target').exists())
            mutex.assert_called_once_with(self.candidate)
            ordinary.assert_not_called()
            formal.assert_called_once()

    def test_waited_formal_exit_zero_requires_closeout_even_before_ready_publication(self):
        output = self.path/'output'
        output.mkdir()
        gate.write_once(output/'attempt.json', dict(schema=1, candidate=asdict(self.candidate),
                                                    native_path=str(self.path/'sealed-native')))
        formal_builds.enroll(output, candidate=self.candidate.head, observer=self.observer,
                             targets=(gate.FORMAL_TARGET,))
        exit_record = formal_builds.execute(output, ['bash', '-c', 'exit 0'])
        self.assertEqual(exit_record['returncode'], 0)
        self.assertFalse((output/'formal-ready.json').exists())
        with patch.object(gate, 'MOUNT', self.path):
            with self.assertRaisesRegex(ValueError, 'close out the original'):
                gate.previous_attempt(self.candidate, output)

    def test_formal_failure_preserves_original_status_and_sigkill_keeps_target(self):
        native = self.path/'native'
        native.mkdir()
        gate.write_once(native/'complete.json', {})
        for trailing, raw, shell in (('exit 19', 19, 19), ('kill -TERM "$$"', -15, 143), ('exit 137', 137, 137)):
            with self.subTest(code=raw):
                output = self.path/str(raw)
                output.mkdir()
                with (patch.object(gate.Candidate, 'check_checkout'),
                      patch.object(gate, 'MOUNT', self.path),
                      patch.object(gate, 'git', return_value=self.candidate.head),
                      patch.object(gate, 'operator_lock', return_value=nullcontext(9)),
                      patch.object(gate, 'attest_evidence') as attest,
                      patch.object(gate, 'run_entry') as ordinary,
                      patch.object(native_task, 'observe'),
                      patch.dict(os.environ, SANDPROOF_NATIVE_OBSERVER=str(self.observer)),
                      patch.object(formal_builds, 'execute', side_effect=self.formal_fixture(trailing))):
                    with self.assertRaises(subprocess.CalledProcessError) as error:
                        gate.run_stage(self.candidate, self.path, output, native=native)
                self.assertEqual(error.exception.returncode, shell)
                closed = output/formal_builds.DIRECTORY
                self.assertEqual(gate.json_file(closed/'native-task-exit.json')['returncode'], raw)
                self.assertIn('original formal executor log', (closed/'native-task.log').read_text())
                self.assertFalse((output/'complete.json').exists())
                self.assertFalse((output/'formal-ready.json').exists())
                self.assertEqual((output/'formal-cargo-target/private-build').exists(), raw == 137)
                attest.assert_not_called()
                ordinary.assert_not_called()

    def test_full_receipt_rejects_finite_failed_mismatched_or_missing_evidence(self):
        def save(path, value):
            path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
            path.write_text(json.dumps(value))
            path.chmod(0o600)
            return dict(path=str(path), sha256=gate.digest(path))

        native, formal = self.path/'native', self.path/'proof'/'registered'
        snapshot = gate.receipts.digest({})
        functional = save(native/'commands'/'one'/'receipt.json', dict(key=dict(label='functional')))
        original = save(native/'complete.json', dict(scope=gate.SCOPE, complete=True,
            scope_complete=True, exit_code=0, repository_sha256=snapshot, runtime={'rust': 'approved'},
            commands={'one': functional['sha256']}))
        save(formal/'execution-inputs.json', dict(source_tree=self.candidate.tree, profile='full',
            native=dict(seal_sha256=original['sha256']), runtime={'lean': 'approved'}))
        retained = save(formal.parent/'retained'/'result.json', {'original': True})
        result = save(formal/'component'/'result.json', {'original': True})
        checkpoint = save(formal/'component'/'checkpoint.json', {'original': True})
        report = dict(kind='registered-stage1-formal-drivers', profile='full', complete=True,
            execution_failures={}, shared_seal_sha256=original['sha256'], source_snapshot_sha256=snapshot,
            template_audits={'missing': []}, native_receipts={'functional': functional},
            retained_result=retained, results=[dict(name='component', result_path=result['path'],
                result_sha256=result['sha256'], checkpoint_path=checkpoint['path'],
                checkpoint_sha256=checkpoint['sha256'])])
        form = formal/'registered-result.json'
        save(form, report)
        with (patch.object(gate, 'MOUNT', self.path),
              patch.object(gate.receipts, 'repository_inputs', return_value={}),
              patch.object(gate.receipts, 'load_success', return_value=True)):
            args = native, formal, self.candidate, self.path
            self.assertEqual(gate.attest_evidence(*args)['native'], original['sha256'])
            for patch_fields in (dict(profile='protocol-convergence'), dict(complete=False),
                                 dict(shared_seal_sha256='another'), dict(execution_failures={'driver': 'failed'})):
                save(form, dict(report, **patch_fields))
                with self.assertRaisesRegex(ValueError, 'full formal result'):
                    gate.attest_evidence(*args)
            save(form, report)
            save(Path(checkpoint['path']), {'altered': True})
            with self.assertRaisesRegex(ValueError, 'missing or changed: checkpoint'):
                gate.attest_evidence(*args)

    def test_protected_readback_runs_installed_code_as_evidence_owner_without_app_environment(self):
        completed = Mock(stdout=json.dumps(self.evidence))
        account = Mock(pw_gid=1001, pw_dir='/home/evidence')
        with (patch.object(gate.pwd, 'getpwuid', return_value=account),
              patch.object(gate.os, 'geteuid', return_value=0),
              patch.object(gate.subprocess, 'run', return_value=completed) as run,
              patch.dict(os.environ, dict(CREDENTIALS_DIRECTORY='/run/credentials/secret-controller'))):
            self.assertEqual(gate.operator_readback(self.candidate, self.path, self.path, 1001),
                             self.evidence)
        params = run.call_args.kwargs
        self.assertEqual(params['cwd'], gate.ROOT)
        self.assertEqual(params['user'], 1001)
        self.assertEqual(params['group'], 1001)
        self.assertEqual(params['extra_groups'], [])
        self.assertEqual(params['env'], dict(PATH='/usr/bin:/bin', HOME='/home/evidence'))
        self.assertEqual(json.loads(params['input']), asdict(self.candidate))


if __name__ == '__main__':
    unittest.main()
