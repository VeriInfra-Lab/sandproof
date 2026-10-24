"""One approved create/unload fixture on the original protected deployment.

This module never creates a filesystem, mounts, enables quotas or configures
cgroups. Those exact resources and the first snapshot layout are prepared and
independently reviewed before this scenario. It reuses RuntimeServices, its
original manager processes and the existing HostLease, with no new supervisor.
Unknown effects preserve this run's resources and exclusive-host responsibility.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_runpy.run_path(str(_GuardPath(__file__).with_name('_assertions_required.py')))['require_assertions']()

from contextlib import closing
import copy
import fcntl
import hashlib
import json
import math
import os
import re
from pathlib import Path
import secrets
import sqlite3
import stat
import time

from _runtime_services import RuntimeServices, add_arguments as service_arguments
from _center_host_services import local_binding, role_facts
from server_process_crash import free_loopback_port, process_start_ticks
from tools.check_kvm_host import unique_keys
from tools.ci_e2e_supervisor import HostLease
from tools.system_acceptance import private_path, json_file, write_once


def add_arguments(parser):
    service_arguments(parser)
    parser.add_argument('--quota-receipt-sha256',required=True)
    for name in ('run-root','fixture','quota-receipt','ci-state-root','native-lock'):
        parser.add_argument('--'+name,type=Path,required=True)


def remaining(deadline):
    if type(deadline) not in (int,float) or not math.isfinite(deadline):
        raise ValueError('original finite absolute deadline required')
    value=deadline-time.monotonic()
    if value<=0:
        raise TimeoutError('original scenario deadline expired; no completion inferred')
    return value


def original_process(source):
    if source['boot_id']!=Path('/proc/sys/kernel/random/boot_id').read_text().strip():
        raise ValueError('foreign original source boot')
    proc=Path('/proc')/str(source['pid'])
    try:
        fields=(proc/'stat').read_text().rsplit(')',1)[1].split()
    except FileNotFoundError:
        return False
    # A zombie still holds process identity/capacity; it is not a released PID.
    return int(fields[19])==source['start_time_ticks']


def checked_file(path,limit,*,mode=None):
    path=Path(path)
    before=path.lstat()
    if (not stat.S_ISREG(before.st_mode) or before.st_uid!=0 or before.st_nlink!=1
            or before.st_mode&0o077 or not 0<before.st_size<=limit
            or (mode is not None and before.st_mode&0o7777!=mode)):
        raise ValueError('original bounded private file required')
    fd=os.open(path,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK)
    identity=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_mode,m.st_uid,m.st_nlink)
    with os.fdopen(fd,'rb') as stream:
        if identity(os.fstat(stream.fileno()))!=identity(before):
            raise ValueError('original file replaced before read')
        digest=hashlib.file_digest(stream,'sha256').digest()
        if (identity(os.fstat(stream.fileno()))!=identity(before)
                or identity(path.lstat())!=identity(before)):
            raise ValueError('original file changed during read')
    return dict(bytes=before.st_size,sha256=list(digest),device=before.st_dev,inode=before.st_ino)


def original_json(path,limit):
    checked=checked_file(path,limit)
    raw=Path(path).read_bytes()
    if len(raw)!=checked['bytes'] or list(hashlib.sha256(raw).digest())!=checked['sha256']:
        raise ValueError('original JSON changed after file observation')
    return json.loads(raw,object_pairs_hook=unique_keys)


class UnloadRuntime:
    def __init__(self,args,fixture,root,captures,quota,lease,lock):
        self.root,self.args,self.inputs,self.captures=root,args,fixture,captures
        self.quota,self.lease,self.lock=quota,lease,lock
        self.owner_key,self.foreign_key=secrets.token_hex(32),secrets.token_hex(32)
        self.services=None
        self.running=None
        self.deadline=None
        self.physical_complete=False
        self._prepare()

    def _prepare(self):
        native=self.inputs['native']
        if (Path(native['database'])!=self.root/'db/controller.db'
                or Path(native['node_state_dir'])!=self.root/'state'
                or Path(native['chroot_base'])!=Path(self.captures[0]['layout_parent'])/'v'):
            raise ValueError('fixture paths must name the exact prepared run')
        # Deployment receipt is independent of the configuration it checks.
        profile=native['bounded_host']['physical_storage']
        device=Path(profile['coordination_file']).parent.stat().st_dev
        material=self.captures[0]['material_project']
        if (self.quota['filesystem_device']!=device
                or self.quota['project_id']!=material['project_id']
                or self.quota['hard_bytes']!=material['hard_bytes']
                or self.quota['hard_inodes']!=material['hard_inodes']
                or self.quota['accounting'] is not True or self.quota['enforcement'] is not True
                or self.quota['actual_readback_verified'] is not True):
            raise ValueError('independently read back original kernel hard quota required')
        self.services=RuntimeServices.prepare(self.args,self.root,[native],[self.inputs['catalog']],
            self.owner_key,'127.0.0.1:'+str(free_loopback_port()),self.args.service_cgroup_record,
            1,registry_config=self.inputs['registry'],
            extra_credentials=[dict(tenant=2,secret=self.foreign_key)],
            activation_ceiling_bytes=self.inputs['activation_ceiling_bytes'])
        self.configs={name:role_facts(Path(role['config']).parent,role['config'],
            uid=role['uid'],gid=role['gid']) for name,role in self.services.processes.bindings.items()}

    def start(self,deadline):
        remaining(deadline)
        self.deadline=deadline
        self.services.start()
        self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=True)

    def create_parameters(self):
        remaining(self.deadline)
        catalogs=self.services.processes.center_readiness(timeout=min(10,remaining(self.deadline)))['catalogs']
        descriptors=[item for item in self.services.center_config['templates']['templates'] if item['owner']==1]
        if len(descriptors)!=1:
            raise ValueError('one original registered template for owner one required')
        descriptor=descriptors[0]
        revision=descriptor['template_id']+'-'+descriptor['revision']
        node=self.services.host_configs[0]['inbox']['node']['node_id']
        entries=[entry for entry in catalogs[node] if entry['revision']==revision]
        if len(entries)!=1:
            raise ValueError('actual original catalog digest not observed')
        shape=descriptor['resources']
        # Take digest from ORIGINAL Rust's authenticated registered catalog;
        # do not reimplement TemplateBinding serde order in this test script.
        options=dict(env_vars={},metadata={'scenario':'live-unload'},
            networking=dict(allow_internet_access=False,network=None))
        return dict(template_revision=revision,template_digest=entries[0]['digest'],
            resources=dict(instances=1,cpu=shape['cpu_count'],memory_bytes=shape['host_memory_bytes'],
                disk_bytes=shape['disk_bytes']),options=list(json.dumps(options,separators=(',',':')).encode()),
            startup_timeout_ms=self.inputs['startup_timeout_ms'],timeout_ms=self.inputs['timeout_ms'],
            max_lifetime_ms=self.inputs['max_lifetime_ms'])

    def _host_snapshot(self,session,deadline,observe):
        remaining(deadline)
        process=self.services.processes.children.get('host-0')
        if process is None or process.poll() is not None:
            raise RuntimeError('original host manager must be live before planned audit')
        self.services.processes.stop('host-0')
        try:
            audit=self.services.processes.audit('host-0')
            local=local_binding(audit,session,expected_node=self.services.host_configs[0]['inbox']['node'])
            native=self.services.native_pools[0]
            database=private_path(native['database'])
            identity=(database.stat().st_dev,database.stat().st_ino)
            with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,timeout=min(2,remaining(deadline)))) as conn:
                conn.execute('BEGIN')
                ident='sb-'+native['namespace']+'-'+str(local['sandbox'])
                lifetime=conn.execute('SELECT owner_id,started_at_ms,deadline_ms,max_deadline_ms,startup_deadline_ms '
                    'FROM sandbox_lifetimes WHERE sandbox_id=?',(ident,)).fetchone()
                if lifetime is None or lifetime[0]!='tenant-'+str(local['owner']):
                    raise ValueError('original native lifetime owner mismatch')
                values=dict(zip(('started_at_ms','end_at_ms','max_end_at_ms','startup_deadline_ms'),lifetime[1:]))
                if any(type(v) is not int or v<0 for v in values.values()):
                    raise ValueError('complete original lifetime required')
                result=observe(audit,local,values,conn)
            if (database.stat().st_dev,database.stat().st_ino)!=identity:
                raise ValueError('original SQLite replaced during planned audit')
            return result
        finally:
            self.services.processes.start('host-0')
            self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)

    def record_running(self,session,deadline):
        def observe(audit,local,lifetime,_conn):
            slots=[context for context,phase in audit['host_slots']
                if context['binding']['sandbox']==local['sandbox'] and phase=='Occupied']
            if len(slots)!=1:
                raise ValueError('one exact original occupied position required')
            context=slots[0]
            native=self.services.native_pools[0]
            resource='vm-'+native['namespace']+'-'+str(context['binding']['vm'])
            claim=Path(native['chroot_base'])/'.claims'/resource/'record.json'
            source=original_json(claim,16384)
            if source['resource_id']!=resource or not original_process(source):
                raise ValueError('original live VMM not observed')
            result=dict(session=session,local=local,context=context,source=source,lifetime=lifetime)
            self.running=copy.deepcopy(result)
            return result
        return self._host_snapshot(session,deadline,observe)

    def unload_evidence(self,session,deadline):
        if self.running is None or self.running['session']!=session:
            raise ValueError('missing original running observation')
        def observe(audit,local,lifetime,conn):
            if local['sandbox']!=self.running['local']['sandbox'] or lifetime!=self.running['lifetime']:
                raise ValueError('unload changed original native identity or lifetime')
            if [(context,phase) for context,phase in audit['host_slots'] if context==self.running['context']] != [(self.running['context'],'Free')]:
                raise ValueError('original position not durably Free')
            rows=[row for row in audit['native'] if row['sandbox']==local['sandbox']]
            if len(rows)!=1 or rows[0]['phase']!='Paused' or rows[0]['terminal'] or rows[0]['current_vm'] is not None:
                raise ValueError('original native unload not complete')
            encoded,checksum=conn.execute('SELECT state_json,sha256 FROM controller_source_work WHERE id=1').fetchone()
            if len(encoded)>16*1024**2 or hashlib.sha256(encoded).digest()!=checksum:
                raise ValueError('original checked source record changed')
            sources=json.loads(encoded,object_pairs_hook=unique_keys)
            records=[record for record in sources['records'] if record['work']['anchor']['request']['request']['binding']['session']==session]
            if len(records)!=1:
                raise ValueError('one original saved capture responsibility required')
            record=records[0]
            captured=record['capture']
            if captured is None or captured['completed'] is None or captured['material_bytes'] is None:
                raise ValueError('original SQL material adoption absent')
            full=captured['full']
            expected_source=record['work']['requirement']['source']
            if any(expected_source[k]!=self.running['source'][k] for k in ('pid','start_time_ticks','boot_id')):
                raise ValueError('capture used a different VMM')
            if original_process(expected_source) or original_process(captured['worker']['process']):
                raise ValueError('original VMM or Full helper still holds a process identity')
            physical=self._materials(full,captured)
            facts=dict(source_incarnation=full['pause']['binding']['source_incarnation'],
                capture=full['pause']['binding']['capture'],manifest_digest=captured['completed'],
                material_bytes=captured['material_bytes'],**lifetime)
            self.physical_complete=True
            return dict(facts=facts,original_source_exited=True,original_helper_exited=True,
                original_position=copy.deepcopy(self.running['context']),position_phase='Free',**physical)
        return self._host_snapshot(session,deadline,observe)

    def _materials(self,full,captured):
        store=private_path(Path(self.captures[0]['layout_parent'])/'s',directory=True)
        bundles=[path for path in store.iterdir() if path.name.startswith('capture-')]
        if len(bundles)!=1:
            raise ValueError('this isolated scenario requires exactly one original bundle')
        bundle=bundles[0]
        private_path(bundle,directory=True)
        key=bundle.name.removeprefix('capture-')
        if len(key)!=64 or any(c not in '0123456789abcdef' for c in key):
            raise ValueError('invalid original directory key')
        witness=store/('completed-'+key)
        if {path.name for path in store.iterdir()}!={bundle.name,witness.name}:
            raise ValueError('unresolved material staging remains')
        manifest_file=bundle/'manifest.json'
        measured=checked_file(manifest_file,262144,mode=0o400)
        manifest=original_json(manifest_file,262144)
        if measured['sha256']!=captured['completed'] or manifest['binding']!=full['pause']['binding'] or manifest['compatibility']!=full['compatibility']:
            raise ValueError('original manifest binding mismatch')
        expected=('state.bin','memory.bin','disk-0.img')
        if {path.name for path in bundle.iterdir()}!=set(expected)|{'manifest.json'}:
            raise ValueError('original complete bundle file set differs')
        files=[]
        total=0
        for entry,name,cap in zip(manifest['files'],expected,(full['state_bytes'],full['memory_bytes'],full['disk_bytes']),strict=True):
            if entry['relative_path']!=name:
                raise ValueError('original bundle ordering differs')
            measurement=checked_file(bundle/name,cap,mode=0o400)
            if measurement['bytes']!=entry['length'] or measurement['sha256']!=entry['digest']:
                raise ValueError('original material bytes differ from adopted manifest')
            files.append(dict(name=name,**measurement));total+=measurement['bytes']
        completion=original_json(witness,16384)
        if (completion['version']!=1 or completion['action']!=captured['action']
                or completion['job_digest']!=captured['worker']['job_digest']
                or completion['worker']!=captured['worker']['process']
                or completion['source']!=full['pause']['source']
                or completion['source_resource']!=full['pause']['source_resource']):
            raise ValueError('physical completion original action/source/worker mismatch')
        if completion['manifest_digest']!=captured['completed'] or completion['material_bytes']!=total or total!=captured['material_bytes']:
            raise ValueError('original durable completion differs from adopted bytes')
        return dict(materials=files,manifest=measured,completion=checked_file(witness,16384,mode=0o400))

    def restart_managers(self,deadline):
        remaining(deadline)
        before=self.services.processes.observe()
        redis=self.services.processes.children['redis']
        self.services.processes.stop('center')
        self.services.processes.stop('host-0')
        for name in ('center','host-0'):
            role=self.services.processes.bindings[name]
            if role_facts(Path(role['config']).parent,role['config'],uid=role['uid'],gid=role['gid'])!=self.configs[name]:
                raise ValueError('original manager configuration changed before restart')
        # No initialize/provision, Redis restart, fresh database or new layout.
        self.services.processes.start('host-0')
        self.services.processes.start('center')
        self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        after=self.services.processes.observe()
        if redis is not self.services.processes.children['redis'] or before['redis']!=after['redis']:
            raise ValueError('original Redis process changed during manager restart')
        if any(before[name]['pid']==after[name]['pid'] for name in ('center','host-0')):
            raise ValueError('both original manager children must have exited and restarted')
        return dict(before=before,after=after,original_configs=self.configs,redis_preserved=True)

    def finish(self,report):
        # Successful physical checks are not product GC. All original SQLite,
        # immutable materials, byte charges and pins remain for independent
        # exact-resource test teardown, never automatic blanket removal here.
        if not report.get('complete') or not self.physical_complete:
            report['cleanup']='unknown-preserved-original-resources-and-host-lease'
            return
        self.services.stop_all()
        if original_process(self.running['source']):
            raise RuntimeError('original VMM identity remains; preserve host lease')
        audit=self.services.processes.audit('host-0')
        if any(phase!='Free' for _context,phase in audit['host_slots']):
            raise RuntimeError('original physical slot audit not wholly Free')
        from sdk_c02_acceptance import processes_in_uid_range
        native=self.services.native_pools[0]
        slots=native['bounded_host']['maintenance']['max_host_slots']
        if processes_in_uid_range(native['uid_base'],native['uid_base']+slots-1):
            raise RuntimeError('original exclusive VM UID domain not empty')
        self.services.storage_peaks()
        report['cleanup']='exact-runtime-audit-free-managers-exited-original-materials-retained'
        # Actual host audit calls original observe_position_free, including
        # exact retirement of VMM/helpers/network. Product material references
        # and charges remain; CI exclusion continues through test-disk teardown.
        # Exact environment teardown exports evidence and reclaims only this
        # newly created filesystem before restoring CI's original capacity.
        # Keep the original CI exclusion through that separate reviewed step.
        report['host_lease']='retained-until-exact-filesystem-teardown'
        report['host_lease_identity']=self.lease.identity
        write_once(Path(self.quota['deployment_control'])/'scenario-stopped.json',
            dict(host_lease_identity=self.lease.identity,physical_audit='Free',
                 original_vmm_exited=True,managers_exited=True,
                 source=self.running['source']))


def retained_deployment_lease(args,quota):
    """Require existing original CI exclusion; never adopt a foreign marker."""
    identity=quota['host_lease_identity']
    if (re.fullmatch(r'sp638u-[0-9a-f]{16}',identity) is None
            or quota['ci_state_root']!=str(args.ci_state_root)
            or quota['deployment_control']!='/var/lib/sp638u-control'):
        raise ValueError('original reviewed installation/HostLease binding required')
    for name in ('paused','exclusive-host'):
        if json_file(private_path(args.ci_state_root/name))!={'e2e':identity}:
            raise ValueError('existing original deployment exclusion required; cannot acquire another lease')
    return HostLease(args.ci_state_root,identity)


def prepare(args):
    if os.geteuid()!=0:
        raise ValueError('approved isolated root host required')
    os.umask(0o077)
    root=private_path(args.run_root,directory=True)
    fixture=json_file(private_path(args.fixture),limit=1024**2)
    captures=json_file(private_path(args.capture_settings),limit=65536)
    receipt_file=private_path(args.quota_receipt)
    observed=checked_file(receipt_file,65536)
    if bytes(observed['sha256']).hex()!=args.quota_receipt_sha256:
        raise ValueError('quota receipt differs from independently approved fingerprint')
    quota=json_file(receipt_file,limit=65536)
    if (type(captures) is not list or len(captures)!=1 or set(fixture)!={
            'native','catalog','registry','activation_ceiling_bytes',
            'startup_timeout_ms','timeout_ms','max_lifetime_ms'}):
        raise ValueError('one exact current bounded lifecycle fixture required')
    # Prepared layout and quota readback predate all product processes. No
    # image, mount, quota or group configuration occurs in this scenario.
    lock=os.open(args.native_lock,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
    meta=os.fstat(lock)
    actual_lock=dict(device=meta.st_dev,inode=meta.st_ino,owner_uid=meta.st_uid)
    if (not stat.S_ISREG(meta.st_mode) or meta.st_nlink!=1 or meta.st_mode&0o077
            or actual_lock!=quota['native_lock_identity']):
        os.close(lock)
        raise ValueError('original approved native coordination identity changed')
    try:
        lease=retained_deployment_lease(args,quota)
    except BaseException:
        os.close(lock)
        raise
    try:
        lease.pause()
        deadline=time.monotonic()+args.scenario_timeout
        while not lease.exclusive():
            remaining(deadline);time.sleep(min(.2,remaining(deadline)))
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        write_once(private_path(quota['deployment_control'],directory=True)/'scenario-starting.json',
            dict(identity=lease.identity,boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                 pid=os.getpid(),start_time_ticks=process_start_ticks(os.getpid())))
        runtime=UnloadRuntime(args,fixture,root,captures,quota,lease,lock)
        write_once(root/'lifecycle-unload-admission.json',dict(operator_pid=os.getpid(),
            operator_start_time_ticks=process_start_ticks(os.getpid()),identity=lease.identity,
            boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            quota_receipt_sha256=args.quota_receipt_sha256))
        return runtime
    except BaseException:
        # The installed image/mount remain even before a VM. Never reopen CI
        # while that reserved backing space or a staging outcome is unresolved.
        # Preserve the original exclusion for the reviewed exact teardown.
        os.close(lock)
        raise
