"""Quota parser, approval-file boundary, and per-run receipt regressions."""
import importlib.util
import copy
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from tools import capture_acceptance, xfs_project_quota

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

    def test_capture_preserves_argv_and_executes_the_selected_descriptor(self):
        command=['/usr/sbin/xfs_quota','-x','-c','state -p','/synthetic/mount']
        for descriptor in (None,7):
            execution = {} if descriptor is None else {
                'executable':'/proc/self/fd/7','pass_fds':(7,),
            }
            with self.subTest(descriptor=descriptor),patch.object(
                    xfs_project_quota.subprocess,'Popen',side_effect=OSError('controlled spawn')) as spawn:
                with self.assertRaisesRegex(OSError,'controlled spawn'):
                    options = {} if descriptor is None else {'executable_fd':descriptor}
                    xfs_project_quota.capture(command[0],command[3],command[4],**options)
                spawn.assert_called_once_with(command,stdin=xfs_project_quota.subprocess.DEVNULL,
                    stdout=xfs_project_quota.subprocess.PIPE,stderr=xfs_project_quota.subprocess.PIPE,
                    env=xfs_project_quota.ENV,**execution)


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

    def test_readback_bounds_tool_before_read_and_holds_the_approved_inode(self):
        for case in ('valid','oversized','wrong-digest','replaced'):
            with self.subTest(case=case),tempfile.TemporaryDirectory() as directory,ExitStack() as mocks:
                base=Path(directory)
                root=base/'mount'
                root.mkdir()
                root.chmod(0o711)
                output=root/'run'
                output.mkdir()
                tool=base/'xfs_quota'
                original=b'approved executable fixture'
                tool.write_bytes(b'different bytes' if case=='wrong-digest' else original)
                tool.chmod(0o755)
                if case=='oversized':
                    with tool.open('r+b') as stream:
                        stream.truncate(16*1024**2+1)
                lock=root/'native.lock'
                lock.touch(mode=0o600)
                lock_stat=lock.stat()
                tool_stat=tool.stat()
                static=dict(self.static,mount=str(root),filesystem_device=root.stat().st_dev,
                    xfs_quota_sha256=hashlib.sha256(original).hexdigest(),
                    native_lock_identity=dict(device=lock_stat.st_dev,inode=lock_stat.st_ino,owner_uid=lock_stat.st_uid))
                args=SimpleNamespace(deployment_receipt=base/'approved.json',deployment_receipt_sha256='d'*64,
                    run_identity='a'*64,ci_state_root=base/'ci',capture_settings=base/'capture.json',
                    output=output,native_lock=lock)
                mounted=dict(target=str(root),fstype='xfs',source=static['source'],
                    uuid=static['filesystem_uuid'],options='rw,prjquota')
                responses={
                    'state -p':f'Project quota state on {root}\nAccounting: ON\nEnforcement: ON\n',
                    'report -p -b -N -n':f"#63802 0 0 {self.project['hard_bytes']//1024} 00\n",
                    'report -p -i -N -n':f"#63802 0 0 {self.project['hard_inodes']} 00\n",
                }

                path_stat=Path.stat
                descriptor_stat=os.fstat
                def root_owned(info):
                    fields=('st_dev','st_ino','st_size','st_mtime_ns','st_ctime_ns','st_mode','st_gid','st_nlink')
                    return SimpleNamespace(st_uid=0,**{key:getattr(info,key) for key in fields})

                def observe_path(path,**kwargs):
                    info=path_stat(path,**kwargs)
                    return root_owned(info) if path in (root,tool) else info

                def observe_descriptor(descriptor):
                    info=descriptor_stat(descriptor)
                    if (info.st_dev,info.st_ino)==(tool_stat.st_dev,tool_stat.st_ino):
                        return root_owned(info)
                    return info

                path_reads=[]
                read_bytes=Path.read_bytes
                def read_path(path):
                    if path==tool:
                        path_reads.append(path)
                        if case=='oversized':
                            # Observe the obsolete read without filling memory with the oversized file.
                            return original
                    return read_bytes(path)

                observed=[]
                def report(path,command,mount,*,executable_fd=None):
                    self.assertEqual((path,mount),(tool,root))
                    inode=None if executable_fd is None else descriptor_stat(executable_fd).st_ino
                    observed.append((command,executable_fd,inode))
                    if case=='replaced' and command=='state -p':
                        replacement=base/'same-bytes'
                        replacement.write_bytes(original)
                        replacement.chmod(0o755)
                        replacement.replace(tool)
                    return responses[command]

                mocks.enter_context(patch.object(os,'geteuid',return_value=0))
                mocks.enter_context(patch.object(os,'umask'))
                mocks.enter_context(patch.object(os,'fstat',side_effect=observe_descriptor))
                mocks.enter_context(patch.object(Path,'stat',observe_path))
                mocks.enter_context(patch.object(Path,'read_bytes',read_path))
                mocks.enter_context(patch.object(quota,'private_path',side_effect=lambda path,**_:path))
                mocks.enter_context(patch.object(quota,'original_json',return_value=static))
                mocks.enter_context(patch.object(quota,'trusted_path',side_effect=[root,tool]))
                mocks.enter_context(patch.object(quota,'json_file',side_effect=[
                    {'e2e':args.run_identity},{'e2e':args.run_identity},self.captures]))
                mocks.enter_context(patch.object(capture_acceptance,'validate_settings',return_value=self.captures))
                findmnt=mocks.enter_context(patch.object(quota,'bounded_json_command',return_value={'filesystems':[mounted]}))
                mocks.enter_context(patch.object(quota,'capture',side_effect=report))
                receipt=output/'quota-receipt.json'
                if case=='valid':
                    result=quota.readback(args)
                    self.assertEqual(json.loads(receipt.read_text()),result)
                    self.assertEqual(result['xfs_quota_sha256'],static['xfs_quota_sha256'])
                else:
                    with self.assertRaises(ValueError):
                        quota.readback(args)
                    self.assertEqual(list(output.iterdir()),[])

                self.assertEqual(path_reads,[])
                if case in ('valid','replaced'):
                    findmnt.assert_called_once()
                    self.assertEqual([command for command,_,_ in observed],list(responses))
                    descriptor=observed[0][1]
                    self.assertIsNotNone(descriptor)
                    self.assertEqual([(fd,inode) for _,fd,inode in observed],[(descriptor,tool_stat.st_ino)]*3)
                    with self.assertRaises(OSError):
                        descriptor_stat(descriptor)
                else:
                    findmnt.assert_not_called()
                    self.assertEqual(observed,[])

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
