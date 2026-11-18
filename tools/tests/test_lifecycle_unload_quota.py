"""Quota parser, approval-file boundary, and per-run receipt regressions."""
import importlib.util
import copy
import hashlib
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

PATH=Path(__file__).resolve().parents[2]/'tests/system/lifecycle_unload_quota.py'
sys.path.insert(0,str(PATH.parent))
SPEC=importlib.util.spec_from_file_location('lifecycle_unload_quota_readback',PATH)
quota=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(quota)


class QuotaReadbackTests(unittest.TestCase):
    def test_exact_numeric_project_hard_limit_without_human_units(self):
        output='Project ID Used Soft Hard Warn/Grace\n#0 9 0 0 00 [------]\n#63801 12 0 16777216 00 [------]\n'
        self.assertEqual(quota.project_hard_limit(output,63801),16777216)
        self.assertEqual(quota.project_hard_limit('63801 3 0 65536 00 [------]\n',63801),65536)

    def test_missing_duplicate_zero_and_partial_limits_cannot_publish_receipt(self):
        for text in ('','#63802 0 0 16777216 00\n','#63801 0 0 0 00\n',
                '#63801 0 0\n','#63801 0 0 16G 00\n',
                '#63801 0 0 16 00\n#63801 0 0 16 00\n',
                '#63801 0 4 16777216 00\n'):
            with self.subTest(output=text),self.assertRaises(ValueError):
                quota.project_hard_limit(text,63801)

    def test_oversize_approval_rejected_before_read_or_quota_tool_lookup(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'deployment.json'
            raw=b' '*65537
            path.write_bytes(raw)
            path.chmod(0o600)
            args=SimpleNamespace(deployment_receipt=path,
                deployment_receipt_sha256=hashlib.sha256(raw).hexdigest(),run_identity='a'*64)
            metadata=SimpleNamespace(st_mode=path.stat().st_mode,st_uid=0,st_nlink=1,st_size=len(raw))
            with patch.object(quota.os,'geteuid',return_value=0), \
                    patch.object(quota.os,'umask'), \
                    patch.object(quota,'private_path',return_value=path), \
                    patch.object(quota.os,'stat',return_value=metadata), \
                    patch.object(quota.os,'open') as opened, \
                    patch.object(Path,'read_bytes',return_value=raw) as read_bytes, \
                    patch.object(quota,'trusted_path') as trusted:
                with self.assertRaises(ValueError):
                    quota.readback(args)
                opened.assert_not_called()
                read_bytes.assert_not_called()
                trusted.assert_not_called()


class StaticDeploymentRunReceiptTests(unittest.TestCase):
    def setUp(self):
        self.project=dict(project_id=63802,hard_bytes=1024**3,hard_inodes=1024)
        self.static=dict(schema_version=1,phase='quota-provisioned',mount='/approved/material',
            filesystem_device=7,filesystem_uuid='original-uuid',source='/dev/original',
            projects=[self.project],xfs_quota_sha256='a'*64,
            native_lock_identity=dict(device=1,inode=9,owner_uid=1000),
            deployment_control='/approved/static-control',
            provisioning_host_lease_identity='previous-provisioning-operator',
            provisioning_ci_state_root='/previous/ci',provision_plan_sha256='b'*64,
            initial_raw_output_sha256={key:'c'*64 for key in ('state','blocks','inodes')})
        self.captures=[dict(material_project=self.project)]
        self.receipt={key:copy.deepcopy(self.static[key]) for key in
            ('filesystem_device','filesystem_uuid','mount','source','projects',
             'native_lock_identity','xfs_quota_sha256')}
        self.receipt.update(schema_version=1,accounting=True,enforcement=True,
            actual_readback_verified=True,deployment_receipt_sha256='d'*64,
            host_lease_identity='e'*64,ci_state_root='/current/ci',
            deployment_control='/approved/material/current-run/quota',
            raw_output_sha256={key:'f'*64 for key in ('state','blocks','inodes')})

    def test_static_history_and_new_run_are_independent(self):
        before=copy.deepcopy(self.static)
        self.assertIs(quota.validate_run_receipt(self.receipt,self.static,self.captures,'d'*64),self.receipt)
        self.assertEqual(self.static,before)
        self.assertNotEqual(self.static['provisioning_host_lease_identity'],self.receipt['host_lease_identity'])
        self.assertEqual(quota.project_hard_limit('#63802 0 0 1024 00\n',63802),1024)

    def test_static_receipt_cannot_depend_on_new_run_or_new_receipt(self):
        for key in ('run_identity','host_lease_identity','ci_state_root','quota_receipt_sha256','capture_settings_sha256'):
            value=copy.deepcopy(self.static);value[key]='e'*64
            with self.subTest(key=key),self.assertRaises(ValueError):
                quota.validate_deployment(value)

    def test_changed_filesystem_lock_tool_project_or_readback_is_rejected(self):
        changes=[('filesystem_device',8),('filesystem_uuid','other'),('source','/dev/other'),
            ('native_lock_identity',dict(device=1,inode=10,owner_uid=1000)),
            ('xfs_quota_sha256','0'*64),('projects',[]),('deployment_receipt_sha256','0'*64),
            ('accounting',False),('enforcement',False),('actual_readback_verified',False),
            ('host_lease_identity','historical-lease'),('raw_output_sha256',{}),
            ('deployment_control','relative/evidence')]
        for key,value in changes:
            changed=copy.deepcopy(self.receipt);changed[key]=value
            with self.subTest(key=key),self.assertRaises(ValueError):
                quota.validate_run_receipt(changed,self.static,self.captures,'d'*64)

    def test_equal_numeric_aliases_do_not_match_the_approved_receipt(self):
        paths=[
            (('schema_version',),True),
            (('schema_version',),1.0),
            (('filesystem_device',),7.0),
            (('native_lock_identity','device'),True),
            (('native_lock_identity','device'),1.0),
            (('native_lock_identity','inode'),9.0),
            (('native_lock_identity','owner_uid'),1000.0),
        ]
        paths.extend((('projects',0,key),float(value)) for key,value in self.project.items())
        for path,replacement in paths:
            changed=copy.deepcopy(self.receipt)
            field=changed
            for key in path[:-1]:
                field=field[key]
            self.assertEqual(field[path[-1]],replacement)
            field[path[-1]]=replacement
            with self.subTest(path=path,replacement=replacement),self.assertRaises(ValueError):
                quota.validate_run_receipt(changed,self.static,self.captures,'d'*64)

        for key,value in self.project.items():
            changed_receipt=copy.deepcopy(self.receipt)
            changed_captures=copy.deepcopy(self.captures)
            changed_receipt['projects'][0][key]=float(value)
            changed_captures[0]['material_project'][key]=float(value)
            with self.subTest(capture_project_key=key),self.assertRaises(ValueError):
                quota.validate_run_receipt(changed_receipt,self.static,changed_captures,'d'*64)

    def test_duplicate_partial_and_unapproved_project_is_rejected(self):
        for project in (dict(self.project,project_id=True),dict(self.project,hard_bytes=1025),
                        dict(self.project,hard_inodes=0)):
            changed=copy.deepcopy(self.static);changed['projects']=[project]
            with self.assertRaises(ValueError):quota.validate_deployment(changed)
        changed=copy.deepcopy(self.static);changed['projects']*=2
        with self.assertRaises(ValueError):quota.validate_deployment(changed)
        changed=copy.deepcopy(self.captures);changed[0]['material_project']['hard_bytes']*=2
        with self.assertRaises(ValueError):quota.validate_run_receipt(self.receipt,self.static,changed,'d'*64)


if __name__=='__main__':
    unittest.main()
