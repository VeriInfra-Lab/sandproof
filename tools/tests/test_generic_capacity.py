import unittest
from tools.generic_capacity import GIB, plan


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

    def test_invalid_shapes_are_rejected_without_silent_clamping(self):
        for field,value in [('cpus',True),('memory_mib',32768),('disk_mib',0),
                            ('build_timeout_sec',3601),('build_timeout_sec',True),
                            ('build_timeout_sec',float('nan')),('build_timeout_sec','600')]:
            tasks=self.tasks(1);tasks[0]['requirements'][field]=value
            with self.subTest(field=field),self.assertRaises(ValueError):plan(tasks,1,32*GIB,44649272)
        with self.assertRaises(ValueError):plan(self.tasks(1)*2,1,32*GIB,44649272)


if __name__=='__main__':unittest.main()
