"""Shared assembly contracts only: no product process, TLS, Redis or KVM."""
import argparse
import copy
import os
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tests/system'))
import _runtime_services as services
from _complete_runtime import complete_runtime
from tools.benchmark_ci import remove_private_tree


class RuntimeServicesTests(unittest.TestCase):
    def node(self):
        identity=dict(deployment_id='exact-domain',node_id='host-runtime',node_generation=1)
        bounds=dict(instances=2,cpu=2,memory_bytes=4000,disk_bytes=8000)
        return dict(identity=identity,role='runtime',mode='accepting',protocol_version=2,retention_slots=4,
                    healthy_until_ms=1100,free=copy.deepcopy(bounds),limit=bounds,
                    templates=[dict(revision='base-r1',digest=[1]*32)])

    def test_readiness_is_original_identity_cache_health_and_explicit_capacity_purpose(self):
        node=self.node()
        registered={node['identity']['node_id']:dict(registration=copy.deepcopy(node))}
        templates={'host-runtime':{'base-r1'}}
        self.assertTrue(services.nodes_ready(dict(nodes=[node]),registered,templates,now_ms=1000))
        node['templates'].append(dict(revision='later-ready',digest=[2]*32))
        self.assertTrue(services.nodes_ready(dict(nodes=[node]),registered,templates,now_ms=1000))
        node['free']['instances']=0
        self.assertFalse(services.nodes_ready(dict(nodes=[node]),registered,templates,now_ms=1000))
        self.assertTrue(services.nodes_ready(dict(nodes=[node]),registered,templates,now_ms=1000,for_admission=False))
        for key,value in (('instances',True),('cpu',-1),('disk_bytes',8001)):
            malformed=copy.deepcopy(node)
            malformed['free'][key]=value
            with self.subTest(key=key):
                self.assertFalse(services.nodes_ready(dict(nodes=[malformed]),registered,templates,
                    now_ms=1000,for_admission=False))
        for field,value in (('healthy_until_ms',999),('protocol_version',1),('retention_slots',3),
                            ('retention_slots',True),('retention_slots',None),('role','builder'),
                            ('templates',[]),('mode','draining')):
            malformed=copy.deepcopy(node)
            malformed[field]=value
            self.assertFalse(services.nodes_ready(dict(nodes=[malformed]),registered,templates,
                now_ms=1000,for_admission=False),field)
        self.assertFalse(services.nodes_ready(dict(nodes=[node,node]),registered,templates,now_ms=1000))

    def test_common_cli_has_two_products_and_no_old_server_option(self):
        parser=argparse.ArgumentParser()
        services.add_arguments(parser)
        options={option for action in parser._actions for option in action.option_strings}
        self.assertIn('--center',options)
        self.assertIn('--host',options)
        self.assertIn('--candidate-manifest',options)
        self.assertNotIn('--server',options)

    def test_native_mapping_refuses_live_host_before_any_audit_or_global_suffix_parse(self):
        instance=services.RuntimeServices()
        instance.host_configs=[dict(inbox=dict(node={'node_id':'exact-node'}))]
        instance.native_pools=[dict(namespace='a'*16)]
        instance.processes=Mock(children={'host-0':Mock()})
        instance.processes.children['host-0'].poll.return_value=None
        with self.assertRaisesRegex(RuntimeError,'stop exact host'):
            instance.native_id_for('global-999')
        instance.processes.audit.assert_not_called()
        instance.processes.children['host-0'].poll.return_value=0
        with patch.object(services,'local_binding',return_value=dict(sandbox=17)) as mapped:
            self.assertEqual(instance.native_id_for('global-999'),'sb-'+'a'*16+'-17')
        self.assertEqual(mapped.call_args.args[1],'global-999')

    def test_static_cache_declaration_is_complete_and_original_supplied_quota_not_raised(self):
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve(),prefix='rs-') as temporary:
            root=Path(temporary).resolve()
            native=complete_runtime(dict(namespace='a'*16,database=str(root/'db/controller.db'),
                node_state_dir=str(root/'state'),chroot_base=str(root/'jails'),kernel=str(root/'kernel'),
                rootfs=str(root/'rootfs'),firecracker=str(root/'firecracker'),uid_base=3200000,
                max_active_global=1,max_active_per_tenant=1,max_disk_bytes_global=8192,
                max_disk_bytes_per_tenant=4096,cleanup_reserve_bytes=4096),slots=1,file_size=lambda _:1024)
            original=copy.deepcopy(native['bounded_host']['physical_storage']['pools'])
            layout=dict(hosts=[root],center_identity=(1001,1001),registry=root/'registry')
            catalog=dict(templates=[dict(kernel=dict(size_bytes=1024),rootfs=dict(size_bytes=1024))])
            registry=dict(max_artifact_bytes_global=2048,max_templates_global=1)
            with patch.object(services.os,'statvfs',return_value=SimpleNamespace(
                    f_bavail=10**12,f_frsize=4096,f_favail=10**9)):
                profile=services._bind_profile([native],[catalog],layout,registry=registry,service_peaks=[])
            self.assertEqual(profile['pools'],original)
            cache=profile['host_template_caches'][0]
            self.assertEqual((cache['max_entry_bytes'],cache['artifact_bytes'],cache['temporary_bytes']),
                             (2048,2048,2048))
            self.assertEqual(cache['metadata_bytes'],4096+2*(65536+4096))
            native['bounded_host']['physical_storage']['service_data']=[]
            smaller=copy.deepcopy(cache)
            smaller.update(max_entry_bytes=1024,artifact_bytes=1024,temporary_bytes=1024)
            native['bounded_host']['physical_storage']['host_template_caches']=[smaller]
            with self.assertRaisesRegex(ValueError,'no capacity is raised'):
                services._bind_profile([native],[catalog],layout,registry=registry,service_peaks=[])
            self.assertEqual(native['bounded_host']['physical_storage']['host_template_caches'][0]['artifact_bytes'],1024)

    def test_serial_static_batches_fit_without_reducing_original_runtime_or_cleanup_quota(self):
        gib = 1024**3
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve(),prefix='rs-') as temporary:
            root=Path(temporary).resolve()
            native=complete_runtime(dict(namespace='a'*16,database=str(root/'db/controller.db'),
                node_state_dir=str(root/'state'),chroot_base=str(root/'jails'),kernel=str(root/'kernel'),
                rootfs=str(root/'rootfs'),firecracker=str(root/'firecracker'),uid_base=3200000,
                max_active_global=2,max_active_per_tenant=2,max_disk_bytes_global=48*gib,
                max_disk_bytes_per_tenant=40*gib,cleanup_reserve_bytes=8*gib),slots=2,file_size=lambda _:1024)
            physical=native['bounded_host']['physical_storage']
            physical['filesystem_overhead_bytes']=2*gib
            physical['pools'][0].update(rootfs_budget_bytes=40*gib,cleanup_reserve_bytes=8*gib)
            original=copy.deepcopy(physical['pools'])
            layout=dict(hosts=[root],center_identity=(1001,1001),registry=root/'registry')
            image=dict(kernel=dict(size_bytes=44_649_272),rootfs=dict(size_bytes=10*gib))
            def bind(count,ceiling=services.MAX_ACTIVATION_BYTES):
                return services._bind_profile([native],[dict(templates=[image]*count)],layout,
                    registry=dict(max_artifact_bytes_global=count*(10*gib+44_649_272),max_templates_global=count),
                    service_peaks=[dict(root=str(root/'assets'),owner_uid=1001,
                                        peak_bytes=count*(10*gib+44_649_272)+gib,inode_budget=32)],
                    activation_ceiling_bytes=ceiling)
            with patch.object(services.os,'statvfs',return_value=SimpleNamespace(
                    f_bavail=10**12,f_frsize=4096,f_favail=10**9)):
                with self.assertRaisesRegex(ValueError,'exceeds prepared CI ceiling; no quota is reduced'):
                    bind(2)
                self.assertEqual(native['bounded_host']['physical_storage']['pools'],original)
                self.assertEqual(native['bounded_host']['physical_storage']['host_template_caches'],[])
                profile=bind(1)
                native['bounded_host']['physical_storage']['service_data']=[]
                native['bounded_host']['physical_storage']['host_template_caches']=[]
                for invalid in (0,True,services.MAX_ACTIVATION_BYTES+1):
                    with self.subTest(ceiling=invalid),self.assertRaisesRegex(ValueError,'explicit prepared CI activation ceiling'):
                        bind(2,invalid)
                generic_profile=bind(2,None)
            self.assertLessEqual(2*services.envelope(profile)[0],services.MAX_ACTIVATION_BYTES)
            self.assertEqual(profile['pools'],original)
            # General template/build probes keep their own original approved
            # envelope and actual free-space check, not the two-task CI ceiling.
            self.assertGreater(2*services.envelope(generic_profile)[0],services.MAX_ACTIVATION_BYTES)
            self.assertEqual(generic_profile['pools'],original)

    def test_role_cleanup_requires_original_exit_and_parent_identity(self):
        instance=services.RuntimeServices()
        instance.processes=Mock(bindings={'center':{},'redis':{},'host-0':{}},
                                children={'center':Mock()})
        with patch.object(services,'remove_private_tree') as removed,self.assertRaises(RuntimeError):
            instance.cleanup_staging()
        removed.assert_not_called()
        with tempfile.TemporaryDirectory() as temporary:
            common=Path(temporary)/'common'
            common.mkdir(mode=0o711)
            common.chmod(0o711)
            for name in ('center','redis'):
                (common/name).mkdir(mode=0o700)
            instance.layout=dict(common=common,center=common/'center',redis=common/'redis')
            instance.staging_identity={name:(path.stat().st_dev,path.stat().st_ino)
                                      for name,path in instance.layout.items()}
            instance.staging_owners=dict(center=os.geteuid(),redis=os.geteuid())
            instance.processes.children={name:Mock() for name in instance.processes.bindings}
            for child in instance.processes.children.values():
                child.poll.return_value=0
            # Actual root fixture uses owner0; component test keeps our uid and
            # exercises the identity refusal before any privileged cleanup.
            instance.staging_identity['common']=(0,1)
            with patch.object(services,'remove_private_tree') as removed,self.assertRaises(ValueError):
                instance.cleanup_staging()
            removed.assert_not_called()

    def test_original_cleanup_supports_one_explicit_role_owner_without_general_path_relaxation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve()/'role'
            root.mkdir(mode=0o700)
            (root/'test-only-key').write_text('component-only-secret')
            identity=[root.stat().st_dev,root.stat().st_ino]
            with self.assertRaises(ValueError):
                remove_private_tree(root,identity,owner_uid=True)
            self.assertTrue(root.exists())
            remove_private_tree(root,identity,owner_uid=os.geteuid())
            self.assertFalse(root.exists())


if __name__=='__main__':
    unittest.main()
