import unittest
from tools.generic_capacity import GIB, MIB, plan


class GenericCapacityTests(unittest.TestCase):
    def tasks(self, count):
        return [{'name':str(i),'requirements':{'cpus':4,'memory_mib':8192,'disk_mib':10240,
                                             'build_timeout_sec':600}} for i in range(count)]

    def test_full_set_has_room_for_each_candidate_and_two_runtime_identities(self):
        p=plan(self.tasks(89),1,32*GIB,44649272)
        self.assertGreater(p['uid_stride'], 2*89)
        self.assertEqual(p['registry_bytes'],89*(10*GIB+44649272))
        self.assertGreater(p['required_free_bytes'],p['registry_bytes']+32*GIB+20*GIB)
        self.assertEqual(p['jobs'],89)
        self.assertEqual(p['max_disk_mib'],10240)
        self.assertEqual(p['max_build_seconds'],600)

    def test_repetition_doubles_retained_artifacts_and_identity_demand(self):
        p=plan(self.tasks(89),1,32*GIB,44649272)
        twice=plan(self.tasks(89),2,32*GIB,44649272)
        self.assertEqual(twice['registry_bytes'],2*p['registry_bytes'])
        self.assertGreater(twice['uid_stride'],2*178)
        self.assertGreater(twice['required_free_bytes'],p['required_free_bytes'])

    def test_trial_budget_and_retained_original_contexts_are_additive(self):
        tasks = self.tasks(89)
        tasks[0]['files'] = [{'role': 'environment', 'bytes': 123456, 'path': 'environment/Dockerfile'},
                             {'role': 'oracle', 'bytes': GIB, 'path': 'solution/private'}]
        p = plan(tasks, 2, 32 * GIB, 44649272, artifact_bytes=128 * MIB)
        larger = plan(tasks, 2, 32 * GIB, 44649272, artifact_bytes=256 * MIB)
        self.assertEqual(larger['required_free_bytes'] - p['required_free_bytes'], 178 * 128 * MIB)
        self.assertEqual(p['context_reserve_bytes'], 178 * 10240 + 2 * (123456 + len('environment/Dockerfile') + 4096))
        self.assertEqual(p['trial_evidence_bytes'], 178 * 132 * MIB)

    def test_invalid_shapes_are_rejected_without_silent_clamping(self):
        for field,value in [('cpus',True),('memory_mib',32768),('disk_mib',0),
                            ('build_timeout_sec',3601),('build_timeout_sec',True),
                            ('build_timeout_sec',float('nan')),('build_timeout_sec','600')]:
            tasks=self.tasks(1);tasks[0]['requirements'][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):plan(tasks,1,32*GIB,44649272)
        with self.assertRaises(ValueError):plan(self.tasks(1)*2,1,32*GIB,44649272)

    def test_larger_public_import_budgets_reserve_staging_without_changing_task(self):
        original = plan(self.tasks(1), 1, 64 * GIB, 44649272)
        large = plan(self.tasks(1), 1, 64 * GIB, 44649272,
                     input_bytes=8 * GIB, expanded_bytes=32 * GIB)
        self.assertEqual(large['input_bytes'], 8 * GIB)
        self.assertEqual(large['expanded_bytes'], 32 * GIB)
        self.assertEqual(large['worker_required_free_bytes'], 58 * GIB + 64 * MIB)
        self.assertEqual(large['workspace_bytes'] - original['workspace_bytes'], 4 * GIB)
        self.assertEqual(large['required_free_bytes'] - original['required_free_bytes'], 4 * GIB)
        for key in ('registry_bytes', 'max_cpu', 'max_memory_mib', 'max_disk_mib', 'max_build_seconds'):
            self.assertEqual(large[key], original[key])

    def test_invalid_import_limits_are_not_clamped_to_protocol_bounds(self):
        for field, values in (('input_bytes', (True, 511, 16 * GIB + 1, '17179869184')),
                              ('expanded_bytes', (True, 10239, 32 * GIB + 1, float('inf')))):
            for value in values:
                with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                    plan(self.tasks(1), 1, 64 * GIB, 44649272, **{field: value})


if __name__=='__main__':unittest.main()
