import copy
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from tools.benchmark_proxy import BenchmarkProxy, configuration, guest_links, layout, mark_script, selector_matches


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
            self.links.append({'ifname': p['device']})
        elif 'LoadState' in args:
            return b'loaded\n' if self.unit else b'not-found\n'
        elif 'stop' in args:
            self.unit = False
            self.links = [x for x in self.links if x['ifname'] != p['device']]
            self.routes = [r for r in self.routes if r.get('type') == 'unreachable']
        return b''


class BenchmarkProxyTests(unittest.TestCase):
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


if __name__ == '__main__':
    unittest.main()
