"""Configuration and exact-identity tests only; no process, TLS, Redis or KVM."""
import copy
import hashlib
import importlib.util
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


PATH = Path(__file__).resolve().parents[2] / 'tests/system/_center_host_services.py'
SPEC = importlib.util.spec_from_file_location('center_host_fixture', PATH)
fixture = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(fixture)


class FixtureTests(unittest.TestCase):
    def terminal_audits(self):
        binding = dict(protocol_version=4, node=dict(deployment_id='domain', node_id='host-runtime', node_generation=1),
            session='sandbox-29', key=dict(deployment='domain', owner='tenant-1', window=7, sequence=8),
            expected=dict(state='absent'))
        identity = dict(binding=binding, center=dict(deployment='domain', center='center-a'),
            retention=dict(slot=1, window=1), accepted_version=1)
        latest = copy.deepcopy(binding)
        latest['key']['sequence'] = 10
        latest['expected'] = dict(state='version', version=2)
        response = dict(request=dict(request=dict(binding=latest, operation=dict(kind='destroy')),
            accepted_version=3, center=copy.deepcopy(identity['center']), retention=dict(slot=1, window=1)), status='cleaned')
        versions = dict(internal_protocol=2, center_store=11, host_store=23)
        first = copy.deepcopy(identity) | dict(operation=dict(kind='create', target_incarnation=1),
            acceptance_time=dict(kind='sampled', at_ms=1000), running=None,
            create_refusal=None, terminal=None, unloaded=None)
        last = dict(binding=latest, accepted_version=3, center=copy.deepcopy(identity['center']),
            retention=dict(slot=1, window=1), operation=dict(kind='destroy'),
            acceptance_time=dict(kind='unsampled'), running=None, create_refusal=None,
            terminal=copy.deepcopy(response), unloaded=None)
        center = dict(schema_version=3, versions=copy.deepcopy(versions), deployment='domain',
            retention_lanes=[dict(node=copy.deepcopy(binding['node']), retention_slot=1, retired_through=0,
                phase='open', last_retired=None)], sandboxes=[dict(create=dict(binding=binding), released=True,
                operations=[first, last], terminal_confirmation=copy.deepcopy(response))])
        host = dict(schema_version=2, versions=copy.deepcopy(versions), node=copy.deepcopy(binding['node']),
            retention_lanes=[dict(slot=1, retired_through=0, current_window=1, last_retired=None)],
            operations=[dict(binding=copy.deepcopy(latest), accepted_version=3, current=True, terminal=True,
                local=None, local_operation=None, confirmed_terminal=copy.deepcopy(response))])
        return identity, center, host

    def test_terminal_observation_requires_exact_two_layer_confirmation_not_stopped_or_absence(self):
        identity, center, host = self.terminal_audits()
        self.assertEqual(fixture.terminal_result(center, host, identity)['status'], 'cleaned')
        for target, field, value in ((center['sandboxes'][0], 'released', False),
                (center['sandboxes'][0], 'terminal_confirmation', None),
                (host['operations'][0], 'confirmed_terminal', None)):
            before = copy.deepcopy(target[field])
            target[field] = value
            self.assertIsNone(fixture.terminal_result(center, host, identity))
            target[field] = before
        center['sandboxes'][0]['operations'][-1]['terminal']['status'] = 'pending'
        self.assertIsNone(fixture.terminal_result(center, host, identity))
        center['sandboxes'] = []
        self.assertIsNone(fixture.terminal_result(center, host, identity))
        host['operations'] = []
        self.assertIsNone(fixture.terminal_result(center, host, identity))

    def test_retired_evidence_requires_both_exact_original_slot_frontiers_and_saved_incarnation(self):
        identity, center, host = self.terminal_audits()
        response = copy.deepcopy(center['sandboxes'][0]['terminal_confirmation'])
        center['sandboxes'] = []
        host['operations'] = []
        center['retention_lanes'][0].update(retired_through=1, last_retired=copy.deepcopy(response))
        self.assertIsNone(fixture.terminal_result(center, host, identity))
        host['retention_lanes'][0].update(retired_through=1, current_window=2, last_retired=copy.deepcopy(response))
        self.assertEqual(fixture.terminal_result(center, host, identity)['status'], 'retired_terminal_slot')
        foreign = copy.deepcopy(host)
        foreign['node']['node_generation'] = 2
        with self.assertRaises(ValueError):
            fixture.terminal_result(center, foreign, identity)
        for bad in (True, 0, 2**64):
            altered = copy.deepcopy(identity)
            altered['retention']['window'] = bad
            with self.subTest(window=bad), self.assertRaises(ValueError):
                fixture.terminal_result(center, host, altered)
        host['retention_lanes'][0]['retired_through'] = True
        with self.assertRaises(ValueError):
            fixture.terminal_result(center, host, identity)

    def test_retired_stamp_is_exact_and_other_slot_cannot_retire_the_saved_identity(self):
        identity, center, host = self.terminal_audits()
        response = copy.deepcopy(center['sandboxes'][0]['terminal_confirmation'])
        center['sandboxes'] = []
        host['operations'] = []
        other = copy.deepcopy(response)
        other['request']['retention'] = dict(slot=2, window=8)
        other['request']['request']['binding']['session'] = 'unrelated-sandbox'
        center['retention_lanes'].append(dict(node=identity['binding']['node'], retention_slot=2,
            retired_through=8, phase='open', last_retired=copy.deepcopy(other)))
        host['retention_lanes'].append(dict(slot=2, retired_through=8, current_window=9, last_retired=copy.deepcopy(other)))
        self.assertIsNone(fixture.terminal_result(center, host, identity))
        for lane in (center['retention_lanes'][0], host['retention_lanes'][0]):
            lane.update(retired_through=1, last_retired=copy.deepcopy(response))
        malformed = []
        for path, value in ((('request', 'request', 'binding', 'key', 'owner'), 'other-owner'),
                (('request', 'request', 'binding', 'session'), 'other-sandbox'),
                (('request', 'retention', 'slot'), 2), (('request', 'retention', 'window'), 2),
                (('request', 'request', 'binding', 'protocol_version'), 1),
                (('request', 'request', 'binding', 'key', 'sequence'), 11), (('status',), 'pending')):
            stamp = copy.deepcopy(response)
            target = stamp
            for part in path[:-1]: target = target[part]
            target[path[-1]] = value
            malformed.append(stamp)
        malformed.append(None)
        for stamp in malformed:
            changed = copy.deepcopy(host)
            changed['retention_lanes'][0]['last_retired'] = stamp
            with self.subTest(stamp=stamp), self.assertRaises(ValueError):
                fixture.terminal_result(center, changed, identity)
        newer = copy.deepcopy(response)
        newer['request']['request']['binding']['session'] = 'later-sandbox'
        newer['request']['request']['binding']['key']['sequence'] = 12
        newer['request']['retention']['window'] = 2
        for lane in (center['retention_lanes'][0], host['retention_lanes'][0]):
            lane.update(retired_through=2, last_retired=copy.deepcopy(newer))
        self.assertEqual(fixture.terminal_result(center, host, identity)['status'], 'retired_terminal_slot')

    def test_audit_schema_and_full_terminal_stamp_refuse_missing_or_loosely_typed_fields(self):
        identity, center, host = self.terminal_audits()
        response = copy.deepcopy(center['sandboxes'][0]['terminal_confirmation'])
        for lane in (center['retention_lanes'][0], host['retention_lanes'][0]):
            lane.update(retired_through=1, last_retired=copy.deepcopy(response))
        for audit in (center, host):
            for schema in (1, True, float(audit['schema_version'])):
                changed = copy.deepcopy(audit)
                changed['schema_version'] = schema
                pair = (changed, host) if audit is center else (center, changed)
                with self.subTest(schema=schema), self.assertRaises(ValueError):
                    fixture.terminal_result(*pair, identity)
        for path, value in ((('request',), None), (('request', 'accepted_version'), True),
                (('request', 'retention', 'slot'), True), (('request', 'retention', 'window'), True),
                (('request', 'request', 'operation'), {}), (('request', 'request', 'binding'), {}),
                (('extra',), 'unknown')):
            stamp = copy.deepcopy(response)
            target = stamp
            for part in path[:-1]: target = target[part]
            target[path[-1]] = value
            changed_center, changed_host = copy.deepcopy(center), copy.deepcopy(host)
            changed_center['retention_lanes'][0]['last_retired'] = copy.deepcopy(stamp)
            changed_host['retention_lanes'][0]['last_retired'] = copy.deepcopy(stamp)
            with self.subTest(stamp=stamp), self.assertRaises(ValueError):
                fixture.terminal_result(changed_center, changed_host, identity)

    def native(self):
        return dict(namespace='a'*16, maintenance=dict(max_operations_per_sandbox=8),
            max_active_global=2, max_active_per_tenant=2, max_disk_bytes_global=12000,
            max_disk_bytes_per_tenant=8000, cleanup_reserve_bytes=4000,
            max_timeout_secs=86400, max_lifetime_secs=86400, startup_timeout_secs=300,
            public_ipv4_enabled=False, expiry_scan_seconds=1,
            data_transfer=dict(max_transfers=8, file_upload_bytes=64*1024**2,
                               file_download_bytes=96*1024**2),
            bounded_host=dict(physical_storage=dict(pools=[dict(namespace='a'*16,role='runtime')],
                host_template_caches=[dict(namespace='a'*16, root='/approved/host/cache',
                max_entries=2, max_downloads=1, max_entry_bytes=1000, artifact_bytes=2000,
                temporary_bytes=1000, metadata_bytes=282624, inode_budget=19)])),
            templates=dict(templates=[dict(template_id='runtime-base', revision='r1', owner=1)],
                aliases=dict(base='runtime-base'), compute_limits=dict(max_vcpus_global=4,
                    max_vcpus_per_tenant=2, max_memory_bytes_global=4000,
                    max_memory_bytes_per_tenant=2000)))

    @staticmethod
    def captures(pools):
        # Explicit controlled inputs only; no quota/tool-success claim.
        return [dict(layout_parent='/approved/capture-'+str(index),layout_binding=[1]*32,
            state_copy_bytes=1000000,memory_copy_bytes=536870912,disk_copy_bytes=8000,
            api_timeout_ms=1000,worker_timeout_ms=300000,
            material_project=dict(project_id=63801+index,hard_bytes=1073741824,hard_inodes=1024,
                chattr=dict(path='/protected/chattr',sha256=[2]*32),
                lsattr=dict(path='/protected/lsattr',sha256=[3]*32))) for index,_ in enumerate(pools)]

    def render(self, pools=None, *, retained_sessions=4, redis_operation_timeout_ms=1000):
        native = pools or [self.native()]
        return fixture.render(native, capture_settings=self.captures(native), registered_catalogs=[p['templates'] for p in native], deployment='one-approved-deployment',
            center_root='/approved/center', host_tls_roots=['/approved/host/tls'],
            host_addresses=['127.0.0.1:9001'], public_address='127.0.0.1:9000',
            artifact_address='127.0.0.1:9002', redis_address='127.0.0.1:9003',
            password='TESTONLYREDISKEY', key='TESTONLYPUBLICKEY', retained_sessions=retained_sessions,
            registry=dict(root='/approved/center/registry', max_templates_global=2),
            versions=dict(internal_protocol=2, center_store=6, host_store=16),
            redis_operation_timeout_ms=redis_operation_timeout_ms)

    def test_explicit_redis_wait_changes_only_the_original_store_timeout(self):
        original,original_hosts=self.render()
        self.assertEqual(original['redis']['operation_timeout_ms'],1000)
        adjusted,adjusted_hosts=self.render(redis_operation_timeout_ms=5000)
        self.assertEqual(adjusted['redis']['operation_timeout_ms'],5000)
        self.assertEqual(adjusted_hosts,original_hosts)
        adjusted['redis']['operation_timeout_ms']=1000
        self.assertEqual(adjusted,original)
        self.assertEqual(original['reply_timeout_ms'],25000)
        config=fixture.redis_configuration('/approved/redis',port=6379,password='TESTONLY')
        self.assertIn('appendonly yes\nappendfsync always\n',config)
        self.assertIn('no-appendfsync-on-rewrite no\naof-load-truncated no\n',config)

    def test_redis_wait_limits_match_the_existing_product_and_reject_nonintegers(self):
        for value in (1,60000):
            with self.subTest(value=value):
                center,_=self.render(redis_operation_timeout_ms=value)
                self.assertEqual(center['redis']['operation_timeout_ms'],value)
        for value in (None,True,False,0,-1,60001,5000.0,'5000'):
            with self.subTest(value=value),self.assertRaisesRegex(ValueError,'Redis operation timeout'):
                self.render(redis_operation_timeout_ms=value)

    def test_public_credentials_are_only_at_center_and_original_native_budgets_unchanged(self):
        source = self.native()
        original = copy.deepcopy(source)
        center, hosts = self.render([source])
        self.assertEqual(source, original)
        self.assertEqual(center['credentials'], [dict(tenant=1, secret='TESTONLYPUBLICKEY')])
        self.assertEqual(hosts[0]['native'], source)
        self.assertNotIn('credentials', hosts[0])
        self.assertNotIn('TESTONLYPUBLICKEY', str(hosts))
        self.assertEqual(hosts[0]['inbox']['node'], center['nodes'][0]['registration']['identity'])
        self.assertEqual(center['nodes'][0]['registration']['retention_slots'],hosts[0]['inbox']['max_sessions'])
        self.assertNotIn('template_registrations', hosts[0])
        self.assertEqual(center['template_registrations'][0]['template_id'], center['templates']['templates'][0]['template_id'])

    def test_explicit_two_owner_fixture_preserves_shared_platform_and_host_has_no_keys(self):
        native=self.native()
        def render(extra):
            return fixture.render([native],capture_settings=self.captures([native]),registered_catalogs=[native['templates']],deployment='one',
                center_root='/approved/center',host_tls_roots=['/approved/host/tls'],
                host_addresses=['127.0.0.1:9001'],public_address='127.0.0.1:9000',
                artifact_address='127.0.0.1:9002',redis_address='127.0.0.1:9003',
                password='TESTREDIS',key='OWNERONE',retained_sessions=2 + native['max_active_global'],registry={'root':'/approved/registry'},
                versions=dict(internal_protocol=2,center_store=6,host_store=16),extra_credentials=extra)
        base,_=render([])
        center,hosts=render([dict(tenant=2,secret='OWNERTWO')])
        self.assertEqual([value['tenant'] for value in center['credentials']],[1,2])
        self.assertEqual(center['bounds']['platform'],base['bounds']['platform'])
        self.assertEqual([value['limit'] for value in center['tenants']],[base['tenants'][0]['limit']]*2)
        self.assertNotIn('OWNERTWO',str(hosts))
        for extra in ([dict(tenant=1,secret='OTHER')],[dict(tenant=2,secret='OWNERONE')],
                      [dict(tenant=True,secret='OTHER')],[dict(tenant=2,secret='\nprivate')]):
            with self.subTest(extra=extra),self.assertRaises(ValueError):
                render(extra)

    def test_retired_public_host_and_build_fields_are_rejected_not_silently_upgraded(self):
        for name in ('bind','domain','credentials','request_admission','template_registry',
                     'internal_build_vms','build_retry_windows'):
            source=dict(self.native(),**{name:None})
            with self.subTest(field=name),self.assertRaisesRegex(ValueError,'not upgraded'):
                self.render([source])

    def test_retention_slots_use_existing_inbox_capacity_and_overflow_is_not_wrapped(self):
        source=self.native()
        center,hosts=self.render([source])
        self.assertEqual(hosts[0]['inbox']['max_sessions'],4)
        self.assertEqual(center['nodes'][0]['registration']['retention_slots'],4)
        self.assertEqual(source,self.native())
        with self.assertRaisesRegex(ValueError,'retention slot count'):
            self.render([source],retained_sessions=2**16)
        center,hosts=self.render([source],retained_sessions=2)
        self.assertEqual(hosts[0]['inbox']['max_sessions'],2)
        self.assertEqual(center['nodes'][0]['registration']['retention_slots'],2)
        self.assertEqual(center['bounds']['max_sandboxes'],2)
        self.assertEqual(source,self.native())

    def test_missing_or_insufficient_native_operation_bound_is_not_upgraded(self):
        for bound in (None,0,True,7,2**64):
            source=self.native()
            if bound is None:
                del source['maintenance']['max_operations_per_sandbox']
            else:
                source['maintenance']['max_operations_per_sandbox']=bound
            with self.subTest(bound=bound),self.assertRaisesRegex(ValueError,'operation bound'):
                self.render([source])
        for policy in (None,{}):
            source=self.native();source['maintenance']=policy
            with self.subTest(policy=policy),self.assertRaisesRegex(ValueError,'operation bound'):
                self.render([source])
        source=self.native();del source['maintenance']
        with self.assertRaisesRegex(ValueError,'operation bound'):
            self.render([source])

    def test_no_free_health_cache_or_canonical_digest_is_manufactured(self):
        center, hosts = self.render()
        node = center['nodes'][0]['registration']
        self.assertEqual(node['healthy_until_ms'], 0)
        self.assertEqual(node['free'], dict(instances=0, cpu=0, memory_bytes=0, disk_bytes=0))
        self.assertEqual(node['templates'], [])
        self.assertEqual(center['metadata_budget']['max_certificate_bytes'], fixture.CERTIFICATE_BYTES)
        self.assertEqual(center['redis']['max_block_bytes'], fixture.BLOCK_BYTES)
        self.assertEqual(center['redis']['memory_bytes'], fixture.REDIS_MEMORY_BYTES)
        self.assertEqual(set(center['bounds']), {'max_nodes', 'max_templates_per_node',
            'max_sandboxes', 'operations_per_sandbox', 'max_options_bytes',
            'result_retention_ms', 'platform'})
        self.assertEqual(center['bounds']['result_retention_ms'], 300_000)
        self.assertEqual(hosts[0]['inbox']['max_operations_per_session'], 8)
        self.assertEqual(center['bounds']['operations_per_sandbox'],8)
        self.assertEqual(hosts[0]['inbox']['max_operations_per_session'],
            center['bounds']['operations_per_sandbox'])
        self.assertEqual(hosts[0]['inbox']['center'],dict(deployment=center['deployment_id'],center=center['center_id']))
        self.assertIsNone(hosts[0]['inbox']['host_epoch'])
        self.assertEqual(hosts[0]['source_observation']['max_material_bytes'],hosts[0]['capture']['material_project']['hard_bytes'])
        self.assertGreater(hosts[0]['internal']['max_connections'], hosts[0]['native']['data_transfer']['max_transfers'])
        self.assertNotIn('passed', center)
        self.assertEqual(hosts[0]['template_cache'], hosts[0]['native']['bounded_host']['physical_storage']['host_template_caches'][0])
        self.assertEqual(hosts[0]['artifact_source']['tls']['artifact_leaves'], ['/approved/host/tls/center.der'])
        self.assertEqual(node['role'], 'runtime')

    def test_private_keys_are_role_specific_and_two_directions_pin_peers(self):
        center, hosts = self.render()
        self.assertNotIn('host-key.der', str(center))
        self.assertNotIn('center-key.der', str(hosts))
        self.assertEqual(hosts[0]['internal']['tls']['center_leaves'], ['/approved/host/tls/center.der'])
        self.assertEqual(center['nodes'][0]['peer_certificates_der'], ['/approved/center/tls/host-runtime.der'])
        self.assertEqual(center['artifact_service']['tls']['private_key_der'], '/approved/center/tls/center-key.der')

    def test_two_nodes_have_distinct_fixed_leaf_and_namespace_cache(self):
        runtime = self.native()
        builder = self.native()
        builder['namespace'] = 'b'*16
        builder['bounded_host']['physical_storage']['pools'] = [dict(namespace='b'*16,role='builder')]
        builder['templates']['templates'][0]['template_id'] = 'builder-base'
        builder['templates']['aliases'] = dict(base='builder-base')
        cache = builder['bounded_host']['physical_storage']['host_template_caches'][0]
        cache.update(namespace=builder['namespace'], root='/approved/builder/cache')
        center, hosts = fixture.render([runtime, builder],capture_settings=self.captures([runtime,builder]),
            registered_catalogs=[p['templates'] for p in (runtime,builder)], deployment='one-deployment',
            center_root='/approved/center', host_tls_roots=['/approved/runtime/tls','/approved/builder/tls'],
            host_addresses=['127.0.0.1:9001','127.0.0.1:9004'], public_address='127.0.0.1:9000',
            artifact_address='127.0.0.1:9002',redis_address='127.0.0.1:9003',
            password='TESTONLYREDISKEY',key='TESTONLYPUBLICKEY',retained_sessions=2 + sum(pool['max_active_global'] for pool in [runtime, builder]),
            registry=dict(root='/approved/center/registry', max_templates_global=2),
            versions=dict(internal_protocol=2,center_store=6,host_store=16))
        self.assertEqual([node['registration']['role'] for node in center['nodes']], ['runtime','builder'])
        self.assertEqual([node['peer_certificates_der'] for node in center['nodes']],
            [['/approved/center/tls/host-runtime.der'],['/approved/center/tls/host-builder.der']])
        self.assertEqual([host['template_cache']['namespace'] for host in hosts], ['a'*16,'b'*16])
        self.assertEqual([host['internal']['tls']['private_key'] for host in hosts],
            ['/approved/runtime/tls/host-key.der','/approved/builder/tls/host-key.der'])
        self.assertNotIn('/approved/builder/tls/host-key.der', str(hosts[0]))
        self.assertNotIn('/approved/runtime/tls/host-key.der', str(hosts[1]))
        # Each source catalog is valid, but prefixing the builder's "base"
        # alias must not shadow its canonical ID in the merged center catalog.
        self.assertEqual(center['templates']['aliases'], {'base': 'runtime-base'})
        self.assertEqual([binding['template_id'] for binding in center['templates']['templates']],
                         ['runtime-base', 'builder-base'])
        self.assertEqual(builder['templates']['aliases'], {'base': 'builder-base'})

    def test_builder_files_fit_center_without_changing_either_native_pool(self):
        runtime = self.native()
        builder = self.native()
        builder['namespace'] = 'b'*16
        builder['bounded_host']['physical_storage']['pools'] = [dict(namespace='b'*16,role='builder')]
        builder['bounded_host']['physical_storage']['host_template_caches'][0].update(
            namespace='b'*16, root='/approved/builder/cache')
        builder['templates']['templates'][0]['template_id'] = 'builder-base'
        builder['templates']['aliases'] = dict(base='builder-base')
        # The original builder admits the fixed 77,683,250-byte buildkitd
        # upload and a complete 10 GiB candidate. The runtime pool remains
        # limited to 64/96 MiB; its limits cannot configure both routes.
        builder['data_transfer'].update(max_transfers=16, file_upload_bytes=4*1024**3+65536,
                                        file_download_bytes=10*1024**3)
        pools = [runtime, builder]
        before = copy.deepcopy(pools)
        center, hosts = fixture.render(pools,capture_settings=self.captures(pools),
            registered_catalogs=[p['templates'] for p in pools], deployment='one-deployment',
            center_root='/approved/center', host_tls_roots=['/approved/runtime/tls','/approved/builder/tls'],
            host_addresses=['127.0.0.1:9001','127.0.0.1:9004'], public_address='127.0.0.1:9000',
            artifact_address='127.0.0.1:9002',redis_address='127.0.0.1:9003',
            password='TESTONLYREDISKEY',key='TESTONLYPUBLICKEY',retained_sessions=2 + sum(pool['max_active_global'] for pool in pools),
            registry=dict(root='/approved/center/registry', max_templates_global=2),
            versions=dict(internal_protocol=2,center_store=6,host_store=16))
        self.assertGreaterEqual(center['data_transfer']['file_upload_bytes'], 77683250)
        self.assertGreaterEqual(center['data_transfer']['file_download_bytes'], 10*1024**3)
        self.assertEqual(center['data_transfer']['file_upload_bytes'], 4*1024**3+65536)
        self.assertEqual(center['data_transfer']['file_download_bytes'], 10*1024**3)
        self.assertEqual(center['data_transfer']['max_transfers'], 8)
        self.assertEqual([host['native'] for host in hosts], before)
        self.assertEqual(pools, before)
        self.assertEqual(center['request_admission']['max_pending_requests'], 12)
        single, _ = self.render([runtime])
        self.assertEqual(single['data_transfer'], runtime['data_transfer'])
        self.assertEqual(single['data_transfer']['file_upload_bytes'], 64*1024**2)

    def test_alias_cannot_redirect_a_canonical_template_identity(self):
        native = self.native()
        native['templates']['templates'].append(dict(template_id='other', revision='r1', owner=1))
        native['templates']['aliases']['other'] = 'runtime-base'
        with self.assertRaisesRegex(ValueError, 'shadows another template identity'):
            self.render([native])

    def test_numeric_role_refusals_are_before_filesystem_or_launch(self):
        self.assertEqual(fixture.identities(65532, 65532, 65533, 65533, uid_base=3100000),
                         dict(host=(0, 0), center=(65532, 65532), redis=(65533, 65533)))
        for pair in ((0, 0, 65533, 65533), (True, 65532, 65533, 65533),
                     (65532, 65532, 65532, 65533), (3100000, 65532, 65533, 65533)):
            with self.subTest(pair=pair), patch.object(fixture.os, 'open') as files, \
                    patch.object(fixture.subprocess, 'Popen') as launch, self.assertRaises(ValueError):
                fixture.identities(*pair, uid_base=3100000)
            files.assert_not_called()
            launch.assert_not_called()

    def test_staging_rejects_unknown_roles_and_missing_material_before_effects(self):
        with patch.object(fixture,'trusted_path') as paths,patch.object(fixture.os,'chown') as owners:
            for name in ('unsafe/name','sp292-'+'z'*16,'../root'):
                with self.subTest(name=name),self.assertRaises(ValueError):
                    fixture.stage_role_directories('/approved',name,['/approved/host'],
                        service_ids=dict(center=(1001,1001),redis=(1002,1002),host=(0,0)))
            paths.assert_not_called()
            owners.assert_not_called()
        with patch.object(fixture,'private_path') as paths,patch.object(fixture.os,'chown') as owners:
            with self.assertRaisesRegex(ValueError,'complete exact fixed TLS'):
                fixture.stage_tls('/approved',{},center='/approved/center',hosts=['/approved/host'],
                    center_identity=(1001,1001))
            with self.assertRaises(ValueError):
                fixture.stage_tls('/approved',{},center='/approved/center',hosts=[],center_identity=(1001,1001))
            paths.assert_not_called()
            owners.assert_not_called()

    def test_fixed_tls_copy_contains_only_each_role_key_and_rejects_changed_receipt(self):
        # Real bounded source files; owner/chown at the destination is mocked.
        # This proves copy selection, NOT DER validity, Linux roles or mTLS.
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary).resolve()
            material=root/'fixed'
            material.mkdir(mode=0o700)
            names=fixture.tls_material_files(('runtime','builder'))
            expected={}
            for name in names:
                path=material/name
                path.write_bytes(('test-only-not-a-certificate-'+name).encode())
                path.chmod(0o600)
                expected[name]=hashlib.sha256(path.read_bytes()).hexdigest()
            center=root/'center'
            hosts=[root/'runtime',root/'builder']
            for path in [center,*hosts]:
                path.mkdir(mode=0o700)
            original_lstat=Path.lstat
            def owners(path):
                info=original_lstat(path)
                if path in [center,*hosts]:
                    uid=1001 if path==center else 0
                    return SimpleNamespace(st_uid=uid,st_gid=uid,st_mode=info.st_mode)
                return info
            with patch.object(Path,'lstat',owners),patch.object(fixture.os,'chown'), \
                    patch.object(fixture,'write_role') as copied:
                roots=fixture.stage_tls(material,expected,center=center,hosts=hosts,center_identity=(1001,1001))
            self.assertEqual(roots,[path/'tls' for path in [center,*hosts]])
            contents={str(call.args[0]):call.args[1] for call in copied.call_args_list}
            self.assertNotIn(str(center/'tls/host-key.der'),contents)
            for role,path in zip(('runtime','builder'),hosts):
                self.assertEqual(contents[str(path/'tls/host-key.der')],(material/('host-'+role+'-key.der')).read_bytes())
                self.assertEqual(contents[str(path/'tls/host.der')],(material/('host-'+role+'.der')).read_bytes())
                self.assertNotIn(str(path/'tls/center-key.der'),contents)
            expected['host-runtime.der']='0'*64
            with patch.object(fixture.os,'chown') as changed,patch.object(fixture,'write_role') as written:
                with self.assertRaisesRegex(ValueError,'TLS material changed'):
                    fixture.stage_tls(material,expected,center=center,hosts=hosts,center_identity=(1001,1001))
                changed.assert_not_called()
                written.assert_not_called()

    def test_role_facts_are_observed_exactly_and_refuse_replaced_or_exposed_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o700)
            config = root / 'config.json'
            config.write_text('{"test_only":true}')
            config.chmod(0o600)
            pair = dict(uid=root.stat().st_uid, gid=root.stat().st_gid)
            facts = fixture.role_facts(root, config, **pair)
            self.assertEqual(facts['directory']['inode'], root.stat().st_ino)
            self.assertEqual(facts['directory']['mode'], 0o700)
            self.assertEqual(len(facts['config_sha256']), 64)
            self.assertNotIn(str(root), str(facts))
            with self.assertRaises(ValueError):
                fixture.role_facts(root, config, uid=pair['uid'] + 1, gid=pair['gid'])
            config.chmod(0o644)
            with self.assertRaises(ValueError):
                fixture.role_facts(root, config, **pair)
            config.chmod(0o600)
            link = root / 'link.json'
            link.symlink_to(config)
            with self.assertRaises(ValueError):
                fixture.role_facts(root, link, **pair)
            root.chmod(0o755)
            with self.assertRaises(ValueError):
                fixture.role_facts(root, config, **pair)

    def test_launch_has_only_unique_cli_and_original_cgroup_privilege_drop(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(fixture.service_cgroup, 'prefix', return_value=['approved-launcher']) as prefix, \
                patch.object(fixture.subprocess, 'Popen') as launch:
            fixture.launch('/approved/sandproof-center', '/approved/center.json',
                Path(temporary) / 'log', '/approved/record', uid=65532, gid=65532)
            prefix.assert_called_once_with('/approved/record', uid=65532, gid=65532)
            self.assertEqual(launch.call_args.args[0], ['approved-launcher', '/approved/sandproof-center',
                                                       'serve', '--config', '/approved/center.json'])
            self.assertNotIn('preexec_fn', launch.call_args.kwargs)
            with self.assertRaises(ValueError):
                fixture.launch('/approved/sandproof-center', '/approved/center.json',
                    Path(temporary) / 'log', '/approved/record', action='retired')
            self.assertEqual(launch.call_count, 1)

    def test_global_identity_is_never_converted_by_parsing_suffix(self):
        node = dict(deployment_id='one', node_id='host-runtime', node_generation=3)
        identity = dict(protocol_version=4, node=node, session='sandbox-999',
            key=dict(deployment='one', owner='4', window=1, sequence=1), expected=dict(state='absent'))
        local = dict(sandbox=17, owner=4)
        row = dict(current=True, binding=identity, accepted_version=1, local=local)
        audit = dict(schema_version=2, node=node, operations=[row])
        self.assertEqual(fixture.local_binding(audit, 'sandbox-999', expected_node=node), local)
        for changed in (dict(audit, operations=[]), dict(audit, operations=[row, row]),
                        dict(audit, operations=[dict(row, local=None)]),
                        dict(audit, operations=[dict(row, current=False)]),
                        dict(audit, node=dict(node, node_generation=2))):
            with self.subTest(audit=changed), self.assertRaises(ValueError):
                fixture.local_binding(changed, 'sandbox-999', expected_node=node)

    def test_lost_sdk_reply_correlates_hashed_saved_intent_not_raw_user_values_or_absence(self):
        metadata = dict(runtime_attempt='test-attempt-42', benchmark='terminal-bench')
        node = dict(deployment_id='one', node_id='host-runtime', node_generation=3)
        identity, _center, _host = self.terminal_audits()
        identity = copy.deepcopy(identity)
        identity['binding'].update(node=node, session='sandbox-999')
        identity['binding']['key']['deployment'] = 'one'
        identity['center']['deployment'] = 'one'
        checksum = hashlib.sha256(b'{"benchmark":"terminal-bench","runtime_attempt":"test-attempt-42"}').hexdigest()
        first = copy.deepcopy(_center['sandboxes'][0]['operations'][0])
        first.update(copy.deepcopy(identity))
        record = dict(private=False, create=dict(binding=identity['binding'], metadata_sha256=checksum), operations=[first])
        audit = dict(schema_version=3, versions=dict(internal_protocol=2,center_store=11,host_store=23), deployment='one', sandboxes=[record])
        self.assertEqual(fixture.find_runtime_create(audit,metadata,deployment='one'),identity)
        with self.assertRaisesRegex(RuntimeError, 'ambiguous'):
            fixture.find_runtime_create(dict(audit,sandboxes=[record,record]),metadata,deployment='one')
        for changed in (dict(audit,sandboxes=[]),
                        dict(audit,sandboxes=[dict(record,private=True)]), dict(audit,deployment='foreign')):
            with self.subTest(audit=changed), self.assertRaises(ValueError):
                fixture.find_runtime_create(changed,metadata,deployment='one')
        with self.assertRaises(ValueError):
            fixture.find_runtime_create(audit,dict(metadata,runtime_attempt='other'),deployment='one')

    def test_dedicated_redis_is_durable_noeviction_not_an_optional_cache(self):
        text = fixture.redis_configuration('/approved/redis', port=6379, password='TESTONLYKEY')
        for setting in ('bind 127.0.0.1', 'appendonly yes', 'appendfsync always',
                        'maxmemory-policy noeviction', 'aof-load-truncated no', 'set-proc-title no'):
            self.assertIn(setting + '\n', text)
        for port, password in ((True, 'KEY'), (0, 'KEY'), (6379, 'KEY\nunsafe')):
            with self.assertRaises(ValueError):
                fixture.redis_configuration('/approved/redis', port=port, password=password)

    def process_inputs(self, root):
        return dict(center=dict(binary='/approved/sandproof-center', config='/approved/center.json', uid=1001, gid=1001),
            redis=dict(binary='/approved/redis-server', config='/approved/redis.conf', uid=1002, gid=1002),
            hosts=[dict(binary='/approved/sandproof-host', config='/approved/host.json', uid=0, gid=0)],
            record=root / 'record.json', logs=root)

    def test_explicit_fault_target_restart_does_not_stop_other_roles_or_reinitialize(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(fixture, 'launch') as launch, \
                patch.object(fixture, 'command') as command:
            root = Path(temporary)
            assembly = fixture.Processes(**self.process_inputs(root))
            host, center = Mock(), Mock()
            host.poll.return_value = center.poll.return_value = None
            launch.side_effect = [host, center]
            assembly.start('host-0')
            assembly.start('center')
            with self.assertRaises(RuntimeError):
                assembly.start('center')
            with self.assertRaises(RuntimeError):
                assembly.audit('center')
            with self.assertRaises(ValueError):
                assembly.initialize('center')
            assembly.stop('center', force=True)
            center.kill.assert_called_once()
            host.kill.assert_not_called()
            host.terminate.assert_not_called()
            center.poll.return_value = 0
            restarted = Mock()
            launch.side_effect = [restarted]
            assembly.start('center')
            self.assertEqual(launch.call_args.args[:2], ('/approved/sandproof-center', '/approved/center.json'))
            command.assert_not_called()  # Recovery never reruns initialize.

    def test_fault_environment_is_host_only_bounded_and_never_inherited(self):
        faults = {'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION': 'start'}
        with tempfile.TemporaryDirectory() as temporary, patch.object(fixture, 'launch') as launched:
            assembly = fixture.Processes(**self.process_inputs(Path(temporary)))
            for role in ('center', 'redis'):
                with self.subTest(role=role), self.assertRaisesRegex(ValueError, 'cannot affect'):
                    assembly.start(role, env=faults)
            for values in ({'HTTP_PROXY': 'private-proxy'}, {'PATH': '/unsafe'},
                           {next(iter(faults)): ''}, {next(iter(faults)): '\0'},
                           {next(iter(faults)): 'x' * 4097}):
                with self.subTest(values=list(values)), self.assertRaises(ValueError):
                    assembly.start('host-0', env=values)
            launched.assert_not_called()
            assembly.start('host-0', env=faults)
            self.assertEqual(launched.call_args.kwargs['env'], faults)
        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(fixture.service_cgroup, 'prefix', return_value=['approved']), \
                patch.object(fixture.subprocess, 'Popen') as launched, \
                patch.dict(os.environ, {'API_KEY': 'secret-canary', 'HTTPS_PROXY': 'secret-canary'}):
            fixture.launch('/approved/sandproof-host', '/approved/host.json',
                           Path(temporary) / 'host.log', '/approved/record', env=faults)
            self.assertEqual(launched.call_args.kwargs['env'],
                             {'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8', **faults})

    def test_missing_shared_root_or_duplicate_role_configuration_fails_before_commands(self):
        with tempfile.TemporaryDirectory() as temporary, patch.object(fixture, 'launch') as launch, \
                patch.object(fixture, 'command') as command:
            inputs = self.process_inputs(Path(temporary))
            invalid = [dict(inputs, hosts=[]), dict(inputs, hosts=inputs['hosts']*2),
                       dict(inputs, redis=dict(inputs['redis'], uid=1001)),
                       dict(inputs, hosts=[dict(inputs['hosts'][0], uid=False)])]
            for values in invalid:
                with self.subTest(inputs=values), self.assertRaises(ValueError):
                    fixture.Processes(**values)
            assembly = fixture.Processes(**inputs)
            with self.assertRaises(ValueError):
                assembly.observe()
            with self.assertRaises(ValueError):
                assembly.audit('redis')
            with self.assertRaises(ValueError):
                assembly.start('retired-server')
            launch.assert_not_called()
            command.assert_not_called()

    def test_actual_role_observation_requires_exact_exec_uid_caps_and_original_cgroup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            proc = root / 'proc/123'
            proc.mkdir(parents=True)
            binary, config = root / 'sandproof-center', root / 'center.json'
            binary.write_bytes(b'controlled-not-executed')
            (proc / 'exe').symlink_to(binary)
            (proc / 'stat').write_text('123 (same child) ' + ' '.join(['S', *(['0']*18), '17']))
            expected = [str(binary), 'serve', '--config', str(config)]
            (proc / 'cmdline').write_bytes(b'\0'.join(map(os.fsencode, expected)) + b'\0')
            status = ('Uid:\t1001 1001 1001 1001\nGid:\t1001 1001 1001 1001\nGroups:\t\n'
                      'CapInh:\t0\nCapPrm:\t0\nCapEff:\t0\nCapAmb:\t0\n')
            (proc / 'status').write_text(status)
            process = Mock(pid=123)
            process.poll.return_value = None
            actual_path = Path
            def path(value):
                return root / 'proc' if value == '/proc' else actual_path(value)
            with patch.object(fixture, 'Path', side_effect=path), \
                    patch.object(fixture.service_cgroup, 'observe') as cgroup:
                observed = fixture.process_facts(process, binary, config, root / 'record', uid=1001, gid=1001)
                self.assertEqual((observed['pid'], observed['start_time_ticks']), (123, 17))
                cgroup.assert_called_once_with(root / 'record', pids=[123])
                for changed in (status.replace('CapEff:\t0', 'CapEff:\t1'),
                                status.replace('Groups:\t', 'Groups:\t0'),
                                status.replace('1001 1001 1001 1001', '1001 1001 0 1001')):
                    (proc / 'status').write_text(changed)
                    with self.assertRaises(ValueError):
                        fixture.process_facts(process, binary, config, root / 'record', uid=1001, gid=1001)
                (proc / 'status').write_text(status)
                (proc / 'cmdline').write_bytes(b'retired-server\0configuration\0')
                with self.assertRaisesRegex(ValueError, 'unique product entry'):
                    fixture.process_facts(process, binary, config, root / 'record', uid=1001, gid=1001)


if __name__ == '__main__':
    unittest.main()
