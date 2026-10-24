"""Shared assembly contracts only: no product process, TLS, Redis or KVM."""
import argparse
import copy
import json
from contextlib import ExitStack
import os
from pathlib import Path
import shutil
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


class CaptureProvisionTests(unittest.TestCase):
    def fixture(self, root):
        control=root/'control';control.mkdir(mode=0o700)
        parent=root/'p';parent.mkdir(mode=0o700)
        return control,dict(inbox=dict(host_epoch=None),capture=dict(layout_parent=str(parent)))

    def write_role(self,path,value,*,uid,gid):
        self.assertEqual((uid,gid),(0,0))
        path.write_text(json.dumps(value));path.chmod(0o600)

    def test_original_capture_command_precedes_bootstrap_and_final_role_config(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);control,config=self.fixture(root)
            calls=[]
            def capture(command,**kwargs):
                self.assertEqual(command,['original-cgroup',str(control/'sandproof-host'),
                    'provision-capture-layout','--config',str(control/'host-provision.json')])
                self.assertEqual(kwargs['timeout'],30)
                self.assertTrue(kwargs['check'])
                self.assertFalse((control/'host.json').exists())
                calls.append('capture')
                for name in ('v','s'):
                    (root/'p'/name).mkdir(mode=0o700)
            registration=dict(version=1,boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),policy=[1]*32)
            def bootstrap(command,*args,**kwargs):
                self.assertEqual(command,['original-cgroup',str(control/'sandproof-host'),
                    'provision-bootstrap','--config',str(control/'host-provision.json')])
                self.assertEqual(calls,['capture'])
                calls.append('bootstrap')
                return registration
            with patch.object(services,'write_role',side_effect=self.write_role), \
                    patch('tools.service_cgroup.prefix',return_value=['original-cgroup']), \
                    patch.object(services.subprocess,'run',side_effect=capture), \
                    patch('tools.ci_e2e_prepare.bounded_json_command',side_effect=bootstrap):
                path=services._provision_host(control/'sandproof-host',control,config,root/'group.json')
            self.assertEqual(calls,['capture','bootstrap'])
            self.assertEqual(json.loads(path.read_text())['inbox']['host_epoch'],registration)
            self.assertIsNone(json.loads((control/'host-provision.json').read_text())['inbox']['host_epoch'])

    def test_partial_capture_error_is_preserved_and_never_starts_bootstrap_or_retries(self):
        with tempfile.TemporaryDirectory() as directory:
            root=Path(directory);control,config=self.fixture(root)
            failure=services.subprocess.CalledProcessError(1,['capture'])
            def partial(*args,**kwargs):
                (root/'p/v').mkdir(mode=0o700)
                raise failure
            with patch.object(services,'write_role',side_effect=self.write_role), \
                    patch('tools.service_cgroup.prefix',return_value=['original-cgroup']), \
                    patch.object(services.subprocess,'run',side_effect=partial) as command, \
                    patch('tools.ci_e2e_prepare.bounded_json_command') as bootstrap, \
                    self.assertRaises(services.subprocess.CalledProcessError) as caught:
                services._provision_host(control/'sandproof-host',control,config,root/'group.json')
            self.assertIs(caught.exception,failure)
            command.assert_called_once()
            bootstrap.assert_not_called()
            self.assertTrue((root/'p/v').is_dir())
            self.assertTrue((control/'host-provision.json').is_file())
            self.assertFalse((control/'host.json').exists())


