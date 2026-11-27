from dataclasses import asdict
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from tools import ci_e2e_supervisor as e2e
from tools import ci_promotion_acceptance as gate


class PreparationRecoveryTest(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name).resolve()
        self.state_dir = self.root/'e2e'
        self.ci_dir = self.root/'ci'
        self.checkout = self.root/'checkout'
        for d in (self.state_dir, self.ci_dir, self.checkout):
            d.mkdir(mode=0o700)
        self.next = gate.Candidate(681, 'integration', '1'*40, '2'*40, '3'*40, '4'*40)
        self.config = dict(state_root=str(self.state_dir), ci_state_root=str(self.ci_dir),
                           repository=str(self.checkout), ordinary_check_app_id=15368,
                           promotion_floor='5'*40)
        identity = 'a'*64
        unit = 'sandproof-e2e-prepare-' + 'b'*32 + '.service'
        self.old = dict(id=identity, architecture=e2e.service_artifacts.ARCHITECTURE,
            candidate=self.next.base, base='6'*40, prepare_unit=unit, check_id=42,
            configuration={'original': True, 'ci_state_root': str(self.ci_dir)}, phase='blocked',
            reason='candidate_preparation_identity_unknown', check_publish_pending=False,
            candidate_manifest=str(self.state_dir/(identity+'-candidate-artifacts.json')),
            result=dict(candidate=self.next.base, passed=False, tasks=[], executed_task_count=0),
            dk=dict(phase='new'), batches=[dict(names=['fixture'], oracle_mode='original', phase='new')])
        self.state_file = self.state_dir/'state.json'
        self.intent = self.state_dir/(identity+'-prepare.json')
        e2e.write_once(self.state_file, self.old)
        e2e.write_once(self.intent, dict(self.old['configuration'], candidate=self.next.base,
                                       candidate_manifest=self.old['candidate_manifest']))
        e2e.write_once(self.ci_dir/'state.json', {})
        self.failed = dict(id=42, name=e2e.CHECK_NAME, head_sha=self.next.base, external_id=identity,
                           app=dict(id=5175756), status='completed', conclusion='failure')
        self.api = Mock(app_id=5175756)
        self.api.request.return_value = dict(total_count=1, check_runs=[self.failed])
        self.supervisor = e2e.Supervisor(self.config, e2e.Checks(self.api))
        original = {k: self.old[k] for k in ('id', 'candidate', 'base', 'prepare_unit', 'check_id')}
        original['state_sha256'] = e2e.file_digest(self.state_file)
        original['prepare_intent_sha256'] = e2e.file_digest(self.intent)
        self.approval = dict(original=original, next_promotion=asdict(self.next))

    def test_binds_published_failure_and_unchanged_original_intent(self):
        args = self.supervisor, self.approval['original'], self.next, self.api
        self.assertEqual(e2e.check_recovery_origin(*args), e2e.file_digest(self.intent))
        self.failed['conclusion'] = 'success'
        with self.assertRaisesRegex(ValueError, 'failure publication not confirmed'):
            e2e.check_recovery_origin(*args)
        self.failed['conclusion'] = 'failure'
        self.supervisor.state['dk']['phase'] = 'launched'
        with self.assertRaisesRegex(ValueError, 'unstarted batches not confirmed'):
            e2e.check_recovery_origin(*args)

    def test_live_unit_or_host_lease_keeps_old_state(self):
        unit = self.old['prepare_unit']
        with patch.object(e2e.subprocess, 'check_output', side_effect=['active\n']):
            with self.assertRaisesRegex(ValueError, 'stop the E2E supervisor'):
                e2e.require_stopped_preparation(unit, self.ci_dir)
        with patch.object(e2e.subprocess, 'check_output', side_effect=['inactive\n', unit]):
            with self.assertRaisesRegex(ValueError, 'unit or cgroup'):
                e2e.require_stopped_preparation(unit, self.ci_dir)
        e2e.write_once(self.ci_dir/'exclusive-host', {'e2e': self.old['id']})
        with patch.object(e2e.subprocess, 'check_output', side_effect=['inactive\n', '', '']):
            with self.assertRaisesRegex(ValueError, 'host pause'):
                e2e.require_stopped_preparation(unit, self.ci_dir)
        self.assertEqual(e2e.file_digest(self.state_file), self.approval['original']['state_sha256'])

    def test_ready_recovery_keeps_original_exact_bytes_before_queuing_named_pr(self):
        prepared = dict(self.config, checkout=str(self.checkout))
        next_promotion = self.approval['next_promotion']
        archive = self.state_dir/(self.old['id']+'.json')
        original_bytes = self.state_file.read_bytes()

        def queue(head, base, observed_promotion, recovery):
            self.assertEqual(archive.read_bytes(), original_bytes)
            self.assertEqual((head, base), (self.next.head, self.next.base))
            self.assertEqual(observed_promotion, ('integration', 'test', self.next.head))
            self.assertEqual(recovery['next_promotion'], next_promotion)
            self.assertEqual(recovery['original_intent_sha256'], e2e.file_digest(self.intent))
            self.supervisor.state = dict(phase='waiting', candidate=head, base=base,
                                         operator_recovery=recovery)
            self.supervisor.save()

        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(e2e, 'recovery_ready', return_value=True) as ready,
              patch.object(e2e, 'git', return_value=e2e.REPO),
              patch.object(e2e, 'checkout_candidate', return_value=prepared),
              patch.object(e2e, 'is_ancestor', return_value=True),
              patch.object(e2e, 'require_stopped_preparation') as absent,
              patch.object(self.supervisor, 'begin', side_effect=queue)):
            e2e.recover_stopped_preparation(self.supervisor, self.approval, None)
        self.assertEqual(self.supervisor.state['phase'], 'waiting')
        self.assertEqual(ready.call_count, 2)
        self.assertEqual(archive.read_bytes(), original_bytes)
        absent.assert_called_once_with(self.old['prepare_unit'], self.ci_dir)

    def test_no_replacement_when_the_approved_next_identity_changes(self):
        prepared = dict(self.config, checkout=str(self.checkout))
        with (patch.object(gate.Candidate, 'check_checkout'),
              patch.object(e2e, 'recovery_ready', side_effect=[True, False]),
              patch.object(e2e, 'git', return_value=e2e.REPO),
              patch.object(e2e, 'checkout_candidate', return_value=prepared),
              patch.object(e2e, 'is_ancestor', return_value=True),
              patch.object(self.supervisor, 'begin') as begin):
            with self.assertRaisesRegex(ValueError, 'changed during recovery'):
                e2e.recover_stopped_preparation(self.supervisor, self.approval, None)
        begin.assert_not_called()
        self.assertEqual(e2e.file_digest(self.state_file), self.approval['original']['state_sha256'])


if __name__ == '__main__':
    unittest.main()
