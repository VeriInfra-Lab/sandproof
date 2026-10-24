"""Sample wiring contracts, not a second product schema or systemd/KVM run.

The ORIGINAL Rust deployment tests deserialize and validate both JSON samples.
These checks retain role separation, one CLI and the host's narrow stop policy.
"""
import configparser
import json
from pathlib import Path
import shlex
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEPLOY = ROOT / 'deploy'


def unit(role):
    value = configparser.ConfigParser(interpolation=None)
    value.optionxform = str
    value.read(DEPLOY / ('sandproof-' + role + '.service'))
    return value


class DeploymentExamplesTests(unittest.TestCase):
    def test_only_three_explicit_role_units_no_initialization_or_old_entry(self):
        for role in ('center', 'host', 'redis'):
            service = unit(role)['Service']
            command = shlex.split(service['ExecStart'])
            self.assertNotIn('ExecStartPre', service)
            self.assertNotIn('ExecStop', service)
            self.assertNotIn('ExecStartPost', service)
            self.assertNotIn('initialize', command)
            self.assertNotIn('sandproof-server', command)
            if role != 'redis':
                self.assertEqual(Path(command[0]).name, 'sandproof-' + role)
                self.assertEqual(command[1:3], ['serve', '--config'])
                self.assertEqual(Path(command[3]).name, role + '.json')
            else:
                self.assertEqual(Path(command[0]).name, 'redis-server')
                self.assertEqual(Path(command[1]).name, 'redis.conf')
            self.assertEqual(service['UMask'], '0077')
            self.assertEqual(service['StandardOutput'], 'null')
            self.assertEqual(service['StandardError'], 'null')

    def test_nonroot_roles_have_no_host_permissions_or_shared_writable_role_tree(self):
        center, redis = unit('center')['Service'], unit('redis')['Service']
        self.assertEqual(center['User'], 'sandproof-center')
        self.assertEqual(redis['User'], 'sandproof-redis')
        self.assertNotEqual(center['User'], redis['User'])
        for service, role, peer in ((center, 'center', 'redis'), (redis, 'redis', 'center')):
            self.assertEqual(service['NoNewPrivileges'], 'yes')
            self.assertEqual(service['CapabilityBoundingSet'], '')
            self.assertEqual(service['PrivateDevices'], 'yes')
            self.assertEqual(service['ProtectSystem'], 'strict')
            self.assertEqual(service['ReadWritePaths'], '/var/lib/sandproof-services/' + role)
            self.assertEqual(set(service['InaccessiblePaths'].split()), {
                '/var/lib/sandproof-host', '/var/lib/sandproof-services/' + peer})

    def test_host_stop_targets_only_main_service_not_original_vm_cgroups(self):
        service = unit('host')['Service']
        self.assertEqual(service['User'], 'root')
        self.assertEqual(service['KillMode'], 'process')
        self.assertEqual(service['KillSignal'], 'SIGTERM')
        self.assertEqual(service['SendSIGKILL'], 'yes')
        self.assertEqual(service['Delegate'], 'yes')
        host = json.loads((DEPLOY / 'host.example.json').read_text())
        vm_parent = Path(host['native']['host_resources']['cgroup_parent'])
        self.assertEqual(vm_parent, Path('/sys/fs/cgroup/sandproof-vms'))
        self.assertNotIn('sandproof-host.service', vm_parent.parts)
        self.assertNotIn('ExecStop', service)

    def test_sample_role_owners_identity_and_original_retention_capacity_are_one_binding(self):
        host = json.loads((DEPLOY / 'host.example.json').read_text())
        center = json.loads((DEPLOY / 'center.example.json').read_text())
        node = center['nodes'][0]
        self.assertEqual(node['registration']['identity'], host['inbox']['node'])
        self.assertEqual(node['registration']['retention_slots'], host['inbox']['max_sessions'])
        self.assertEqual(node['address'], host['internal']['bind'])
        self.assertEqual(host['artifact_source']['address'], center['artifact_service']['bind'])
        self.assertEqual(node['registration']['healthy_until_ms'], 0)
        self.assertEqual(node['registration']['templates'], [])
        self.assertTrue(all(value == 0 for value in node['registration']['free'].values()))
        profile = host['native']['bounded_host']['physical_storage']
        self.assertEqual(profile['host_template_caches'], [host['template_cache']])
        owners = {Path(item['root']).name: item['owner_uid'] for item in profile['service_data']}
        self.assertEqual(owners, {'center': 1001, 'redis': 1002, 'control': 0})
        self.assertEqual(center['credentials'][0]['secret'], '')
        self.assertEqual(center['redis']['password'], '')

    def test_redis_requires_operator_acl_and_original_durable_fail_closed_flags(self):
        rows = [shlex.split(line) for line in (DEPLOY / 'redis.example.conf').read_text().splitlines()
                if line.strip() and not line.startswith('#')]
        settings = {row[0]: row[1:] for row in rows}
        self.assertEqual(settings['bind'], ['127.0.0.1'])
        self.assertEqual(settings['aclfile'], ['/var/lib/sandproof-services/redis/control/users.acl'])
        self.assertNotIn('requirepass', settings)
        self.assertEqual(settings['appendonly'], ['yes'])
        self.assertEqual(settings['appendfsync'], ['always'])
        self.assertEqual(settings['maxmemory-policy'], ['noeviction'])
        self.assertEqual(settings['aof-load-truncated'], ['no'])
        self.assertEqual(settings['no-appendfsync-on-rewrite'], ['no'])
        self.assertEqual(settings['auto-aof-rewrite-percentage'], ['100'])
        self.assertEqual(settings['logfile'], [''])

    def test_units_do_not_invent_resource_limits_or_ephemeral_authority_owners(self):
        for role in ('center', 'host', 'redis'):
            service = unit(role)['Service']
            for field in ('MemoryMax', 'CPUQuota', 'DynamicUser', 'StateDirectory', 'RuntimeDirectory'):
                self.assertNotIn(field, service)


if __name__ == '__main__':
    unittest.main()
