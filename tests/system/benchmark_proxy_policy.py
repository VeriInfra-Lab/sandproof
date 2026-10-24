"""Opt-in proxy ACL acceptance on fresh, operator-approved Linux/KVM fixtures.

Not an original benchmark run. A static test helper is uploaded only to these
private guests; no Python/curl assumption or task/image modification. Unknown
creation or cleanup retains the controller, transport and private evidence.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)['require_assertions']
_require_assertions()

import argparse
from io import BytesIO
import importlib.metadata
import ipaddress
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from tools.benchmark_proxy import BenchmarkProxy, save, trusted
from tools import service_cgroup
from tools.runtime_create_recovery import recover_original_create, find_runtime_create


def probe_result(result):
    if result.exit_code != 0 or result.stderr.strip() or result.stdout.strip() not in ('allowed', 'denied'):
        raise RuntimeError('network probe execution failed, not a policy denial')
    return result.stdout.strip() == 'allowed'


def exercise(args):
    from e2b import Sandbox
    from sdk_c02_acceptance import processes_in_uid_range, sha256_file
    from server_process_crash import fetch, free_loopback_port
    from _complete_runtime import complete_runtime
    from _runtime_services import RuntimeServices
    from _center_host_services import local_binding
    from server_template_isolation import audit_gone
    from server_templates import descriptor, MIB, record_for
    if sys.platform != 'linux' or os.geteuid() != 0:
        raise RuntimeError('trusted Linux root operator required')
    if importlib.metadata.version('e2b') != '2.51.0':
        raise RuntimeError('fixed SDK required')
    checkout = Path(__file__).resolve().parents[2]
    if subprocess.check_output(['git', '-C', str(checkout), 'status', '--porcelain']).strip():
        raise RuntimeError('clean exact source commit required')
    commit = subprocess.check_output(['git', '-C', str(checkout), 'rev-parse', 'HEAD'], text=True).strip()
    parent = args.scratch_parent.resolve(strict=True)
    trusted(parent, directory=True)
    for name in ('center', 'host', 'kernel', 'rootfs', 'firecracker', 'jailer', 'probe', 'proxy_binary'):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    inputs = {name: {'sha256': sha256_file(getattr(args, name)), 'bytes': getattr(args, name).stat().st_size}
              for name in ('center', 'host', 'kernel', 'rootfs', 'firecracker', 'jailer', 'probe', 'proxy_binary')}
    if inputs['probe']['sha256'] != args.probe_sha256:
        raise ValueError('approved static probe digest required')
    if not 1000 <= args.uid_base < 2**32 - 1 or processes_in_uid_range(args.uid_base, args.uid_base):
        raise RuntimeError('exclusive UID range required')
    available = int(next(x.split()[1] for x in Path('/proc/meminfo').read_text().splitlines() if x.startswith('MemAvailable:'))) * 1024
    if available < (args.memory_mib + 256 + 1024 + 256 + 2048) * MIB or len(os.sched_getaffinity(0)) < 4:
        raise RuntimeError('insufficient VM/service/proxy and host headroom')
    if shutil.disk_usage(parent).free < 3 * inputs['rootfs']['bytes'] + 1024 * MIB:
        raise RuntimeError('insufficient private image and cleanup space')
    args.cgroup_parent = args.cgroup_parent.resolve(strict=True)
    args.service_cgroup_parent = args.service_cgroup_parent.resolve(strict=True)
    for group in (args.cgroup_parent, args.service_cgroup_parent):
        service_cgroup.filesystem(group)
        if any(p.is_dir() for p in group.iterdir()):
            raise RuntimeError('exclusive empty delegated parent required')
    if args.cgroup_parent.is_relative_to(args.service_cgroup_parent) or args.service_cgroup_parent.is_relative_to(args.cgroup_parent):
        raise ValueError('disjoint VM and service parents required')
    if Path('/proc/sys/net/ipv4/ip_forward').read_text().strip() != '1':
        raise RuntimeError('operator-approved temporary forwarding required')
    host = str(ipaddress.IPv4Address(args.host_address))
    before = subprocess.check_output(['/usr/sbin/ip', '-j', 'route', 'show', 'default'])
    root = Path(tempfile.mkdtemp(prefix='pa-', dir=parent))
    print(json.dumps({'root': str(root), 'commit': commit}), flush=True)
    namespaces = [secrets.token_hex(8), secrets.token_hex(8)]
    while namespaces[0][:6] == namespaces[1][:6]:
        namespaces[1] = secrets.token_hex(8)
    for name in ('db', 'state', 'jails', 'images'):
        (root / name).mkdir(mode=0o700)
    for source, target in ((args.kernel, root / 'images/kernel'), (args.rootfs, root / 'images/rootfs')):
        subprocess.run(['/usr/bin/cp', '--reflink=auto', '--sparse=always', str(source), str(target)], check=True)
        target.chmod(0o444)
        if sha256_file(target) != inputs['kernel' if source == args.kernel else 'rootfs']['sha256']:
            raise RuntimeError('private fixture copy mismatch')
    template = descriptor('proxy-policy', root / 'images/kernel', root / 'images/rootfs', 1, args.memory_mib)
    port, key = free_loopback_port(), secrets.token_hex(32)
    cfg = {'namespace': namespaces[0], 'database': str(root / 'db/controller.db'),
        'node_state_dir': str(root / 'state'), 'chroot_base': str(root / 'jails'),
        'jailer': str(args.jailer), 'firecracker': str(args.firecracker),
        'kernel': template['kernel']['path'], 'rootfs': template['rootfs']['path'],
        'uid_base': args.uid_base, 'vcpu_count': 1, 'memory_mib': args.memory_mib,
        'host_resources': {'cgroup_parent': str(args.cgroup_parent), 'memory_max_bytes': (args.memory_mib + 256) * MIB},
        'file_policy_digest': 1, 'public_ipv4_enabled': True,
        'max_active_global': 1, 'max_active_per_tenant': 1,
        'max_disk_bytes_global': 2 * inputs['rootfs']['bytes'], 'max_disk_bytes_per_tenant': 2 * inputs['rootfs']['bytes'],
        'cleanup_reserve_bytes': inputs['rootfs']['bytes'], 'max_timeout_secs': 600,
        'default_timeout_secs': 300, 'max_lifetime_secs': 600,
        'startup_timeout_secs': 300, 'expiry_scan_seconds': 1,
        'templates': {'templates': [template], 'aliases': {'base': 'proxy-policy'},
            'compute_limits': {'max_vcpus_global': 1, 'max_vcpus_per_tenant': 1,
                'max_memory_bytes_global': (args.memory_mib + 256) * MIB,
                'max_memory_bytes_per_tenant': (args.memory_mib + 256) * MIB}}}
    cfg = complete_runtime(cfg, slots=1)
    url = f'http://127.0.0.1:{port}'
    options = dict(api_url=url, sandbox_url=url, api_key=key, domain='example.invalid', retries=0, request_timeout=90)
    proxy = BenchmarkProxy(root / 'proxy', namespaces, args.proxy_binary, args.proxy_sha256, port=args.proxy_socks_port)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    failure, quiescent = None, False
    intents, rows, cleanup_errors, dispatched = {}, [], [], set()
    known, identities = {}, {}
    service_record = args.service_cgroup_record
    registry = dict(root=str(root/'unused-registry'),max_templates_global=1,
        max_templates_per_tenant=1,max_artifact_bytes_global=inputs['rootfs']['bytes']*2,
        max_artifact_bytes_per_tenant=inputs['rootfs']['bytes']*2,
        max_template_disk_bytes=inputs['rootfs']['bytes'],validation_timeout_secs=120)
    services = RuntimeServices.prepare(args,root,[cfg],[cfg['templates']],key,
        url.removeprefix('http://'),service_record,job_count=4,registry_config=registry)
    create_identities = {}
    helper = args.probe.read_bytes()

    def command(sb, text):
        return sb.commands.run(text, user='root', timeout=45)

    def check(sb, protocol, address, port, expected):
        actual = probe_result(command(sb, f'/tmp/network-probe {protocol} {address} {port}'))
        if actual != expected:
            raise AssertionError('network policy differs: ' + protocol + ' ' + address)
        rows.append({'protocol': protocol, 'address': address, 'port': port, 'allowed': actual})

    def capture(attempt, identity, center, host):
        if attempt in known and known[attempt] != identity['binding']['session']:
            raise RuntimeError('saved original global identity changed')
        known[attempt] = identity['binding']['session']
        save(root/'original-identities.json',known)
        create_identities[attempt] = identity
        save(root/'original-create-identities.json',create_identities)
        local = local_binding(host,identity['binding']['session'],expected_node=identity['binding']['node'])
        original = [row for row in host['native'] if row['sandbox']==local['sandbox']]
        if len(original)!=1:
            raise RuntimeError('original native observation unknown')
        if original[0]['phase']=='Starting':
            return False
        services.processes.stop('host-0')
        try:
            native_id=services.native_id_for(identity['binding']['session'])
            secret,name,record=record_for(root,native_id=native_id)
            bound=secret['template']
            for field in ('template_id','revision','resources','envd_version'):
                if bound[field]!=template[field]:
                    raise RuntimeError('original immutable template changed')
            for field in ('kernel','rootfs'):
                if bound[field]['sha256']!=template[field]['sha256']:
                    raise RuntimeError('original template fingerprint changed')
            record['network_inode']=json.loads((root/'jails/.claims'/name/'network.json').read_text())['namespace_inode']
            identities[name]=record
            save(root/'resource-evidence.json',identities)
        finally:
            services.processes.start('host-0')
        services.wait_ready(for_admission=False)

    def create(label, **policy):
        attempt = secrets.token_hex(16)
        intents[attempt] = {'proxy_policy_probe': namespaces[0], 'attempt': attempt, 'case': label}
        save(root / 'intents.json', intents)
        sb = Sandbox.create(template='base', timeout=600, metadata=intents[attempt], **policy, **options)
        known[attempt] = sb.sandbox_id
        save(root / 'original-identities.json', known)
        center=services.planned_audit('center')
        identity=find_runtime_create(center,intents[attempt],deployment=services.center_config['deployment_id'])
        if identity['binding']['session']!=sb.sandbox_id:
            raise RuntimeError('original SDK identity changed')
        if capture(attempt,identity,center,services.planned_audit('host-0')) is False:
            raise RuntimeError('successful SDK create lacks its original running VM')
        sb.files.write('/tmp/network-probe', BytesIO(helper), user='root')
        if command(sb, 'chmod 700 /tmp/network-probe').exit_code != 0:
            raise RuntimeError('static probe installation failed')
        return sb

    def delete(identity):
        dispatched.add(identity)
        save(root / 'destroy-dispatched.json', sorted(dispatched))
        status, _ = fetch(opener, url, 'DELETE', '/sandboxes/' + identity, key)
        if status not in (200, 204, 404):
            raise RuntimeError('original destruction rejected')
        return status

    def settle(deadline):
        expected=dict(instances=1,cpu=template['resources']['cpu_count'],
            memory_bytes=template['resources']['host_memory_bytes'],disk_bytes=template['resources']['disk_bytes'])
        result={}
        for attempt,metadata in intents.items():
            result[attempt]=recover_original_create(metadata,expected_resources=expected,
                read_center=lambda:services.planned_audit('center'),
                read_host=lambda node:services.planned_audit('host-0'),
                deployment=services.center_config['deployment_id'],owner=1,delete=delete,
                deadline=deadline,sandbox_id=known.get(attempt),create_identity=create_identities.get(attempt),
                delete_dispatched=known.get(attempt) in dispatched,
                before_delete=lambda identity,center,host:capture(attempt,identity,center,host))
        return result

    def finish_vm(sb):
        settle(time.monotonic()+60)

    with socket.socket() as sentinel:
        sentinel.bind((host, 0))
        sentinel.listen(16)
        sentinel_port = sentinel.getsockname()[1]
        try:
            proxy.start()
            services.start()
            services.wait_ready()
            public = create('public', allow_internet_access=True)
            for protocol, address, port in [('dns', '1.1.1.1', 53), ('dns', '8.8.8.8', 53),
                                            ('tcp', '1.1.1.1', 443), ('tcp', '8.8.8.8', 53)]:
                check(public, protocol, address, port, True)
            with socket.create_connection((host, sentinel_port), timeout=3):
                pass
            check(public, 'tcp', host, sentinel_port, False)
            with socket.create_connection((host, sentinel_port), timeout=3):
                pass
            finish_vm(public)
            closed = create('closed', allow_internet_access=False)
            for protocol in ('dns', 'tcp'):
                check(closed, protocol, '1.1.1.1', 53 if protocol == 'dns' else 443, False)
            if command(closed, "printf 'nameserver 8.8.8.8\\n' > /etc/resolv.conf && echo CONTROL_OK").stdout.strip() != 'CONTROL_OK':
                raise RuntimeError('guest root resolver rewrite failed')
            check(closed, 'dns', '8.8.8.8', 53, False)
            finish_vm(closed)
            static = create('static', allow_internet_access=False,
                            network={'allow_out': ['1.1.1.1'], 'deny_out': ['0.0.0.0/0']})
            check(static, 'dns', '1.1.1.1', 53, True)
            check(static, 'tcp', '1.1.1.1', 443, True)
            check(static, 'dns', '8.8.8.8', 53, False)
            check(static, 'tcp', '8.8.8.8', 53, False)
            finish_vm(static)
            public = create('proxy_down', allow_internet_access=True)
            check(public, 'dns', '1.1.1.1', 53, True)
            check(public, 'tcp', '1.1.1.1', 443, True)
            proxy.stop_for_probe()  # Exact original unit identity, no parent VPN stop.
            check(public, 'dns', '1.1.1.1', 53, False)
            check(public, 'tcp', '1.1.1.1', 443, False)
            if command(public, 'echo CONTROL_OK').stdout.strip() != 'CONTROL_OK':
                raise RuntimeError('management channel lost with proxy')
            finish_vm(public)
        except BaseException as error:
            failure = error
        finally:
            try:
                if intents:
                    settle(time.monotonic()+90)
                audits = [audit_gone(root, args.firecracker, args.cgroup_parent, name, record)
                          for name, record in identities.items()]
                if len(audits) != len(intents) or processes_in_uid_range(args.uid_base, args.uid_base):
                    raise RuntimeError('claim count or UID cleanup unknown')
                links = proxy.query('link', 'show')
                if any(x['ifname'].startswith('sp' + namespaces[0][:6]) for x in links):
                    raise RuntimeError('original VM uplink survives')
                tables = json.loads(proxy.run(['/usr/sbin/nft', '-j', 'list', 'tables']))['nftables']
                if any(t.get('table', {}).get('name', '').startswith(('spf' + namespaces[0], 'spn' + namespaces[0])) for t in tables):
                    raise RuntimeError('original VM firewall survives')
                quiescent = True
            except Exception as error:
                cleanup_errors.append(type(error).__name__)
            transport = None
            if quiescent:
                try:
                    services.processes.stop_all()
                    services.cleanup_staging()
                    if proxy.state['phase'] != 'new':
                        transport = proxy.finish(quiescent=True)
                except Exception as error:
                    cleanup_errors.append(type(error).__name__)
            elif proxy.state['phase'] != 'new':
                try:
                    transport = proxy.finish(quiescent=False)
                except Exception as error:
                    cleanup_errors.append(type(error).__name__)
            try:
                default_unchanged = before == subprocess.check_output(['/usr/sbin/ip', '-j', 'route', 'show', 'default'])
            except Exception as error:
                default_unchanged = False
                cleanup_errors.append(type(error).__name__)
            report = {'commit': commit, 'inputs': inputs, 'passed': failure is None and quiescent and not cleanup_errors and default_unchanged,
                'scope': 'dedicated real KVM ACL fixture; not original tasks, full capacity or 89-task evidence',
                'observations': rows, 'error': type(failure).__name__ if failure else None,
                'exact_quiescence': quiescent, 'cleanup_errors': cleanup_errors,
                'transport': transport, 'host_default_unchanged': default_unchanged}
            save(root / 'report.json', report)
            print(json.dumps(report), flush=True)
    if not report['passed']:
        raise RuntimeError('proxy policy acceptance failed; exact artifacts retained') from failure


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('kernel', 'rootfs', 'jailer', 'firecracker', 'probe', 'proxy-binary',
                 'scratch-parent', 'cgroup-parent', 'service-cgroup-parent'):
        parser.add_argument('--' + name, required=True, type=Path)
    for name in ('probe-sha256', 'proxy-sha256', 'host-address'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--uid-base', type=int, required=True)
    parser.add_argument('--memory-mib', type=int, default=2048)
    parser.add_argument('--proxy-socks-port', type=int, default=7897)
    from _runtime_services import add_arguments
    add_arguments(parser)
    exercise(parser.parse_args())
