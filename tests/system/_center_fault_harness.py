"""One original RuntimeServices cohort for mandatory real DK fault probes.

This is test orchestration, not a second controller, recovery algorithm or
resource authority. Every restart, audit and terminal observation delegates
the reviewed product entry and original system helpers. Unknown retains the
exact fixture; no public create is retried or local identity guessed.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import time
import urllib.request

from tools import ci_e2e_artifacts, service_cgroup
from tools.runtime_create_recovery import find_runtime_create, terminal_result
from tools.system_acceptance import digest, json_file, private_path, write_once
from _center_host_services import local_binding, process_facts
from _internal_fault_relay import InternalFaultRelay, RelayMaterials
from _runtime_services import MAX_ACTIVATION_BYTES
from server_process_crash import fetch, free_loopback_port, prepare_fault_services, process_start_ticks
from server_unready_abort import wait_for
from server_vm_id_capacity import check_no_resources
from server_bounded_churn import assert_budget, database_snapshot, tree_records
from tools.template_physical_storage import envelope

MIB = 1024**2
CASE_BYTES = 512 * 1024
HISTORY_RECORDS = 256


def same_running_process(facts):
    """A PID alone is not an observation of the original living process."""
    return process_start_ticks(facts['pid']) == facts['start_time_ticks']


def center_record(audit, identity):
    found = [row for row in audit['sandboxes'] if row['create']['identity'] == identity]
    if len(found) != 1:
        raise ValueError('original center record missing or ambiguous')
    return found[0]


def physical_record(root, host_audit, identity):
    """Read one exact claim through checked native mapping, not an ID suffix."""
    local = local_binding(host_audit, identity['sandbox_id'], expected_node=identity['node'])
    native = [row for row in host_audit['native'] if row['sandbox'] == local['sandbox']]
    if len(native) != 1 or native[0]['current_vm'] is None:
        raise ValueError('original admitted VM binding unknown')
    node = json_file(Path(root)/'control/host.json')['native']
    name = 'vm-' + node['namespace'] + '-' + str(native[0]['current_vm'])
    claim_path = Path(root)/'jails/.claims'/name/'record.json'
    claim = json_file(private_path(claim_path), limit=MIB)
    if claim['resource_id'] != name or not same_running_process(claim):
        raise ValueError('exact original VM process is not living')
    metadata = claim_path.parent.stat()
    return dict(resource_id=name, pid=claim['pid'], start_time_ticks=claim['start_time_ticks'],
                claim_directory=dict(device=metadata.st_dev, inode=metadata.st_ino), local=local)


def vm_gone(root, identity, firecracker, cgroup_parent):
    """Observe exact known resources only; never send a host kill or remove them."""
    name = identity['resource_id']
    if same_running_process(identity):
        return False
    return not any(path.exists() for path in (
        Path(root)/'jails'/Path(firecracker).name/name,
        Path('/run/netns')/name, Path(cgroup_parent)/name))


def prepared_native(args, root, namespace):
    """Fresh small test inputs; the original renderer computes all future budgets."""
    disk = Path(args.rootfs).stat().st_size
    return dict(namespace=namespace, database=str(root/'db/controller.db'),
        node_state_dir=str(root/'state'), jailer=str(args.jailer), firecracker=str(args.firecracker),
        chroot_base=str(root/'jails'), kernel=str(args.kernel), rootfs=str(args.rootfs),
        uid_base=args.uid_base, vcpu_count=1, memory_mib=256, file_policy_digest=1,
        max_active_global=2, max_active_per_tenant=2, max_disk_bytes_global=disk*4,
        max_disk_bytes_per_tenant=disk*4, cleanup_reserve_bytes=disk,
        max_timeout_secs=600, max_lifetime_secs=600, default_timeout_secs=300,
        startup_timeout_secs=120, expiry_scan_seconds=1, public_ipv4_enabled=False,
        host_resources=dict(cgroup_parent=str(args.cgroup_parent), memory_max_bytes=512*MIB))


class Harness:
    """A single candidate/config/manifest with original detached observations."""
    def __init__(self, args):
        self.args, self.root = args, private_path(args.run_root, directory=True)
        self.key, self.foreign_key = secrets.token_hex(32), secrets.token_hex(32)
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.known, self.history = {}, []
        self.cases, self.last_processes = [], None
        self.next_center_audit = 0
        for part in ('db','state','jails'):
            (self.root/part).mkdir(mode=0o700)
        native = prepared_native(args,self.root,secrets.token_hex(8))
        self.services = prepare_fault_services(args,self.root,native,slots=2,key=self.key,
            port=free_loopback_port(),job_count=2,shared_template=True,
            extra_credentials=[dict(tenant=2,secret=self.foreign_key)])
        self.native = self.services.native_pools[0]
        profile = self.native['bounded_host']['physical_storage']
        control=self.services.layout['host_controls'][0]
        self.evidence_root=control/'dk-evidence'
        self.evidence_root.mkdir(mode=0o700)
        services=[entry for entry in profile['service_data'] if entry['root']==str(control)]
        if len(services)!=1 or services[0]['owner_uid']!=0:
            raise ValueError('original declared private host-control evidence budget missing')
        self.evidence_budget=services[0]
        peak, inodes = envelope(profile)
        assert 2*peak <= MAX_ACTIVATION_BYTES == 200*1024**3
        self.policy = dict(schema_version=1,rounds=args.rounds,case_wait_seconds=args.wait_seconds,
            controlled_test_debug_assertions=True,
            rootfs_bytes=Path(args.rootfs).stat().st_size,rootfs_sha256=digest(args.rootfs),
            kernel_sha256=digest(args.kernel),service_profile_sha256=digest(args.service_profile),
            native_config_sha256=hashlib.sha256(json.dumps(self.native,sort_keys=True).encode()).hexdigest(),
            producer_sha256=digest(Path(__file__).with_name('center_host_faults.py')),
            stale_probe_sha256=args.stale_probe_sha256,
            physical_activation_bytes=2*peak,physical_activation_inodes=2*inodes)
        write_once(self.root/'dk-policy.json',self.policy)
        backend = tuple_address(self.services.host_configs[0]['internal']['bind'])
        relay_address = ('127.0.0.1',free_loopback_port())
        self.materials = RelayMaterials(args.tls_material,self.services.layout['host_controls'][0],
                                       self.services.tls_receipt)
        self.relay = InternalFaultRelay(listen=relay_address,backend=backend,
            node=self.services.host_configs[0]['inbox']['node'],materials=self.materials)
        # One fresh test-only registered endpoint, before initialization and
        # before sealing the execution receipt. No live config is rewritten.
        self.services.center_config['nodes'][0]['address'] = ':'.join(map(str,relay_address))
        self._seal_fresh_center_config()
        self._instrument_original_starts()

    def _seal_fresh_center_config(self):
        path = Path(self.services.processes.bindings['center']['config'])
        old = path.stat()
        if not stat.S_ISREG(old.st_mode) or old.st_nlink != 1 or old.st_mode&0o7777 != 0o600:
            raise ValueError('fresh protected center input changed before initialization')
        raw = json.dumps(self.services.center_config,separators=(',',':'),ensure_ascii=False).encode()
        fd = os.open(path,os.O_WRONLY|os.O_NOFOLLOW)
        with os.fdopen(fd,'wb') as output:
            opened = os.fstat(output.fileno())
            if (opened.st_dev,opened.st_ino)!=(old.st_dev,old.st_ino):
                raise ValueError('fresh center input inode changed')
            output.write(raw)
            output.truncate()
            output.flush()
            os.fsync(output.fileno())

    def _instrument_original_starts(self):
        processes = self.services.processes
        original = processes.start
        def start(name, *, env=None):
            child = original(name,env=env)
            role = processes.bindings[name]
            def actual():
                try:
                    argv = [str(role['binary']),str(role['config'])] if name=='redis' else None
                    return process_facts(child,role['binary'],role['config'],processes.record,
                        uid=role['uid'],gid=role['gid'],argv=argv)
                except (OSError,ValueError):
                    return None
            observed = wait_for(actual,10,'actual original role exec/credentials')
            if len(self.history)>=HISTORY_RECORDS:
                raise ValueError('finite role-restart evidence budget exhausted')
            self.history.append(dict(role=name,**observed))
            return child
        processes.start = start

    def start(self):
        self.relay.start()
        self.services.start()
        self.services.wait_ready()
        self.manifest, self.last_processes = self.services.execution(
            self.args.run_identity,self.root/'dk-policy.json')
        write_once(self.root/'execution-manifest.json',self.manifest)
        self.execution_sha = digest(self.root/'execution-manifest.json')

    def sdk_options(self, *, foreign=False):
        from sdk_server_smoke import options
        return options(self.foreign_key if foreign else self.key,self.services.sdk_url,self.services.sdk_url)

    def fetch(self, method, path, body=None, *, foreign=False):
        return fetch(self.opener,self.services.sdk_url,method,path,
                     self.foreign_key if foreign else self.key,body)

    def center(self):
        # Poll checked facts at the original coordinator's scan cadence. Do
        # not repeatedly restart it faster than one scheduled reconciliation.
        remaining=self.next_center_audit-time.monotonic()
        if remaining>0:
            time.sleep(remaining)
        result=self.services.planned_audit('center')
        self.next_center_audit=time.monotonic()+(
            self.services.center_config['expiry_scan_ms']+self.services.center_config['poll_interval_ms'])/1000
        return result

    def host(self, *, resume_ready=True):
        # The original CLI's physical inspection requires its offline host
        # lease. This is an explicit planned stop/read/restart, never a live
        # SQL shortcut. Fault intervals that assert an unchanged host PID must
        # use their saved process/VM observations and must not call this helper.
        child=self.services.processes.children.get('host-0')
        if child is not None and child.poll() is None:
            result=self.services.planned_audit('host-0')
            if resume_ready:
                self.services.wait_ready(for_admission=False)
            return result
        return self.services.processes.audit('host-0')

    def remember(self, sandbox_id, *, metadata=None):
        audit = self.center()
        if metadata is not None:
            identity = find_runtime_create(audit,metadata,deployment=self.services.center_config['deployment_id'])
            assert identity['sandbox_id'] == sandbox_id
        else:
            rows = [row['create']['identity'] for row in audit['sandboxes']
                    if row['create']['identity']['sandbox_id']==sandbox_id]
            assert len(rows)==1
            identity = rows[0]
        physical = physical_record(self.root,self.host(),identity)
        self.known[sandbox_id] = dict(create=identity,physical=physical)
        return self.known[sandbox_id]

    def create(self, *, timeout=180, foreign=False, metadata=None):
        from e2b import Sandbox
        metadata = metadata or dict(dk_attempt=secrets.token_hex(16))
        sandbox = Sandbox.create(template='base',timeout=timeout,allow_internet_access=False,
                                 metadata=metadata,**self.sdk_options(foreign=foreign))
        self.remember(sandbox.sandbox_id,metadata=metadata)
        return sandbox

    def terminal(self, sandbox_id):
        known = self.known[sandbox_id]
        def observation():
            result = terminal_result(self.center(),self.host(),known['create'])
            return result if result and vm_gone(self.root,known['physical'],self.args.firecracker,
                                               self.args.cgroup_parent) else None
        return wait_for(observation,self.args.wait_seconds,'exact two-layer terminal and physical cleanup')

    def kill(self, sandbox_id, *, foreign=False):
        from e2b import Sandbox
        acknowledged=Sandbox.kill(sandbox_id,**self.sdk_options(foreign=foreign))
        assert acknowledged
        return self.terminal(sandbox_id)

    def complete_case(self, case, details):
        # Each mandatory case ends empty before the next fault, and every
        # original known claim has independently been observed gone.
        def empty():
            # This original read-only SQL snapshot is ONLY a poll hint. Avoid
            # repeatedly restarting healthy authorities throughout the fixed
            # native retention interval. Zero rows never proves cleanup: the
            # original checked two-layer audits below remain mandatory.
            pending=database_snapshot(self.native['database'])
            assert_budget(pending,self.native)
            if pending['counts']['sandboxes']!=0:
                return None
            center,host=self.center(),self.host()
            return (center,host) if (center['sandboxes']==[] and host['operations']==[] and host['native']==[]) else None
        center,host=wait_for(empty,self.args.wait_seconds,'both original retained histories independently retire')
        check_no_resources(self.root,self.native['namespace'],self.args.firecracker,self.args.uid_base,
            [value['physical'] for value in self.known.values()],native=self.native)
        snapshot = database_snapshot(self.native['database'])
        assert_budget(snapshot,self.native)
        assert not self.relay.failed.is_set()
        manifest, actual = self.services.execution(self.args.run_identity,self.root/'dk-policy.json')
        assert manifest==self.manifest, 'case changed the sealed role/config/candidate inputs'
        self.last_processes=actual
        service = service_cgroup.observe(self.args.service_cgroup_record,pids=[os.getpid(),*(row['pid'] for row in actual.values())])
        assert service['memory_events'].get('oom_kill',0)==0
        evidence = dict(schema_version=1,case=case,candidate=self.manifest['services']['candidate'],
            execution_manifest_sha256=self.execution_sha,details=details,
            actual_role_history=self.history,relay=self.relay.facts(),
            exact_cleanup=dict(center_rows=0,host_rows=0,physical_claims='gone',
                database_snapshot=snapshot,service_resources=service))
        raw = (json.dumps(evidence,sort_keys=True,indent=2)+'\n').encode()
        if len(raw)>CASE_BYTES:
            raise ValueError('private case evidence exceeds original finite test budget')
        self.check_evidence_budget(additional_bytes=len(raw),additional_entries=2)
        path=self.evidence_root/(case+'.json')
        write_once(path,evidence)
        self.check_evidence_budget()
        self.cases.append(dict(id=case,status='pass',exact_cleanup='passed',
            execution_manifest_sha256=self.execution_sha,evidence_sha256=digest(path)))

    def check_evidence_budget(self, *, additional_bytes=0,additional_entries=0):
        """Observe the ORIGINAL charged service root; never increase its quota."""
        records=tree_records((Path(self.evidence_budget['root']),))
        total=sum(record[3] for record in records.values() if record[2]==stat.S_IFREG)
        if (total+additional_bytes>self.evidence_budget['peak_bytes']
                or len(records)+additional_entries>self.evidence_budget['inode_budget']):
            raise ValueError('DK materials/evidence exceed original host-control service envelope')
        return dict(logical_file_bytes=total,entries=len(records),
                    peak_bytes=self.evidence_budget['peak_bytes'],inode_budget=self.evidence_budget['inode_budget'])

    def finish(self):
        dk = dict(schema_version=1,candidate=self.manifest['services']['candidate'],
                  execution_manifest_sha256=self.execution_sha,cases=self.cases)
        ci_e2e_artifacts.require_dk(dk,candidate=dk['candidate'])
        report = dict(schema_version=1,services=ci_e2e_artifacts.public_identity(self.services.manifest),
            service_boundary=dict(public_api='center',center_unprivileged=True,
                redis_authority='validated',host_transport='mutual-tls',host_public_management=False),
            execution_manifest=self.manifest,service_processes=self.last_processes,dk=dk)
        ci_e2e_artifacts.require_service_boundary(report,self.services.manifest)
        ci_e2e_artifacts.require_process_evidence(report['service_processes'],self.manifest)
        return report

    def stop(self, *, successful):
        self.relay.stop()
        self.services.stop_all()
        if successful:
            self.materials.remove()
            self.services.cleanup_staging()


def tuple_address(address):
    host, port = address.rsplit(':',1)
    if host!='127.0.0.1' or not port.isdecimal() or not 0<int(port)<65536:
        raise ValueError('one fixed loopback host endpoint required')
    return host,int(port)