class ReadinessSimulation:
    """Fake time and canceled worker progress, not a product health authority."""
    def __init__(self,node,*,commit_after=0,missing_observations=0,never_ready=False,host_exit_at=None,
                 health_fresh_after=0):
        self.now=0.0
        self.center_started_at=0.0
        self.center_running=True
        self.committed=False
        self.commit_after=commit_after
        self.missing_observations=missing_observations
        self.never_ready=never_ready
        self.host_exit_at=host_exit_at
        self.health_fresh_after=health_fresh_after
        self.node=copy.deepcopy(node)
        self.stops=[]
        self.starts=[]
        self.audits=[]
        self.observations=[]
        self.canceled=0
        self.requests=[]
        self.instance=services.RuntimeServices()
        self.instance.center_config=dict(reply_timeout_ms=5000,expiry_scan_ms=1000,poll_interval_ms=100,
            node_observation=dict(health_window_ms=10000),
            nodes=[dict(registration=copy.deepcopy(node))])
        self.instance.expected_templates={node['identity']['node_id']:{'base-r1'}}
        self.instance.host_configs=[dict(artifact_source=dict(download_timeout_ms=900000))]
        self.instance.sdk_url='http://fixed-component.invalid'
        self.instance.key='component-only-key'
        self.processes=Mock(children={})
        for name in ('center','host-0','redis'):
            self.processes.children[name]=Mock()
            self.processes.children[name].poll.side_effect=lambda name=name:self.poll(name)
        self.processes.stop.side_effect=self.stop
        self.processes.start.side_effect=self.start
        self.processes.audit.side_effect=AssertionError('readiness must never offline-audit')
        self.processes.center_readiness.side_effect=self.readiness
        self.instance.processes=self.processes

    def poll(self,name):
        if name=='center' and not self.center_running:
            return 0
        if name=='host-0' and self.host_exit_at is not None and self.now>=self.host_exit_at:
            return 93
        return None

    def sleep(self,seconds):
        self.now+=seconds

    def stop(self,name):
        if name!='center':
            raise ValueError('readiness must not stop another role')
        if self.now-self.center_started_at>=self.commit_after:
            self.committed=True
        elif not self.committed:
            self.canceled+=1
        self.center_running=False
        self.stops.append(self.now)

    def start(self,name):
        if name!='center':
            raise ValueError('readiness must not restart another role')
        self.center_running=True
        self.center_started_at=self.now
        self.starts.append(self.now)

    def readiness(self,*,timeout):
        if not self.center_running:
            raise ValueError('online original center required')
        self.observations.append(self.now)
        if len(self.observations)<=self.missing_observations:
            raise ValueError('authenticated response lost')
        if self.now-self.center_started_at>=self.commit_after:
            self.committed=True
        node=copy.deepcopy(self.node)
        observed=self.now if self.now>=self.health_fresh_after else 0
        node['healthy_until_ms']=int((1000+observed)*1000)+10000
        if not self.committed or self.never_ready:
            node['templates']=[]
        return dict(nodes=[node],catalogs={node['identity']['node_id']:[dict(revision='base-r1',digest=[1]*32)]})

    def fetch(self,_opener,_url,method,path,_key,*args,**kwargs):
        self.requests.append((method,path,args,kwargs))
        return 200,b'[]'

    def run(self,*,audit=None,**kwargs):
        with ExitStack() as stack:
            stack.enter_context(patch.object(services.time,'monotonic',side_effect=lambda:self.now))
            stack.enter_context(patch.object(services.time,'time',side_effect=lambda:1000+self.now))
            stack.enter_context(patch.object(services.time,'sleep',side_effect=self.sleep))
            stack.enter_context(patch.object(services,'fetch',side_effect=self.fetch))
            return (self.instance.wait_ready(**kwargs) if audit is None else
                    self.instance.planned_audit(audit,**kwargs))


