import copy
import fcntl
import hashlib
import json
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools.benchmark_proxy import (BenchmarkProxy, configuration, coordinator, guest_links, layout,
                                   mark_script, named_namespaces, routing_precedes_unknown, save, selector_matches)
from tools import benchmark_proxy


NS = ['0123456789abcdef', 'fedcba9876543210']


def nft_body(p):
    return [{'chain': {'family': 'inet', 'table': p['nft_table'], 'name': 'select',
                      'handle': 1, 'type': 'filter', 'hook': 'prerouting', 'prio': -140, 'policy': 'accept'}},
            {'rule': {'family': 'inet', 'table': p['nft_table'], 'chain': 'select', 'handle': 3,
                      'expr': [
                          {'match': {'op': '==', 'left': {'meta': {'key': 'nfproto'}}, 'right': 'ipv4'}},
                          {'match': {'op': '==', 'left': {'meta': {'key': 'iifname'}},
                                     'right': {'set': sorted('sp' + n[:6] + '*' for n in p['namespaces'])}}},
                          {'mangle': {'key': {'meta': {'key': 'mark'}}, 'value': p['mark']}},
                          {'counter': {'packets': 42, 'bytes': 2048}}]}}]


class SimulatedProxy(BenchmarkProxy):
    """Command-boundary fault exploration only; never counted as Linux/KVM."""
    def __init__(self, root):
        super().__init__(root, NS, '/approved/mihomo', hashlib.sha256(b'binary').hexdigest())
        self.binary = Mock()
        self.binary.__str__ = Mock(return_value='/approved/mihomo')
        self.binary.lstat.return_value = SimpleNamespace(st_mode=0o100755, st_uid=0)
        self.binary.read_bytes.return_value = b'binary'
        self.commands, self.rules, self.routes, self.links = [], [], [], []
        self.nft, self.unit, self.fail_at, self.fail_once = False, False, None, True
        self.invocation = '0123456789abcdef0123456789abcdef'
        self.unit_overrides = {}
        self.credential_source = None

    def query(self, *args):
        if args[:2] == ('route', 'get'):
            return [{'dev': self.plan['device']}]
        return {'rule': self.rules, 'route': self.routes, 'link': self.links, 'address': []}[args[0]]

    def run(self, args, *, data=None):
        self.commands.append((args, data))
        if self.fail_at and self.fail_once and self.fail_at in ' '.join(args):
            self.fail_once = False
            raise RuntimeError('injected command failure')
        p = self.plan
        if args[0].endswith('nft'):
            if 'tables' in args:
                return json.dumps({'nftables': [{'table': {'name': p['nft_table']}}] if self.nft else []}).encode()
            if 'table' in args and 'list' in args:
                return json.dumps({'nftables': nft_body(p)}).encode()
            if 'delete' in args:
                self.nft = False
            elif '-f' in args:
                self.nft = True
        elif 'route' in args:
            if 'add' in args:
                self.routes.append({'table': str(p['table']), 'dst': 'default',
                                    **({'type': 'unreachable', 'metric': 32767} if 'unreachable' in args
                                       else {'dev': p['device'], 'metric': 10})})
            elif 'del' in args:
                self.routes.clear()
        elif 'rule' in args:
            if 'add' in args:
                self.rules.append({'priority': p['priority'], 'table': str(p['table']), 'fwmark': hex(p['mark'])})
            elif 'del' in args:
                self.rules.clear()
        elif args[0].endswith('systemd-run'):
            self.unit = True
            self.links.append({'ifname': p['device'], 'ifindex': 42})
        elif args[0].endswith('busctl'):
            source = self.credential_source or self.root / 'config.json'
            return json.dumps({'type': 'a(ss)', 'data': [['config.json', str(source)]]}).encode()
        elif '--property=LoadState' in args:
            command = self.state.get('unit_command', [])
            fields = {'LoadState': 'loaded' if self.unit else 'not-found',
                      'ExecStart': ('{ path=' + command[0] + ' ; argv[]=' + ' '.join(command) +
                                    ' ; ignore_errors=no ; start_time=[n/a] ; stop_time=[n/a] ; pid=123 ; code=(null) ; status=0/0 }')
                                   if self.unit and command else '',
                      'InvocationID': self.invocation if self.unit else '',
                      'Transient': 'yes' if self.unit else 'no'}
            fields.update(self.unit_overrides)
            # Actual systemctl show (without --all) omits empty properties.
            return ''.join(k + '=' + v + '\n' for k, v in fields.items() if v).encode()
        elif 'LoadState' in args:
            return b'loaded\n' if self.unit else b'not-found\n'
        elif 'stop' in args:
            self.unit = False
            self.links = [x for x in self.links if x['ifname'] != p['device']]
            self.routes = [r for r in self.routes if r.get('type') == 'unreachable']
        return b''


class BenchmarkProxyTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        # These are command-boundary component tests, never root host effects.
        for replacement in (patch('tools.benchmark_proxy.COORDINATION_DIRECTORY', Path(directory.name) / 'locks'),
                            patch('tools.benchmark_proxy.trusted'),
                            patch('tools.benchmark_proxy.os.geteuid', return_value=0)):
            replacement.start()
            self.addCleanup(replacement.stop)

    def test_namespaces_are_exact_and_cannot_capture_another_prefix(self):
        for value in [[], NS[:1], [NS[0], NS[0]], [NS[0], '012345aaaaaaaaaa'],
                      [NS[0], '*'], [NS[0], 'x' * 16], [NS[0], NS[1].upper()]]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                layout(value)
        p = layout(NS)
        script = mark_script(p)
        self.assertIn('"sp012345*", "spfedcba*"', script)
        self.assertNotIn('output', script)
        self.assertNotIn('redirect', script)
        self.assertNotIn('accept\n', script)

    def test_transport_does_not_change_policy_dns_or_host_default_routes(self):
        c = configuration(layout(NS)['device'], 7897)
        for field in ('auto-route', 'auto-redirect', 'auto-detect-interface'):
            self.assertFalse(c['tun'][field])
        self.assertEqual(c['tun']['dns-hijack'], [])
        self.assertFalse(c['dns']['enable'])
        self.assertFalse(c['sniffer']['enable'])
        self.assertEqual(c['mixed-port'], 0)
        self.assertFalse(c['ipv6'])
        self.assertEqual(c['proxies'], [{'name': 'host-proxy', 'type': 'socks5',
            'server': '127.0.0.1', 'port': 7897, 'udp': True}])
        self.assertNotIn('password', json.dumps(c))

    def test_local_transport_rejects_invalid_config_before_any_effect(self):
        for port in (True, 0, 65536, '7897'):
            with self.subTest(port=port), self.assertRaises(ValueError):
                configuration(layout(NS)['device'], port)
        with self.assertRaises(ValueError):
            configuration('host-interface', 7897)

    def test_absent_systemd_unit_accepts_omitted_empty_identity_fields_only(self):
        obj = BenchmarkProxy(Path('/private/proxy'), NS, '/approved/mihomo', 'a' * 64)
        for raw in (b'LoadState=not-found\nTransient=no\n', b'LoadState=not-found\n',
                    b'ExecStart=\nInvocationID=\nLoadState=not-found\nTransient=no\n'):
            with self.subTest(raw=raw), patch.object(obj, 'run', return_value=raw):
                self.assertIsNone(obj.verify_unit())
        for raw in (b'LoadState=loaded\nTransient=yes\n', b'LoadState=not-found\nTransient=yes\n',
                    b'LoadState=not-found\nInvocationID=' + b'a' * 32 + b'\n',
                    b'LoadState=not-found\nExecStart=/foreign\n',
                    b'LoadState=not-found\nUnexpected=yes\n', b'Transient=no\n',
                    b'LoadState=not-found\nLoadState=not-found\n'):
            with self.subTest(raw=raw), patch.object(obj, 'run', return_value=raw), self.assertRaises(ValueError):
                obj.verify_unit()

    def test_live_link_detection_is_exact_and_not_a_global_scan_target(self):
        p = layout(NS)
        own = 'sp012345000001'
        self.assertEqual(guest_links(p, [{'ifname': own}, {'ifname': 'sp000000000001'},
                                       {'ifname': 'sp012345unknown'}, {'ifname': p['device']}]), [own])

    def test_policy_rule_precedes_the_normal_main_table_for_every_tag(self):
        for value in range(1000):
            p = layout([f'{value:016x}', 'fedcba9876543210'])
            self.assertGreater(p['priority'], 0)
            self.assertLess(p['priority'], 32766)

    def test_target_specific_earlier_routes_are_rejected_not_just_probe_destination(self):
        p = layout(NS)
        baseline = [{'priority': 0, 'src': 'all', 'table': 'local'},
                    {'priority': 32766, 'src': 'all', 'table': 'main'}]
        routing_precedes_unknown(baseline, p)
        for additional in ({'priority': 5000, 'src': 'all', 'dst': '140.82.0.0/16', 'table': 'main'},
                           {'priority': 5000, 'src': 'all', 'iif': 'sp012345000001', 'table': 'main'},
                           {'priority': 0, 'src': 'all', 'table': 'main'},
                           {'src': 'all', 'table': 'local'}):
            with self.subTest(rule=additional), self.assertRaises(ValueError):
                routing_precedes_unknown(baseline + [additional], p)

    def test_missing_netns_is_empty_but_unknown_observation_is_not(self):
        with patch('tools.benchmark_proxy.Path.iterdir', side_effect=FileNotFoundError):
            self.assertEqual(named_namespaces(), [])
        with patch('tools.benchmark_proxy.Path.iterdir', side_effect=PermissionError), self.assertRaises(PermissionError):
            named_namespaces()

    def test_coordinator_uses_one_shared_lock_for_distinct_run_roots(self):
        first_entered, second_blocked, release_first, second_entered = [threading.Event() for _ in range(4)]
        failures = []
        actual_flock = fcntl.flock
        def observed_flock(fd, operation):
            try:
                return actual_flock(fd, operation)
            except BlockingIOError:
                second_blocked.set()
                raise
        def first():
            try:
                with coordinator():
                    first_entered.set()
                    if not release_first.wait(5):
                        raise RuntimeError('test barrier timed out')
            except BaseException as error:
                failures.append(error)
        def second():
            try:
                with coordinator():
                    second_entered.set()
            except BaseException as error:
                failures.append(error)
        a, b = threading.Thread(target=first), threading.Thread(target=second)
        with patch('tools.benchmark_proxy.fcntl.flock', side_effect=observed_flock):
            a.start()
            try:
                self.assertTrue(first_entered.wait(5))
                b.start()
                self.assertTrue(second_blocked.wait(5))
                self.assertFalse(second_entered.is_set())
            finally:
                release_first.set()
                a.join(5)
                if b.ident is not None:
                    b.join(5)
        self.assertFalse(a.is_alive() or b.is_alive())
        self.assertEqual(failures, [])
        self.assertTrue(second_entered.is_set())

    def test_coordinator_rejects_aliased_lock(self):
        root = benchmark_proxy.COORDINATION_DIRECTORY
        root.mkdir(mode=0o700)
        original = root / 'original'
        original.touch(mode=0o600)
        (root / 'coordination.lock').hardlink_to(original)
        with self.assertRaisesRegex(ValueError, 'aliased'):
            with coordinator():
                self.fail('aliased lock admitted')

    def test_recovery_rejects_replaced_selector_not_just_same_mark(self):
        p = layout(NS)
        self.assertTrue(selector_matches(nft_body(p), p))
        for field in ('source', 'hook', 'action', 'extra'):
            body = copy.deepcopy(nft_body(p))
            if field == 'source':
                body[1]['rule']['expr'][1]['match']['right']['set'] = ['*']
            elif field == 'hook':
                body[0]['chain']['hook'] = 'output'
            elif field == 'action':
                body[1]['rule']['expr'][2] = {'accept': None}
            else:
                body.append({'set': {'name': 'foreign'}})
            with self.subTest(field=field):
                self.assertFalse(selector_matches(body, p))

    def exercise(self, *, failure=None, retained=False, live=False):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.trusted'), \
                patch('tools.benchmark_proxy.os.geteuid', return_value=0), \
                patch('tools.benchmark_proxy.Path.iterdir', return_value=iter(())):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.fail_at = failure
            if failure:
                with self.assertRaises(RuntimeError):
                    obj.start()
            else:
                obj.start()
                if live:
                    obj.links.append({'ifname': 'sp012345000001'})
                obj.finish(quiescent=not retained)
            return obj

    def test_success_cleanup_does_not_remove_unrelated_routes(self):
        obj = self.exercise()
        self.assertEqual(obj.state['phase'], 'removed')
        self.assertFalse(obj.nft or obj.unit or obj.rules or obj.routes or obj.links)
        commands = [c for c, _ in obj.commands]
        self.assertFalse(any('flush' in c for c in commands))
        unreachable = next(i for i, c in enumerate(commands) if 'route' in c and 'add' in c and 'unreachable' in c)
        selector = next(i for i, c in enumerate(commands) if c[0].endswith('nft') and '-f' in c)
        active = next(i for i, c in enumerate(commands) if 'route' in c and 'add' in c and 'dev' in c)
        self.assertLess(unreachable, selector)
        self.assertLess(selector, active)

    def test_each_install_failure_cleans_only_its_acknowledged_resources(self):
        for stage in (' -t ', 'route add unreachable', 'rule add', 'nft -f',
                      'systemd-run', 'route add default'):
            with self.subTest(stage=stage):
                obj = self.exercise(failure=stage.strip())
                self.assertEqual(obj.state['phase'], 'removed')
                self.assertFalse(obj.nft or obj.unit or obj.rules or obj.routes or obj.links)

    def test_unknown_vm_cleanup_retains_fail_closed_path(self):
        obj = self.exercise(retained=True)
        self.assertEqual(obj.state['phase'], 'retained_for_original_vm_recovery')
        self.assertTrue(obj.unit and obj.nft and obj.rules)
        self.assertTrue(any(r.get('type') == 'unreachable' for r in obj.routes))
        self.assertFalse(any('del' in c or 'delete' in c or 'stop' in c for c, _ in obj.commands))

    def test_live_vm_blocks_even_an_operator_quiescent_claim(self):
        obj = self.exercise(live=True)
        self.assertEqual(obj.state['phase'], 'retained_for_original_vm_recovery')
        self.assertTrue(obj.unit and obj.nft and obj.rules)

    def test_shadowed_route_is_rejected_and_rolled_back_before_any_vm(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.trusted'), \
                patch('tools.benchmark_proxy.os.geteuid', return_value=0), \
                patch('tools.benchmark_proxy.Path.iterdir', return_value=iter(())):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            query = obj.query
            obj.query = lambda *args: ([{'dev': 'foreign-uplink'}] if args[:2] == ('route', 'get') else query(*args))
            with self.assertRaisesRegex(RuntimeError, 'shadowed'):
                obj.start()
            self.assertEqual(obj.state['phase'], 'removed')
            self.assertFalse(obj.nft or obj.unit or obj.rules or obj.routes or obj.links)

    def test_lost_route_ack_is_recovered_from_durable_intent(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.trusted'), \
                patch('tools.benchmark_proxy.os.geteuid', return_value=0), \
                patch('tools.benchmark_proxy.Path.iterdir', return_value=iter(())):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            run = obj.run
            def lost_ack(args, *, data=None):
                result = run(args, data=data)
                if 'route' in args and 'add' in args and 'unreachable' in args:
                    raise RuntimeError('route installed but acknowledgment lost')
                return result
            obj.run = lost_ack
            with self.assertRaisesRegex(RuntimeError, 'acknowledgment lost'):
                obj.start()
            self.assertEqual(obj.state['phase'], 'removed')
            self.assertFalse(obj.routes)

    def test_lost_service_ack_uses_strict_original_configuration_and_intent(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            original = obj.run
            def lost_ack(args, *, data=None):
                result = original(args, data=data)
                if args[0].endswith('systemd-run'):
                    raise RuntimeError('service installed but acknowledgment lost')
                return result
            obj.run = lost_ack
            with self.assertRaisesRegex(RuntimeError, 'acknowledgment lost'):
                obj.start()
            self.assertEqual(obj.state['phase'], 'removed')
            self.assertFalse(obj.unit or obj.routes or obj.rules or obj.nft)

    def test_replaced_unit_or_credentials_are_rejected_before_first_deletion(self):
        for replacement in ('command', 'extra_command', 'credential', 'invocation', 'device', 'transient'):
            with self.subTest(replacement=replacement), tempfile.TemporaryDirectory() as directory, \
                    patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
                obj = SimulatedProxy(Path(directory) / 'proxy')
                obj.start()
                if replacement == 'command':
                    obj.unit_overrides['ExecStart'] = '{ path=/foreign ; argv[]=/foreign ; ignore_errors=no ; pid=1 }'
                elif replacement == 'extra_command':
                    obj.unit_overrides['ExecStart'] = ('{ path=' + obj.state['unit_command'][0] +
                        ' ; argv[]=' + ' '.join(obj.state['unit_command']) +
                        ' ; ignore_errors=no ; pid=1 } { path=/foreign ; argv[]=/foreign ; ignore_errors=no ; pid=2 }')
                elif replacement == 'credential':
                    obj.credential_source = Path('/foreign/config.json')
                elif replacement == 'invocation':
                    obj.invocation = 'f' * 32
                elif replacement == 'device':
                    obj.links[0]['ifindex'] += 1
                else:
                    obj.unit_overrides['Transient'] = 'no'
                boundary = len(obj.commands)
                with self.assertRaises(ValueError):
                    obj.finish(quiescent=True)
                self.assertFalse(any('del' in c or 'delete' in c or 'stop' in c for c, _ in obj.commands[boundary:]))
                self.assertTrue(obj.unit and obj.nft and obj.rules and obj.routes)

    def test_unintended_matching_resources_are_never_adopted_for_deletion(self):
        for resource, intent in (('rules', 'rule_intent'), ('routes', 'route_intent'),
                                 ('nft', 'nft_intent'), ('unit', 'unit_intent')):
            with self.subTest(resource=resource), tempfile.TemporaryDirectory() as directory, \
                    patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
                obj = SimulatedProxy(Path(directory) / 'proxy')
                obj.start()
                del obj.state[intent]
                obj.persist('active')
                boundary = len(obj.commands)
                with self.assertRaises(ValueError):
                    obj.finish(quiescent=True)
                self.assertFalse(any('del' in c or 'delete' in c or 'stop' in c for c, _ in obj.commands[boundary:]))

    def test_startup_failure_with_no_netns_cleans_and_does_not_reenter_lock(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch('tools.benchmark_proxy.Path.iterdir', side_effect=FileNotFoundError):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.fail_at = 'systemd-run'
            with self.assertRaisesRegex(RuntimeError, 'injected command failure'):
                obj.start()
            self.assertEqual(obj.state['phase'], 'removed')
            self.assertFalse(obj.rules or obj.routes or obj.nft or obj.unit)

    def test_recovery_uses_persisted_command_instead_of_cli_placeholder(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            obj.binary = Path('/unused')
            obj.state = json.loads(obj.record.read_text())
            obj.finish(quiescent=True)
            self.assertEqual(obj.state['phase'], 'removed')

    def test_missing_netns_cleanup_preserves_unknown_observation(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            boundary = len(obj.commands)
            with patch('tools.benchmark_proxy.named_namespaces', side_effect=PermissionError), self.assertRaises(PermissionError):
                obj.finish(quiescent=True)
            self.assertFalse(any('del' in c or 'delete' in c or 'stop' in c for c, _ in obj.commands[boundary:]))

    def test_stale_recovery_record_cannot_be_overwritten_by_retention(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            newer = dict(obj.state, phase='retained_for_independent_review')
            save(obj.record, newer)
            with self.assertRaisesRegex(ValueError, 'record changed'):
                obj.finish(quiescent=False)
            self.assertEqual(json.loads(obj.record.read_text()), newer)

    def test_probe_stop_retains_selector_and_unreachable_then_finishes(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            boundary = len(obj.commands)
            obj.stop_for_probe()
            self.assertEqual(obj.state['phase'], 'probe_stopped')
            self.assertFalse(obj.unit or obj.links)
            self.assertTrue(obj.nft and obj.rules)
            self.assertEqual(len(obj.routes), 1)
            self.assertEqual(obj.routes[0]['type'], 'unreachable')
            self.assertFalse(any('del' in c or 'delete' in c for c, _ in obj.commands[boundary:]))
            obj.finish(quiescent=True)
            self.assertEqual(obj.state['phase'], 'removed')
            self.assertFalse(obj.nft or obj.rules or obj.routes)

    def test_probe_stop_rejects_replacement_or_missing_fallback_before_stopping(self):
        for replacement in ('invocation', 'fallback', 'rule', 'selector'):
            with self.subTest(replacement=replacement), tempfile.TemporaryDirectory() as directory, \
                    patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
                obj = SimulatedProxy(Path(directory) / 'proxy')
                obj.start()
                if replacement == 'invocation':
                    obj.invocation = 'f' * 32
                elif replacement == 'fallback':
                    obj.routes = [r for r in obj.routes if r.get('type') != 'unreachable']
                elif replacement == 'rule':
                    obj.rules = []
                else:
                    obj.nft = False
                boundary = len(obj.commands)
                with self.assertRaises(ValueError):
                    obj.stop_for_probe()
                self.assertFalse(any('stop' in c or 'del' in c or 'delete' in c for c, _ in obj.commands[boundary:]))
                self.assertTrue(obj.unit)

    def test_probe_stop_lost_receipt_keeps_fallback_and_original_recovery_identity(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            original = obj.run
            def lost_ack(args, *, data=None):
                result = original(args, data=data)
                if 'stop' in args:
                    raise RuntimeError('stop completed but acknowledgment lost')
                return result
            obj.run = lost_ack
            with self.assertRaisesRegex(RuntimeError, 'acknowledgment lost'):
                obj.stop_for_probe()
            self.assertEqual(obj.state['phase'], 'probe_stopping')
            self.assertTrue(obj.nft and obj.rules and obj.routes)
            # The persisted stop intent can finish after its receipt is lost.
            obj.stop_for_probe()
            self.assertEqual(obj.state['phase'], 'probe_stopped')
            obj.finish(quiescent=True)
            self.assertEqual(obj.state['phase'], 'removed')

    def test_probe_stop_rejects_unexpected_exit_without_prior_stop_intent(self):
        with tempfile.TemporaryDirectory() as directory, patch('tools.benchmark_proxy.named_namespaces', return_value=[]):
            obj = SimulatedProxy(Path(directory) / 'proxy')
            obj.start()
            # Observe a prior crash/OOM, not the requested controlled failure.
            obj.unit = False
            obj.links = []
            obj.routes = [r for r in obj.routes if r.get('type') == 'unreachable']
            boundary = len(obj.commands)
            with self.assertRaisesRegex(ValueError, 'exited before controlled'):
                obj.stop_for_probe()
            self.assertEqual(obj.state['phase'], 'active')
            self.assertFalse(any('stop' in c or 'del' in c or 'delete' in c for c, _ in obj.commands[boundary:]))
            self.assertTrue(obj.nft and obj.rules and obj.routes)


if __name__ == '__main__':
    unittest.main()
