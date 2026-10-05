"""Trusted, opt-in transport for an isolated Harbor run, not a network policy.

The product's guest and host firewall still enforce authorization before packets
enter TUN. No global default route, DNS interception, task edits or VPN secrets.
Unknown VM cleanup retains routing and its fail-closed fallback for recovery.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import time


COORDINATION_DIRECTORY = Path('/run/sandproof-benchmark-proxy')


@contextmanager
def coordinator():
    """One root-managed boundary for installation, rollback and recovery."""
    if os.geteuid() != 0:
        raise PermissionError('trusted Linux root operator required')
    COORDINATION_DIRECTORY.mkdir(mode=0o700, exist_ok=True)
    trusted(COORDINATION_DIRECTORY, directory=True)
    path = COORDINATION_DIRECTORY / 'coordination.lock'
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        trusted(path)
        info = os.fstat(fd)
        if info.st_nlink != 1 or (info.st_dev, info.st_ino) != (path.lstat().st_dev, path.lstat().st_ino):
            raise ValueError('transport coordination lock replaced or aliased')
        deadline = time.monotonic() + 30
        while True:
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise RuntimeError('transport coordination is occupied')
                time.sleep(.1)
        trusted(path)
        current = path.lstat()
        if info.st_nlink != 1 or (info.st_dev, info.st_ino) != (current.st_dev, current.st_ino):
            raise ValueError('transport coordination lock changed while waiting')
        yield
    finally:
        os.close(fd)


def routing_precedes_unknown(rules, plan):
    """A target-specific earlier rule can otherwise escape even a dead TUN."""
    local = {'priority': 0, 'src': 'all', 'table': 'local'}
    for rule in rules:
        priority = rule.get('priority')
        if type(priority) is not int:
            raise ValueError('unrecognized transport routing rule priority')
        if priority < plan['priority']:
            normalized = {k: v for k, v in rule.items() if k != 'protocol'}
            if normalized.get('table') == 255:
                normalized['table'] = 'local'
            if normalized != local:
                raise ValueError('transport selector shadowed by earlier host routing')


def named_namespaces():
    try:
        return [x.name for x in Path('/run/netns').iterdir()]
    except FileNotFoundError:
        return []


def layout(namespaces):
    if (len(namespaces) != 2 or any(not re.fullmatch('[0-9a-f]{16}', n) for n in namespaces)
            or namespaces[0][:6] == namespaces[1][:6]):
        raise ValueError('two disjoint deployment namespaces required')
    tag = hashlib.sha256(':'.join(namespaces).encode()).hexdigest()[:8]
    return {'namespaces': list(namespaces), 'device': 'sbp' + tag,
            'table': 20000 + int(tag[:4], 16), 'priority': 10000 + int(tag[:4], 16) % 10000,
            'mark': int('53' + tag[:6], 16), 'nft_table': 'spbench_' + tag,
            'unit': 'sandproof-bench-proxy-' + tag}


def configuration(device, port):
    if not re.fullmatch('sbp[0-9a-f]{8}', device) or type(port) is not int or not 1 <= port <= 65535:
        raise ValueError('invalid local transport')
    return {'mode': 'rule', 'log-level': 'warning', 'ipv6': False, 'allow-lan': False,
            'bind-address': '127.0.0.1', 'mixed-port': 0,
            'dns': {'enable': False}, 'sniffer': {'enable': False},
            'tun': {'enable': True, 'device': device, 'stack': 'gvisor', 'mtu': 1500,
                    'auto-route': False, 'auto-redirect': False, 'auto-detect-interface': False,
                    'dns-hijack': []},
            'proxies': [{'name': 'host-proxy', 'type': 'socks5', 'server': '127.0.0.1',
                         'port': port, 'udp': True}], 'rules': ['MATCH,host-proxy']}


def mark_script(plan):
    selectors = ', '.join('"sp' + n[:6] + '*"' for n in plan['namespaces'])
    return (f"add table inet {plan['nft_table']}\n"
            f"add chain inet {plan['nft_table']} select {{ type filter hook prerouting priority -140; policy accept; }}\n"
            f"add rule inet {plan['nft_table']} select meta nfproto ipv4 iifname {{ {selectors} }} "
            f"meta mark set {plan['mark']} counter\n")


def guest_links(plan, links):
    # Product uplinks contain namespace prefix + a six-digit physical index.
    patterns = ['sp' + n[:6] + '[0-9a-f]{6}' for n in plan['namespaces']]
    return [x['ifname'] for x in links if any(re.fullmatch(p, x['ifname']) for p in patterns)]


def own_table(route, plan):
    # iproute2 serializes unnamed table IDs as strings on current Ubuntu.
    return str(route.get('table')) == str(plan['table'])


def selector_matches(body, plan):
    chains = [x['chain'] for x in body if 'chain' in x]
    rules = [x['rule'] for x in body if 'rule' in x]
    if len(chains) != 1 or len(rules) != 1:
        return False
    expected_chain = {'family': 'inet', 'table': plan['nft_table'], 'name': 'select',
                      'type': 'filter', 'hook': 'prerouting', 'prio': -140, 'policy': 'accept'}
    if {k: v for k, v in chains[0].items() if k != 'handle'} != expected_chain:
        return False
    expected = [
        {'match': {'op': '==', 'left': {'meta': {'key': 'nfproto'}}, 'right': 'ipv4'}},
        {'match': {'op': '==', 'left': {'meta': {'key': 'iifname'}},
                   'right': {'set': sorted('sp' + n[:6] + '*' for n in plan['namespaces'])}}},
        {'mangle': {'key': {'meta': {'key': 'mark'}}, 'value': plan['mark']}}]
    rule = rules[0]
    expr = rule.get('expr', [])
    return (rule.get('family') == 'inet' and rule.get('table') == plan['nft_table']
            and rule.get('chain') == 'select' and len(expr) == 4
            and expr[:3] == expected and set(expr[3]) == {'counter'}
            and all(set(x) <= {'metainfo', 'table', 'chain', 'rule'} for x in body))


def trusted(path, *, directory=False):
    info = path.lstat()
    kind = stat.S_ISDIR if directory else stat.S_ISREG
    if info.st_uid != 0 or info.st_mode & 0o077 or not kind(info.st_mode):
        raise ValueError('private root-owned ordinary path required')


def save(path, value):
    temporary = path.with_suffix('.new')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


class BenchmarkProxy:
    def __init__(self, root, namespaces, binary, digest, *, port=7897):
        self.root = Path(root)
        self.binary = Path(binary)
        self.digest = digest
        self.plan = layout(namespaces)
        self.config = configuration(self.plan['device'], port)
        self.record = self.root / 'transport.json'
        self.state = {'plan': self.plan, 'binary_sha256': digest, 'phase': 'new'}

    @staticmethod
    def run(arguments, *, data=None):
        result = subprocess.run(arguments, input=data, capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError('transport command failed: ' + Path(arguments[0]).name)
        return result.stdout

    def query(self, *arguments):
        return json.loads(self.run(['/usr/sbin/ip', '-j', *arguments]))

    def persist(self, phase):
        self.state['phase'] = phase
        save(self.record, self.state)

    def start(self):
        with coordinator():
            return self._start()

    def _start(self):
        if not self.root.is_absolute() or not str(self.binary).startswith('/'):
            raise ValueError('absolute private transport and binary paths required')
        trusted(self.root.parent, directory=True)
        binary_info = self.binary.lstat()
        if (not stat.S_ISREG(binary_info.st_mode) or binary_info.st_uid != 0 or binary_info.st_mode & 0o022
                or not re.fullmatch('[0-9a-f]{64}', self.digest)
                or hashlib.sha256(self.binary.read_bytes()).hexdigest() != self.digest):
            raise ValueError('approved root-owned transport binary required')
        p = self.plan
        rules = self.query('rule', 'show')
        routing_precedes_unknown(rules, p)
        if any(r.get('priority') == p['priority'] or str(r.get('fwmark', '')) in
               (str(p['mark']), hex(p['mark'])) for r in rules):
            raise ValueError('transport policy routing identity occupied')
        if any(own_table(r, p) for r in self.query('route', 'show', 'table', 'all')):
            raise ValueError('transport route table occupied')
        links = self.query('link', 'show')
        if any(x['ifname'] == p['device'] for x in links) or guest_links(p, links):
            raise ValueError('transport must precede its VMs')
        # v1.19.32 derives this address from its default fake-IP range even
        # with DNS disabled; inet4-address is NOT a supported raw config field.
        subnet = ipaddress.IPv4Network('198.18.0.0/30')
        addresses = self.query('address', 'show')
        if any(ipaddress.ip_address(a['local']) in subnet for x in addresses
               for a in x.get('addr_info', []) if a.get('family') == 'inet'):
            raise ValueError('transport private TUN address occupied')
        tables = json.loads(self.run(['/usr/sbin/nft', '-j', 'list', 'tables']))['nftables']
        if any(t.get('table', {}).get('name') == p['nft_table'] for t in tables):
            raise ValueError('transport nft identity occupied')
        load = self.run(['/usr/bin/systemctl', 'show', p['unit'] + '.service', '-p', 'LoadState', '--value']).strip()
        if load != b'not-found':
            raise ValueError('transport service identity occupied')
        self.root.mkdir(mode=0o700)
        save(self.root / 'config.json', self.config)
        # The recoverable intent includes the complete private source identity,
        # not only the globally visible transient unit name.
        self.state['unit_command'] = [str(self.binary), '-d', '/run/' + p['unit'],
            '-f', '/run/credentials/' + p['unit'] + '.service/config.json']
        self.state['unit_credential'] = 'config.json:' + str(self.root / 'config.json')
        self.state['config_sha256'] = hashlib.sha256((self.root / 'config.json').read_bytes()).hexdigest()
        if any(not re.fullmatch(r'[A-Za-z0-9_./:+-]+', x) for x in self.state['unit_command']):
            raise ValueError('unambiguous ordinary transport command paths required')
        self.persist('starting')
        try:
            self.run([str(self.binary), '-t', '-d', str(self.root), '-f', str(self.root / 'config.json')])
            # Each external effect has a durable intent flag for exact recovery.
            self.state['route_intent'] = True
            self.persist('starting')
            self.run(['/usr/sbin/ip', 'route', 'add', 'unreachable', 'default', 'metric', '32767', 'table', str(p['table'])])
            self.state['rule_intent'] = True
            self.persist('starting')
            self.run(['/usr/sbin/ip', 'rule', 'add', 'priority', str(p['priority']), 'fwmark', hex(p['mark']), 'lookup', str(p['table'])])
            self.state['nft_intent'] = True
            self.persist('starting')
            self.run(['/usr/sbin/nft', '-f', '-'], data=mark_script(p).encode())
            self.state['unit_intent'] = True
            self.persist('starting')
            self.run(['/usr/bin/systemd-run', '--quiet', '--collect', '--unit=' + p['unit'],
                '--property=DynamicUser=yes', '--property=RuntimeDirectory=' + p['unit'],
                '--property=RuntimeDirectoryMode=0700', '--property=LoadCredential=config.json:' + str(self.root / 'config.json'),
                '--property=RuntimeMaxSec=86400', '--property=MemoryMax=268435456', '--property=CPUQuota=100%',
                '--property=UMask=0077', '--property=NoNewPrivileges=yes', '--property=PrivateTmp=yes',
                '--property=ProtectSystem=strict', '--property=ProtectHome=yes',
                '--property=ProtectKernelTunables=yes', '--property=ProtectKernelModules=yes',
                '--property=ProtectControlGroups=yes', '--property=RestrictSUIDSGID=yes',
                '--property=CapabilityBoundingSet=CAP_NET_ADMIN', '--property=AmbientCapabilities=CAP_NET_ADMIN',
                '--property=DevicePolicy=closed', '--property=DeviceAllow=/dev/net/tun rw',
                '--property=RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK',
                *self.state['unit_command']])
            invocation = self.verify_unit()
            if invocation is None:
                raise RuntimeError('transport service disappeared during startup')
            self.state['unit_invocation'] = invocation
            self.persist('starting')
            deadline = time.monotonic() + 10
            while not any(x['ifname'] == p['device'] for x in self.query('link', 'show')):
                if time.monotonic() >= deadline:
                    raise RuntimeError('transport TUN startup failed')
                time.sleep(.1)
            self.state['tun_ifindex'] = next(x['ifindex'] for x in self.query('link', 'show')
                                            if x['ifname'] == p['device'])
            self.state['tun_route_intent'] = True
            self.persist('starting')
            self.run(['/usr/sbin/ip', 'route', 'add', 'default', 'dev', p['device'], 'metric', '10', 'table', str(p['table'])])
            routing_precedes_unknown(self.query('rule', 'show'), p)
            selected = self.query('route', 'get', '1.1.1.1', 'mark', hex(p['mark']))
            if len(selected) != 1 or selected[0].get('dev') != p['device']:
                raise RuntimeError('transport selector shadowed by earlier host routing')
            self.persist('active')
            return self.evidence()
        except BaseException:
            # The public wrapper already holds the same coordination lock.
            self._finish(quiescent=True)
            raise

    def verify_unit(self):
        """Read all service identity fields before any destructive operation."""
        properties = ('LoadState', 'ExecStart', 'InvocationID', 'Transient')
        raw = self.run(['/usr/bin/systemctl', 'show', self.plan['unit'] + '.service',
                        *['--property=' + x for x in properties]]).decode('utf-8')
        fields = {}
        for line in raw.splitlines():
            key, separator, value = line.partition('=')
            if not separator or key not in properties or key in fields:
                raise ValueError('unrecognized transport service observation')
            fields[key] = value
        if set(fields) != set(properties):
            raise ValueError('incomplete transport service observation')
        if fields['LoadState'] == 'not-found':
            return None
        if not self.state.get('unit_intent'):
            raise ValueError('transport service has no original intent')
        command = self.state.get('unit_command')
        if (not isinstance(command, list) or len(command) != 5
                or command[1:] != ['-d', '/run/' + self.plan['unit'], '-f',
                    '/run/credentials/' + self.plan['unit'] + '.service/config.json']
                or any(not isinstance(x, str) or not re.fullmatch(r'[A-Za-z0-9_./:+-]+', x) for x in command)
                or not command[0].startswith('/')):
            raise ValueError('invalid original transport command')
        credential = 'config.json:' + str(self.root / 'config.json')
        trusted(self.root / 'config.json')
        if (self.state.get('unit_credential') != credential
                or hashlib.sha256((self.root / 'config.json').read_bytes()).hexdigest() != self.state.get('config_sha256')):
            raise ValueError('transport credential source replaced')
        observed = re.fullmatch(r'\{ path=([^;\n]+) ; argv\[\]=([^;\n]+) ; ignore_errors=no ; [^{}\n]* \}',
                                fields['ExecStart'])
        # systemctl renders LoadCredential as [unprintable] on current Ubuntu.
        # D-Bus exposes only its source paths here, never credential data.
        unit_path = '/org/freedesktop/systemd1/unit/' + ''.join(
            c if c.isascii() and c.isalnum() else '_' + format(ord(c), '02x')
            for c in self.plan['unit'] + '.service')
        credentials = json.loads(self.run(['/usr/bin/busctl', '--json=short', 'get-property',
            'org.freedesktop.systemd1', unit_path, 'org.freedesktop.systemd1.Service', 'LoadCredential']))
        expected_credentials = [['config.json', str(self.root / 'config.json')]]
        if (fields['LoadState'] != 'loaded' or fields['Transient'] != 'yes'
                or credentials != {'type': 'a(ss)', 'data': expected_credentials} or observed is None
                or observed.group(1) != command[0] or observed.group(2) != ' '.join(command)):
            raise ValueError('transport service configuration replaced')
        invocation = fields['InvocationID']
        if (not re.fullmatch('[0-9a-f]{32}', invocation)
                or ('unit_invocation' in self.state and self.state['unit_invocation'] != invocation)):
            raise ValueError('transport service invocation replaced or unknown')
        return invocation

    def evidence(self):
        return {'mode': 'scoped_tun_to_loopback_socks', 'namespaces': self.plan['namespaces'],
                'binary_sha256': self.digest, 'device': self.plan['device'],
                'phase': self.state['phase'], 'dns_interception': False,
                'host_default_route_changed': False, 'proxy_credentials_copied': False}

    def finish(self, *, quiescent):
        with coordinator():
            return self._finish(quiescent=quiescent)

    def validate_record(self):
        trusted(self.root, directory=True)
        trusted(self.record)
        if json.loads(self.record.read_text()) != self.state:
            raise ValueError('transport recovery record changed')

    def stop_for_probe(self):
        """Fault injection stops only this unit, never its fail-closed routing."""
        with coordinator():
            self.validate_record()
            routing_precedes_unknown(self.query('rule', 'show'), self.plan)
            rules, routes, selector, unit = self.observe_transport()
            if (len(rules) != 1 or not selector
                    or not any(r.get('type') == 'unreachable' for r in routes)):
                raise ValueError('transport fail-closed fallback incomplete')
            self.persist('probe_stopping')
            if unit is not None:
                self.run(['/usr/bin/systemctl', 'stop', self.plan['unit'] + '.service'])
            rules, routes, selector, unit = self.observe_transport()
            if (unit is not None or len(rules) != 1 or not selector or len(routes) != 1
                    or routes[0].get('type') != 'unreachable'):
                raise RuntimeError('transport stop or preserved fallback unknown')
            self.persist('probe_stopped')
            return self.evidence()

    def _finish(self, *, quiescent):
        p = self.plan
        self.validate_record()
        live = guest_links(p, self.query('link', 'show'))
        linked = [name for name in named_namespaces() if any(name.startswith('vm-' + n + '-') for n in p['namespaces'])]
        if not quiescent or live or linked:
            self.persist('retained_for_original_vm_recovery')
            return self.evidence()
        # Never remove selected routing while original VM cleanup is unknown.
        # Exact nft/rule/route identities are checked before any deletion.
        rules, _, selector, unit = self.observe_transport()
        # All observations are checked before the first deletion.
        if selector:
            self.run(['/usr/sbin/nft', 'delete', 'table', 'inet', p['nft_table']])
        if rules:
            self.run(['/usr/sbin/ip', 'rule', 'del', 'priority', str(p['priority']), 'fwmark', hex(p['mark']), 'lookup', str(p['table'])])
        if unit is not None:
            self.run(['/usr/bin/systemctl', 'stop', p['unit'] + '.service'])
        for route in self.query('route', 'show', 'table', 'all'):
            if own_table(route, p):
                if route.get('type') != 'unreachable':
                    raise ValueError('transport route survived service stop')
                self.run(['/usr/sbin/ip', 'route', 'del', 'unreachable', 'default', 'metric', '32767', 'table', str(p['table'])])
        if any(x['ifname'] == p['device'] for x in self.query('link', 'show')):
            raise RuntimeError('transport device survived stop')
        self.persist('removed')
        return self.evidence()

    def observe_transport(self):
        """Validate the complete original resource set without changing it."""
        p = self.plan
        rules = [r for r in self.query('rule', 'show') if r.get('priority') == p['priority']]
        if rules and (not self.state.get('rule_intent') or len(rules) != 1
                      or str(rules[0].get('fwmark')) not in (str(p['mark']), hex(p['mark']))
                      or not own_table(rules[0], p) or rules[0].get('src', 'all') != 'all'
                      or set(rules[0]) - {'priority', 'src', 'fwmark', 'fwmask', 'table', 'protocol'}
                      or str(rules[0].get('fwmask', '0xffffffff')) not in ('0xffffffff', '4294967295')):
            raise ValueError('transport routing rule replaced')
        routes = self.query('route', 'show', 'table', 'all')
        owned = [r for r in routes if own_table(r, p)]
        if any(not self.state.get('route_intent') or r.get('dst') != 'default' or r.get('gateway')
               or (r.get('type') == 'unreachable' and r.get('metric') != 32767)
               or (r.get('type') != 'unreachable' and
                   (not self.state.get('tun_route_intent') or r.get('type', 'unicast') != 'unicast'
                    or r.get('dev') != p['device'] or r.get('metric') != 10))
               for r in owned):
            raise ValueError('transport routing table replaced')
        listed = json.loads(self.run(['/usr/sbin/nft', '-j', 'list', 'tables']))['nftables']
        selector = any(t.get('table', {}).get('name') == p['nft_table'] for t in listed)
        if selector:
            body = json.loads(self.run(['/usr/sbin/nft', '-j', 'list', 'table', 'inet', p['nft_table']]))['nftables']
            if not self.state.get('nft_intent') or not selector_matches(body, p):
                raise ValueError('transport selector replaced or without original intent')
        unit = self.verify_unit()
        tun = [x for x in self.query('link', 'show') if x['ifname'] == p['device']]
        if tun and (unit is None or len(tun) != 1
                    or ('tun_ifindex' in self.state and tun[0].get('ifindex') != self.state['tun_ifindex'])):
            raise ValueError('transport device replaced or without original service')
        return rules, owned, selector, unit


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cleanup', type=Path, required=True, help='private per-run transport.json after independent VM audit')
    parser.add_argument('--quiescent', action='store_true', required=True)
    args = parser.parse_args()
    trusted(args.cleanup.parent, directory=True)
    trusted(args.cleanup)
    state = json.loads(args.cleanup.read_text())
    plan = layout(state['plan']['namespaces'])
    if state['plan'] != plan:
        raise ValueError('invalid original transport identity')
    obj = BenchmarkProxy(args.cleanup.parent, plan['namespaces'], '/unused', state['binary_sha256'])
    obj.state = state
    print(json.dumps(obj.finish(quiescent=True)))