class RuntimeServicesTests(unittest.TestCase):
    def test_one_storage_sampler_precedes_redis_and_spans_all_role_restarts(self):
        instance=services.RuntimeServices()
        events=[]
        instance.service_storage=Mock()
        instance.service_storage.start.side_effect=lambda:events.append('sample-start')
        instance.native_pools=[{}]
        instance.center_config=dict(redis={})
        instance.processes=Mock(children={'redis':Mock()})
        instance.processes.start.side_effect=lambda role,**_:events.append('start-'+role)
        instance.processes.initialize.side_effect=lambda role:events.append('initialize-'+role)
        with patch.object(services,'_wait_redis'):
            instance.start()
        self.assertEqual(events,['sample-start','start-redis','initialize-host-0',
            'start-host-0','initialize-center','start-center'])
        instance.service_storage.start.assert_called_once()

    def test_storage_sampler_is_drained_even_when_an_original_role_exit_is_unknown(self):
        instance=services.RuntimeServices()
        instance.service_storage=Mock()
        instance.processes=Mock()
        instance.processes.stop_all.side_effect=RuntimeError('original role exit unknown')
        with self.assertRaisesRegex(RuntimeError,'original role exit unknown'):
            instance.stop_all()
        instance.service_storage.stop.assert_called_once()
        instance.service_storage.report.return_value={'last-stable':'original'}
        self.assertEqual(instance.storage_peaks(),{'last-stable':'original'})

    def test_sealed_cargo_source_hardlink_copies_to_independent_target_with_original_hash(self):
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve(),prefix='rs-') as temporary:
            root=Path(temporary).resolve()
            source,target=root/'sandproof-center',root/'staged-center'
            source.write_bytes(b'protected-cargo-output')
            source.chmod(0o700)
            os.link(source,root/'cargo-deps-name')
            self.assertEqual(source.lstat().st_nlink,2)
            original_lstat=Path.lstat
            def metadata(path):
                value=original_lstat(path)
                if path==source:
                    # Only the root-owner deployment boundary is substituted;
                    # hardlink/inode/bytes/hash and copy checks remain real.
                    return SimpleNamespace(**{key:getattr(value,key) for key in (
                        'st_dev','st_ino','st_nlink','st_size','st_mtime_ns','st_ctime_ns','st_mode')},st_uid=0)
                return value
            with patch.object(services,'trusted_path',side_effect=lambda path:path), \
                    patch.object(Path,'lstat',metadata),patch.object(services.os,'chown'), \
                    patch.object(services.subprocess,'run',side_effect=lambda *_args,**_kwargs:shutil.copyfile(source,target)):
                services._copy(source,target,identity=(1001,1001),executable=True,expected=services.digest(source))
                with self.assertRaises(ValueError):
                    services._copy(source,target,identity=(1001,1001))
            self.assertEqual(target.lstat().st_nlink,1)
            self.assertNotEqual(source.lstat().st_ino,target.lstat().st_ino)
            self.assertEqual(services.digest(target),services.digest(source))

    def test_real_prepare_validates_original_string_path_manifest_before_role_staging(self):
        """Keep the original artifact validator; stop before any role allocation."""
        with tempfile.TemporaryDirectory(dir=Path('/tmp').resolve(),prefix='rs-') as temporary:
            root=Path(temporary).resolve()
            for part in ('db','state','p'):
                (root/part).mkdir()
            paths={role:str(root/('sandproof-'+role)) for role in ('center','host')}
            versions=dict(internal_protocol=2,center_store=6,host_store=16)
            candidate='a'*40
            manifest=dict(schema_version=1,architecture=services.artifacts.ARCHITECTURE,
                candidate=candidate,versions=versions,
                artifacts={role:dict(path=path,sha256='b'*64) for role,path in paths.items()})
            args=SimpleNamespace(**paths,center_uid=1001,center_gid=1001,redis_uid=1002,
                redis_gid=1002,candidate_manifest=root/'manifest.json',redis_material=root/'redis',
                service_staging_parent=root/'staging',capture_settings=root/'capture.json')
            disk=root/'disk';disk.write_bytes(bytes(512))
            native=dict(uid_base=3200000,database=str(root/'db/controller.db'),
                node_state_dir=str(root/'state'),chroot_base=str(root/'p/v'),
                max_active_global=1,memory_mib=1,rootfs=str(disk))
            tool=dict(path='/approved/project-tool',sha256=[1]*32)
            captures=[dict(layout_parent=str(root/'p'),layout_binding=[1]*32,
                state_copy_bytes=32,memory_copy_bytes=1024**2,disk_copy_bytes=512,
                api_timeout_ms=1000,worker_timeout_ms=2000,
                material_project=dict(project_id=63801,hard_bytes=128*1024**2,hard_inodes=1024,
                    chattr=tool,lsattr=tool))]
            class BeforeStaging(RuntimeError):
                pass
            with patch.object(services.os,'geteuid',return_value=0), \
                    patch.object(services,'private_path',side_effect=lambda path,**_:Path(path)), \
                    patch.object(services,'json_file',side_effect=lambda path,**_: manifest if path==args.candidate_manifest else captures), \
                    patch.object(services.artifacts,'json_file',return_value=manifest), \
                    patch.object(services.artifacts,'private_path',side_effect=lambda path,**_:Path(path)), \
                    patch.object(services.artifacts,'_fingerprint',return_value='b'*64), \
                    patch.object(services.subprocess,'check_output',return_value=candidate), \
                    patch.object(services,'contract_versions',return_value=versions), \
                    patch.object(services,'check_material',return_value={}), \
                    patch.object(services,'stage_role_directories',side_effect=BeforeStaging) as stage, \
                    self.assertRaises(BeforeStaging):
                services.RuntimeServices.prepare(args,root,[native],[{}],'test-key',
                    '127.0.0.1:12345',root/'record.json',2,registry_config={})
            stage.assert_called_once()

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
        view=dict(nodes=[node],catalogs={'host-runtime':[dict(revision='base-r1',digest=[1]*32)]})
        self.assertTrue(services.nodes_ready(view,registered,templates,now_ms=1000))
        node['templates'].append(dict(revision='later-ready',digest=[2]*32))
        self.assertTrue(services.nodes_ready(view,registered,templates,now_ms=1000))
        node['free']['instances']=0
        self.assertFalse(services.nodes_ready(view,registered,templates,now_ms=1000))
        self.assertTrue(services.nodes_ready(view,registered,templates,now_ms=1000,for_admission=False))
        for key,value in (('instances',True),('cpu',-1),('disk_bytes',8001)):
            malformed=copy.deepcopy(node)
            malformed['free'][key]=value
            with self.subTest(key=key):
                self.assertFalse(services.nodes_ready(dict(view,nodes=[malformed]),registered,templates,
                    now_ms=1000,for_admission=False))
        for field,value in (('healthy_until_ms',999),('protocol_version',1),('retention_slots',3),
                            ('retention_slots',True),('retention_slots',None),('role','builder'),
                            ('templates',[]),('mode','draining')):
            malformed=copy.deepcopy(node)
            malformed[field]=value
            self.assertFalse(services.nodes_ready(dict(view,nodes=[malformed]),registered,templates,
                now_ms=1000,for_admission=False),field)
        self.assertFalse(services.nodes_ready(dict(view,nodes=[node,node]),registered,templates,now_ms=1000))
        node['templates'][0]['digest']=[3]*32
        self.assertFalse(services.nodes_ready(view,registered,templates,now_ms=1000,for_admission=False))

    def test_readiness_leaves_initial_preparation_running_instead_of_canceling_it_on_http_200(self):
        fixture=ReadinessSimulation(self.node(),commit_after=5.5)
        view=fixture.run(timeout=10)
        self.assertEqual(view['nodes'][0]['templates'][0]['revision'],'base-r1')
        self.assertEqual(fixture.canceled,0)
        self.assertEqual(fixture.stops,[])
        self.assertEqual(fixture.starts,[])
        fixture.processes.audit.assert_not_called()
        fixture.processes.observe.assert_called_once()
        self.assertTrue(all(request[:3]==('GET','/v2/sandboxes',()) for request in fixture.requests))

    def test_public_http_200_never_substitutes_for_original_persisted_cache_readiness(self):
        fixture=ReadinessSimulation(self.node(),never_ready=True)
        fixture.instance.center_config['reply_timeout_ms']=1000
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            fixture.run(timeout=6)
        self.assertEqual(fixture.now,6)
        self.assertGreaterEqual(len(fixture.observations),1)
        fixture.processes.audit.assert_not_called()
        self.assertEqual(fixture.stops,[])
        fixture.processes.observe.assert_not_called()
        self.assertFalse(hasattr(fixture.instance,'_readiness_catalog_observed'))

    def test_host_exit_is_observed_during_the_uninterrupted_preparation_window(self):
        fixture=ReadinessSimulation(self.node(),commit_after=5.5,host_exit_at=2)
        with self.assertRaisesRegex(RuntimeError,'original service exited'):
            fixture.run(timeout=10)
        self.assertLess(fixture.now,2.3)
        self.assertEqual(fixture.stops,[])
        fixture.processes.audit.assert_not_called()
        fixture.processes.observe.assert_not_called()

    def test_lost_online_observation_does_not_restart_original_services(self):
        fixture=ReadinessSimulation(self.node(),missing_observations=1)
        fixture.instance.center_config['reply_timeout_ms']=1000
        fixture.run(timeout=6)
        self.assertEqual(len(fixture.observations),2)
        self.assertEqual(fixture.stops,[])
        fixture.processes.audit.assert_not_called()
        fixture.processes.observe.assert_called_once()

    def test_each_call_requires_fresh_online_observation_without_a_readiness_hint(self):
        fixture=ReadinessSimulation(self.node())
        fixture.run(timeout=10)
        began=fixture.now
        fixture.run(timeout=3,for_admission=False)
        self.assertGreater(fixture.now,began)
        self.assertEqual(len(fixture.observations),4)
        self.assertEqual(fixture.processes.observe.call_count,2)
        # Earlier success is never health/cache evidence for a later call.
        fixture.never_ready=True
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            fixture.run(timeout=3,for_admission=False)
        self.assertEqual(fixture.processes.observe.call_count,2)

    def test_cache_observation_does_not_hide_a_failed_actual_role_observation(self):
        fixture=ReadinessSimulation(self.node())
        fixture.processes.observe.side_effect=[ValueError('role exec not observed yet'),{}]
        fixture.run(timeout=8)
        self.assertEqual(len(fixture.observations),3)
        self.assertEqual(fixture.stops,[])
        self.assertEqual(fixture.processes.observe.call_count,2)
        refused=ReadinessSimulation(self.node())
        refused.processes.observe.side_effect=ValueError('role exec never observed')
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            refused.run(timeout=8)
        self.assertGreaterEqual(refused.processes.observe.call_count,1)

    def test_unexpired_pre_restart_health_cannot_substitute_for_a_new_host_status_round(self):
        fixture=ReadinessSimulation(self.node(),health_fresh_after=2)
        # The protected catalog and old health lease are valid throughout this
        # interval, but the original host has not yet answered a fresh status.
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            fixture.run(timeout=1)
        fixture.processes.observe.assert_not_called()
        fixture.run(timeout=3,for_admission=False)
        self.assertGreaterEqual(fixture.now,2)
        self.assertLess(fixture.now,2.5)
        self.assertEqual(fixture.stops,[])
        fixture.processes.audit.assert_not_called()
        fixture.processes.observe.assert_called_once()

    def test_changed_expected_catalog_cannot_reuse_previous_ready_result(self):
        fixture=ReadinessSimulation(self.node())
        fixture.run(timeout=10)
        fixture.instance.expected_templates['host-runtime'].add('different-r1')
        began=fixture.now
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            fixture.run(timeout=3)
        self.assertAlmostEqual(fixture.now-began,3)
        self.assertGreater(len(fixture.observations),1)
        self.assertEqual(fixture.stops,[])

    def test_default_readiness_uses_download_budget_without_canceling_slow_preparation(self):
        fixture=ReadinessSimulation(self.node(),commit_after=75)
        fixture.run()
        self.assertGreaterEqual(fixture.now,75)
        self.assertLess(fixture.now,76)
        self.assertEqual(fixture.stops,[])
        fixture.processes.audit.assert_not_called()
        fixture.processes.observe.assert_called_once()

    def test_default_download_bound_is_not_reset_by_failed_polls(self):
        fixture=ReadinessSimulation(self.node(),never_ready=True)
        fixture.instance.host_configs[0]['artifact_source']['download_timeout_ms']=3000
        with self.assertRaisesRegex(RuntimeError,'no sandbox creation attempted'):
            fixture.run()
        self.assertEqual(fixture.now,3)
        self.assertEqual(fixture.stops,[])

    def test_recovery_audits_leave_each_restarted_worker_time_to_commit(self):
        interrupted=ReadinessSimulation(self.node(),commit_after=3)
        interrupted.processes.audit.side_effect=lambda name:dict(committed=interrupted.committed)
        for _ in range(8):
            self.assertFalse(interrupted.run(audit='center')['committed'])
            interrupted.sleep(.5)
        self.assertEqual(interrupted.canceled,8)
        fixture=ReadinessSimulation(self.node(),commit_after=3)
        # Cleanup must progress even when no capacity is available for Create.
        fixture.node['free']['instances']=0
        fixture.processes.audit.side_effect=lambda name:dict(committed=fixture.committed)
        for _ in range(2):
            fixture.committed=False
            observed=fixture.run(audit='center',ready_deadline=10)
            self.assertTrue(observed['committed'])
        self.assertEqual(fixture.canceled,0)
        self.assertGreaterEqual(fixture.stops[0],3)
        self.assertGreaterEqual(fixture.stops[1]-fixture.starts[0],3)
        self.assertLess(fixture.now,10)
        fixture.processes.initialize.assert_not_called()

    def test_recovery_wait_preserves_both_original_absolute_and_download_bounds(self):
        for deadline,download_ms,expected in ((2,900000,2),(20,3000,3)):
            with self.subTest(deadline=deadline,download_ms=download_ms):
                fixture=ReadinessSimulation(self.node(),never_ready=True)
                fixture.instance.host_configs[0]['artifact_source']['download_timeout_ms']=download_ms
                with self.assertRaisesRegex(RuntimeError,'readiness unobserved'):
                    fixture.run(audit='center',ready_deadline=deadline)
                self.assertEqual(fixture.now,expected)
                self.assertEqual(fixture.stops,[])
                fixture.processes.audit.assert_not_called()
                # A later callback cannot reset the absolute recovery deadline.
                if expected==deadline:
                    with self.assertRaises(TimeoutError):
                        fixture.run(audit='center',ready_deadline=deadline)
                    self.assertEqual(fixture.now,deadline)

    def test_recovery_never_stops_a_role_after_expiry_or_invalid_deadline(self):
        for deadline,error in ((0,TimeoutError),(-1,TimeoutError),(True,ValueError),
                               (float('nan'),ValueError),(float('inf'),ValueError)):
            with self.subTest(deadline=deadline):
                fixture=ReadinessSimulation(self.node())
                with self.assertRaises(error):
                    fixture.run(audit='center',ready_deadline=deadline)
                self.assertEqual(fixture.stops,[])
                fixture.processes.audit.assert_not_called()

    def test_recovery_role_exit_and_failed_audit_remain_failures(self):
        fixture=ReadinessSimulation(self.node(),commit_after=3,host_exit_at=1)
        with self.assertRaisesRegex(RuntimeError,'service exited'):
            fixture.run(audit='center',ready_deadline=10)
        self.assertEqual(fixture.stops,[])
        fixture.processes.audit.assert_not_called()
        fixture=ReadinessSimulation(self.node())
        fixture.processes.audit.side_effect=ValueError('original audit rejected')
        with self.assertRaisesRegex(ValueError,'original audit rejected'):
            fixture.run(audit='center',ready_deadline=10)
        self.assertEqual(len(fixture.stops),1)
        self.assertEqual(len(fixture.starts),1)
        fixture.processes.initialize.assert_not_called()

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
                node_state_dir=str(root/'state'),chroot_base=str(root/'p/v'),kernel=str(root/'kernel'),
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
                node_state_dir=str(root/'state'),chroot_base=str(root/'p/v'),kernel=str(root/'kernel'),
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
