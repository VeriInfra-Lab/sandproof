"""Approved repeated create/unload/resume/destroy on the original deployment.

This module never creates a filesystem, mounts, enables quotas or configures
cgroups. Those exact resources and the empty layout parent are prepared and
independently reviewed before this scenario. RuntimeServices invokes the original
host CLI once to provision that fresh layout. It reuses its
original manager processes and the existing HostLease, with no new supervisor.
Unknown effects preserve this run's resources and exclusive-host responsibility.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

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
import selectors
import subprocess
import sys
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
    parser.add_argument('--deployment-receipt-sha256',required=True)
    parser.add_argument('--deployment-receipt',type=Path,required=True)
    for name in ('run-root','fixture','quota-receipt','ci-state-root','native-lock'):
        parser.add_argument('--'+name,type=Path,required=True)


def remaining(deadline):
    if type(deadline) not in (int,float) or not math.isfinite(deadline):
        raise ValueError('original finite absolute deadline required')
    value=deadline-time.monotonic()
    if value<=0:
        raise TimeoutError('original scenario deadline expired; no completion inferred')
    return value


def checked_source_records(conn,host_config,deadline):
    """Read only HOST23's pre-reserved rows; never adopt an aggregate layout."""
    remaining(deadline)
    policy=host_config['native']['maintenance']
    inbox=host_config['inbox']
    configuration=host_config['source_observation']
    if conn.execute('SELECT version FROM controller_meta WHERE id=1').fetchone()!=(23,):
        raise ValueError('HOST23 original source layout required')
    q=inbox['max_sessions']*(policy['max_operations_per_sandbox']-2)
    reserve=configuration['max_work_bytes']+4*configuration['max_configuration_bytes']         +8192+131072+4*16384+4096+8192
    if not 0<q<=policy['capacity']['max_sql_rows'] or not 0<reserve<=min(16*1024**2,policy['capacity']['max_sql_value_bytes']):
        raise ValueError('bounded original per-work reservation required')
    header=conn.execute('SELECT count(*),max(length(state_json)),max(reserve_bytes) '
        'FROM controller_source_work').fetchone()
    if header[0]!=1 or header[1]>8192 or header[2]!=8192:
        raise ValueError('original source metadata reservation changed')
    encoded,checksum=conn.execute('SELECT state_json,sha256 FROM controller_source_work WHERE id=1').fetchone()
    if hashlib.sha256(encoded).digest()!=checksum:
        raise ValueError('original source metadata checksum changed')
    metadata=json.loads(encoded,object_pairs_hook=unique_keys)
    if (set(metadata)!={'config','node','max_sessions'} or metadata['config']!=configuration
            or metadata['node']!=inbox['node'] or metadata['max_sessions']!=inbox['max_sessions']):
        raise ValueError('original source metadata differs from this deployment')
    bounds=conn.execute('SELECT count(*),min(id),max(id),min(reserve_bytes),max(reserve_bytes),'
        'max(length(state_json)) FROM controller_source_work_records').fetchone()
    if bounds[:5]!=(q,1,q,reserve,reserve) or bounds[5]>reserve:
        raise ValueError('original per-work rows or reserved bounds changed')
    records=[];seen=set()
    for encoded,checksum,key in conn.execute('SELECT state_json,sha256,work_key '
            'FROM controller_source_work_records ORDER BY id'):
        remaining(deadline)
        if hashlib.sha256(encoded).digest()!=checksum:
            raise ValueError('original source work checksum changed')
        record=json.loads(encoded,object_pairs_hook=unique_keys)
        if record is None:
            if encoded!=b'null' or key is not None:
                raise ValueError('vacant original source row is not empty')
            continue
        if set(record)!={'work','observation','capture','restore'} or not isinstance(key,bytes) or len(key)!=32:
            raise ValueError('complete original source work row required')
        binding=record['work']['requirement']['work_binding']
        anchor=json.dumps(record['work']['anchor'],ensure_ascii=False,separators=(',',':')).encode()
        if list(key)!=binding or hashlib.sha256(anchor).digest()!=key or key in seen:
            raise ValueError('original immutable work key differs or repeats')
        seen.add(key);records.append(record)
    return records


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


def gate_json(path,limit,*,directory_fd=None):
    """One protected FD: never reopen the observation containing a secret."""
    path=Path(path)
    identity=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_mode,m.st_uid,m.st_nlink)
    name=str(path) if directory_fd is None else path.name
    metadata=lambda:os.stat(name,dir_fd=directory_fd,follow_symlinks=False)
    before=metadata()
    if (not stat.S_ISREG(before.st_mode) or before.st_uid!=0 or before.st_nlink!=1
            or stat.S_IMODE(before.st_mode)!=0o600 or not 0<before.st_size<=limit):
        raise ValueError('bounded root0600 original gate file required')
    fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,dir_fd=directory_fd)
    with os.fdopen(fd,'rb') as stream:
        if identity(os.fstat(stream.fileno()))!=identity(before):
            raise ValueError('gate file changed before protected read')
        raw=stream.read(limit+1)
        if (len(raw)!=before.st_size or len(raw)>limit
                or identity(os.fstat(stream.fileno()))!=identity(before)
                or identity(metadata())!=identity(before)):
            raise ValueError('gate file changed during protected read')
    return json.loads(raw,object_pairs_hook=unique_keys),hashlib.sha256(raw).hexdigest(),identity(before)


def validate_gate_ready(value,digest):
    if (type(value) is not dict or set(value)!={'schema_version','observation_sha256'}
            or type(value['schema_version']) is not int or value['schema_version']!=1
            or type(value['observation_sha256']) is not str
            or re.fullmatch('[0-9a-f]{64}',value['observation_sha256']) is None
            or value['observation_sha256']!=digest):
        raise ValueError('ready must seal the exact original gate observation')


def gate_command(run_identity,digest,command):
    if (type(run_identity) is not str or re.fullmatch('[0-9a-f]{64}',run_identity) is None
            or type(digest) is not str or re.fullmatch('[0-9a-f]{64}',digest) is None
            or command not in ('continue','fail')):
        raise ValueError('exact bounded run/observation gate command required')
    return dict(schema_version=1,run_identity=run_identity,observation_sha256=digest,command=command)


def gate_configuration(run_identity,sandbox,generation,timeout_ms):
    if (type(run_identity) is not str or re.fullmatch('[0-9a-f]{64}',run_identity) is None
            or type(sandbox) is not str or re.fullmatch('sb-[0-9a-f]{16}-[1-9][0-9]*',sandbox) is None
            or type(generation) is not int or not 0<generation<2**64
            or type(timeout_ms) is not int or not 1000<=timeout_ms<=120000):
        raise ValueError('exact original native target and bounded activation gate required')
    return dict(schema_version=1,run_identity=run_identity,sandbox=sandbox,
        generation=generation,timeout_ms=timeout_ms)


def gate_publish(directory_fd,name,value):
    if name not in ('config.json','command.json'):
        raise ValueError('only fixed activation gate consumer files may be published')
    data=(json.dumps(value,sort_keys=True,separators=(',',':'))+'\n').encode()
    if len(data)>1024:
        raise ValueError('bounded original gate command required')
    staging='command-stage.json' if name=='command.json' else name
    fd=os.open(staging,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,
        0o600,dir_fd=directory_fd)
    with os.fdopen(fd,'wb') as stream:
        stream.write(data);stream.flush();os.fsync(stream.fileno())
    if name=='command.json':
        # Linux-only controlled harness. No replace/link fallback: readers
        # see either no final command or the complete single-link file.
        import ctypes
        libc=ctypes.CDLL(None,use_errno=True)
        rename=libc.renameat2
        rename.argtypes=[ctypes.c_int,ctypes.c_char_p,ctypes.c_int,ctypes.c_char_p,ctypes.c_uint]
        rename.restype=ctypes.c_int
        if rename(directory_fd,staging.encode(),directory_fd,name.encode(),1)!=0:
            raise OSError(ctypes.get_errno(),'original no-replace command publication failed')
    os.fsync(directory_fd)


def validate_restore_gate(record, expected, run_identity):
    """Only the original activated candidate's consumed bootstrap input is admissible."""
    if (type(run_identity) is not str or re.fullmatch('[0-9a-f]{64}',run_identity) is None
            or type(record) is not dict or set(record)!={
            'schema_version','run_identity','action','candidate','namespace','token'}
            or type(record['schema_version']) is not int or record['schema_version']!=1
            or record['run_identity']!=run_identity
            or any(record[key]!=expected[key] for key in ('action','candidate','namespace'))
            or type(record['token']) is not str
            or re.fullmatch(r'[A-Za-z0-9_-]{32,128}',record['token']) is None):
        raise ValueError('exact run/action/live candidate bootstrap gate required')
    source,namespace=record['candidate'],record['namespace']
    action=record['action']
    if (type(source) is not dict or set(source)!={'resource_id','pid','start_time_ticks','boot_id'}
            or type(namespace) is not dict or set(namespace)!={'name','device','inode'}
            or namespace['name']!=source['resource_id']
            or any(type(source[key]) is not int or source[key]<=0 for key in ('pid','start_time_ticks'))
            or any(type(namespace[key]) is not int or namespace[key]<=0 for key in ('device','inode'))
            or type(action) is not dict or set(action)!={'identity','command'}
            or set(action['command']['logical']['command'])!={'ActivateVm'}):
        raise ValueError('complete original activation/process/namespace binding required')
    return record


def original_unload_operation(audit, acceptance, unloaded):
    """Read the original center history at the existing manager restart."""
    sessions=[row for row in audit['sandboxes']
        if row['create']['binding']['session']==acceptance['session']]
    if len(sessions)!=1:
        raise ValueError('one original public session required after duplicate delivery')
    operations=sessions[0]['operations']
    matches=[row for session in audit['sandboxes'] for row in session['operations']
        if row['binding']['key']==acceptance['key']]
    if (len(matches)!=1 or len(operations)!=2
            or [row['operation']['kind'] for row in operations]!=['create','unload']
            or matches[0]['binding']['session']!=acceptance['session']
            or matches[0]['accepted_version']!=acceptance['accepted_version']
            or matches[0]['operation']['capture']!=unloaded['unload_outcome']['facts']['capture']
            or matches[0]['operation']['source_incarnation']!=unloaded['unload_outcome']['facts']['source_incarnation']):
        raise ValueError('duplicate delivery produced a different/additional original operation')
    return dict(session=acceptance['session'],key=acceptance['key'],
        accepted_version=acceptance['accepted_version'],matching_operations=1,
        original_session_operations=2,capture=matches[0]['operation']['capture'])


def assert_same_restore_candidate(running, receipt):
    """E7 requires the gate's host candidate, independently of guest RNG state."""
    if (type(running) is not dict or type(receipt) is not dict
            or type(running.get('source')) is not dict
            or type(receipt.get('candidate')) is not dict
            or set(receipt['candidate'])!={'resource_id','pid','start_time_ticks','boot_id'}
            or any(running['source'].get(key)!=value for key,value in receipt['candidate'].items())
            or type(receipt.get('namespace')) is not dict
            or set(receipt['namespace'])!={'name','device','inode'}
            or running.get('namespace')!=receipt['namespace']):
        raise ValueError('recovered Running VMM/namespace differs from the original gate candidate')


def namespace_witness(record, timeout):
    """Fixed original SDK transport in one independently checked host namespace.

    Secret data enters only stdin. No /init, public route, caller command or
    guest mutation is available through this physical observation helper.
    """
    from lifecycle_unload_scenario import RAM_WITNESS_CLIENT
    command='python3 -c '+__import__('shlex').quote(RAM_WITNESS_CLIENT)+' read'
    program='''import json,sys
from e2b import Sandbox
from e2b.connection_config import ConnectionConfig
from packaging.version import Version
value=json.load(sys.stdin)
connection=ConnectionConfig(api_key='private-physical-oracle',api_url='http://127.0.0.1:1',
    sandbox_url='http://172.16.0.2:49983',domain='example.invalid',retries=0,request_timeout=5,
    extra_sandbox_headers={'X-Access-Token':value['token']})
sandbox=Sandbox(sandbox_id=value['resource_id'],sandbox_domain='example.invalid',
    envd_version=Version('0.9.0'),envd_access_token=value['token'],connection_config=connection)
result=sandbox.commands.run(COMMAND,timeout=5)
if result.exit_code!=0: raise RuntimeError('original fixed read failed')
print(result.stdout)
'''.replace('COMMAND',repr(command))
    payload=json.dumps(dict(resource_id=record['candidate']['resource_id'],token=record['token'])).encode()
    if len(payload)>4096:
        raise ValueError('bounded private bootstrap handoff required')
    read_fd,write_fd=os.pipe2(os.O_CLOEXEC)
    process=None
    try:
        if os.write(write_fd,payload)!=len(payload):
            raise ValueError('complete private bootstrap handoff required')
        os.close(write_fd);write_fd=None
        process=subprocess.Popen(['/usr/bin/ip','netns','exec',record['namespace']['name'],
            sys.executable,'-c',program],stdin=read_fd,stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})
        result=bytearray();deadline=time.monotonic()+timeout
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout,selectors.EVENT_READ)
            while True:
                left=remaining(deadline)
                if not selector.select(left):
                    raise TimeoutError('original namespace witness deadline')
                data=os.read(process.stdout.fileno(),min(4096,8193-len(result)))
                if not data:break
                result.extend(data)
                if len(result)>8192:
                    raise ValueError('original namespace witness exceeds bound')
        if process.wait(timeout=max(.001,remaining(deadline)))!=0:
            raise ValueError('original namespace witness command failed')
        return json.loads(result,object_pairs_hook=unique_keys)
    finally:
        os.close(read_fd)
        if write_fd is not None:os.close(write_fd)
        if process is not None:
            process.stdout.close()
            if process.poll() is None:
                process.kill();process.wait(timeout=5)


class UnloadRuntime:
    def __init__(self,args,fixture,root,captures,quota,lease,lock):
        self.root,self.args,self.inputs,self.captures=root,args,fixture,captures
        self.quota,self.lease,self.lock=quota,lease,lock
        self.owner_key,self.foreign_key=secrets.token_hex(32),secrets.token_hex(32)
        self.services=None
        self.running=None
        self.runs={}
        self.material_paths={}
        self.gc_complete=False
        self.restore_gate=None
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
                or self.quota['projects']!=[{key:material[key] for key in
                    ('project_id','hard_bytes','hard_inodes')}]
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

    def measurement_identity(self):
        """Reuse validated artifact identities; disclose no configuration secrets."""
        from tools.ci_e2e_artifacts import public_identity
        def fingerprint(value):
            return hashlib.sha256(json.dumps(value,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        return dict(candidate=public_identity(self.services.manifest),
            candidate_manifest_sha256=self.services.manifest_sha256,
            fixture_sha256=fingerprint(self.inputs),capture_settings_sha256=fingerprint(self.captures),
            role_configuration_identity_sha256=fingerprint(self.configs),
            quota_readback_sha256=fingerprint(self.quota),
            images=[{kind+'_sha256':template[kind]['sha256'] for kind in ('kernel','rootfs')}
                for template in self.inputs['catalog']['templates']],
            cpu_condition='unknown:not separately fingerprinted by this timing instrumentation',
            cache_condition='unknown:not controlled as cold or warm',
            workload='original fixed RAM witness, disk marker and repeated lifecycle; no added performance rounds')

    def create_parameters(self,owner=1):
        remaining(self.deadline)
        catalogs=self.services.processes.center_readiness(timeout=min(10,remaining(self.deadline)))['catalogs']
        descriptors=[item for item in self.services.center_config['templates']['templates'] if item['owner']==owner]
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

    def confirm_running(self,session,expected,deadline):
        """Reobserve the occupying session while another restored VM is live."""
        def observe(audit,local,lifetime,conn):
            occupied=[context for context,phase in audit['host_slots']
                if context['binding']['sandbox']==local['sandbox'] and phase=='Occupied']
            if (occupied!=[expected['context']] or local!=expected['local']
                    or lifetime!=expected['lifetime'] or not original_process(expected['source'])):
                raise ValueError('original other-session slot occupant changed during Resume')
            return dict(context=occupied[0],source=expected['source'],live=True)
        return self._host_snapshot(session,deadline,observe)

    def record_running(self,session,deadline,resume_acceptance=None,gate_receipt=None):
        def observe(audit,local,lifetime,conn):
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
            if gate_receipt is not None:
                if (resume_acceptance is None or gate_receipt.get('session')!=session
                        or gate_receipt.get('operation')!=resume_acceptance['key']):
                    raise ValueError('original Resume acceptance required with activation gate evidence')
                namespace=Path('/run/netns')/resource
                meta=namespace.lstat()
                network=original_json(Path(native['chroot_base'])/'.claims'/resource/'network.json',16384)
                if (not stat.S_ISREG(meta.st_mode) or meta.st_uid!=0
                        or network['resource_id']!=resource or network['namespace_inode']!=meta.st_ino
                        or network['namespace_device']!=meta.st_dev):
                    raise ValueError('original recovered candidate namespace identity required')
                result['namespace']=dict(name=resource,device=meta.st_dev,inode=meta.st_ino)
                assert_same_restore_candidate(result,gate_receipt)
            if resume_acceptance is not None:
                sources={'records':checked_source_records(conn,self.services.host_configs[0],deadline)}
                records=[row for row in sources['records'] if row['restore'] is not None
                    and row['restore']['intent']['request']['binding']['session']==session
                    and row['restore']['intent']['request']['binding']['key']==resume_acceptance['key']]
                if len(records)!=1 or records[0]['restore']['material_use'] is not True:
                    raise ValueError('running restored VMM lacks its original backing pin')
                result['running_backing']=self._materials(records[0]['capture']['full'],records[0]['capture'])
                result['material_use_pin']=True
            prior=self.runs.get(session)
            if prior is not None:
                if (any(local[key]!=prior['local'][key] for key in ('sandbox','owner'))
                        or lifetime!=prior['lifetime'] or source==prior['source']
                        or context['binding']['generation']<=prior['context']['binding']['generation']):
                    raise ValueError('Resume must preserve native session/lifetime with a fresh VMM generation')
            self.running=copy.deepcopy(result)
            self.runs[session]=copy.deepcopy(result)
            return result
        return self._host_snapshot(session,deadline,observe)

    def unload_evidence(self,session,deadline,operation):
        self.running=self.runs.get(session)
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
            sources={'records':checked_source_records(conn,self.services.host_configs[0],deadline)}
            records=[record for record in sources['records']
                if record['work']['anchor']['request']['request']['binding']['session']==session
                and record['work']['anchor']['request']['request']['binding']['key']==operation]
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
            self.material_paths.setdefault(session,set()).update(physical['material_paths'])
            facts=dict(source_incarnation=full['pause']['binding']['source_incarnation'],
                capture=full['pause']['binding']['capture'],manifest_digest=captured['completed'],
                material_bytes=captured['material_bytes'],**lifetime)
            self.physical_complete=True
            return dict(facts=facts,original_source_exited=True,original_helper_exited=True,
                original_capture_responsibilities=len(records),
                original_position=copy.deepcopy(self.running['context']),position_phase='Free',**physical)
        return self._host_snapshot(session,deadline,observe)

    def _materials(self,full,captured):
        store=private_path(Path(self.captures[0]['layout_parent'])/'s',directory=True)
        bundles=[]
        for path in store.iterdir():
            if not path.name.startswith('capture-'):
                continue
            private_path(path,directory=True)
            if checked_file(path/'manifest.json',262144,mode=0o400)['sha256']==captured['completed']:
                bundles.append(path)
        if len(bundles)!=1:
            raise ValueError('exact original manifest-bound bundle required')
        bundle=bundles[0]
        private_path(bundle,directory=True)
        key=bundle.name.removeprefix('capture-')
        if len(key)!=64 or any(c not in '0123456789abcdef' for c in key):
            raise ValueError('invalid original directory key')
        witness=store/('completed-'+key)
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
        return dict(materials=files,manifest=measured,completion=checked_file(witness,16384,mode=0o400),
            material_paths=[str(bundle),str(witness)])

    def arm_restore_gate(self,session,deadline,timeout_ms):
        """Plan only this debug fault, using the original paused native audit."""
        if self.restore_gate is not None:
            raise ValueError('one original activation gate only')
        remaining(deadline)
        native=self.services.native_pools[0]
        previous=self.runs[session]['context']['binding']['generation']
        if type(previous) is not int or not 0<previous<2**64-1:
            raise ValueError('original positive running generation required before arming')
        config=gate_configuration(self.lease.identity,
            'sb-'+native['namespace']+'-'+str(self.runs[session]['local']['sandbox']),previous+1,timeout_ms)
        old=self.services.processes.children['host-0']
        if old.poll() is not None:
            raise ValueError('original live host manager required before arming gate')
        self.services.processes.stop('host-0')
        audit=self.services.processes.audit('host-0')
        local=local_binding(audit,session,expected_node=self.services.host_configs[0]['inbox']['node'])
        rows=[row for row in audit['native'] if row['sandbox']==local['sandbox']]
        if (len(rows)!=1 or rows[0]['phase']!='Paused' or rows[0]['terminal']
                or rows[0]['current_vm'] is not None or local!=self.runs[session]['local']
                or type(rows[0]['generation']) is not int or rows[0]['generation']!=previous):
            raise ValueError('original paused generation required for fault selection')
        control=private_path(self.quota['deployment_control'],directory=True)
        control_meta=control.stat()
        path=control/'restore-activation-gate'
        path.mkdir(mode=0o700)
        fd=os.open(path,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
        meta=os.fstat(fd)
        self.restore_gate=dict(path=path,fd=fd,directory_identity=(meta.st_dev,meta.st_ino),
            control_identity=(control_meta.st_dev,control_meta.st_ino),
            config=config,session=session,deadline=min(deadline,time.monotonic()+timeout_ms/1000),
            command_sent=False,observation_sha256=None,files={})
        self._gate_directory()
        gate_publish(fd,'config.json',config)
        _,digest,identity=gate_json(path/'config.json',1024,directory_fd=fd)
        self.restore_gate['files']['config.json']=(digest,identity)
        role=self.services.processes.bindings['host-0']
        if role_facts(Path(role['config']).parent,role['config'],uid=role['uid'],gid=role['gid'])!=self.configs['host-0']:
            raise ValueError('original host product configuration changed before fault restart')
        process=self.services.processes.start('host-0',env={
            'SANDPROOF_TEST_RESTORE_ACTIVATION_GATE':str(path),
            'SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION':'activate_vm'})
        self.restore_gate['host']=process
        self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)

    def _gate_directory(self):
        gate=self.restore_gate
        fdmeta=os.fstat(gate['fd']);meta=gate['path'].lstat()
        if (not stat.S_ISDIR(meta.st_mode) or meta.st_uid!=0 or stat.S_IMODE(meta.st_mode)!=0o700
                or (meta.st_dev,meta.st_ino)!=gate['directory_identity']
                or (fdmeta.st_dev,fdmeta.st_ino)!=gate['directory_identity']):
            raise ValueError('original root0700 activation gate directory changed')

    def _gate_files_unchanged(self):
        self._gate_directory()
        gate=self.restore_gate
        for name,expected in gate['files'].items():
            _,digest,identity=gate_json(gate['path']/name,
                16384 if name=='observation.json' else 1024,directory_fd=gate['fd'])
            if (digest,identity)!=expected:
                raise ValueError('original sealed activation gate file changed')

    def wait_resume_gate(self,session,accepted,baseline,deadline):
        gate=self.restore_gate
        if gate is None or gate['session']!=session:
            raise ValueError('original session gate must be armed before Resume')
        deadline=min(deadline,gate['deadline'])
        while True:
            remaining(deadline);self._gate_directory()
            if gate['host'] is not self.services.processes.children['host-0'] or gate['host'].poll() is not None:
                raise ValueError('original host exited before its committed activation gate')
            try:
                ready,ready_digest,ready_identity=gate_json(gate['path']/'ready.json',1024,directory_fd=gate['fd'])
            except FileNotFoundError:
                time.sleep(min(.05,remaining(deadline)));continue
            _,digest,identity=gate_json(gate['path']/'observation.json',16384,directory_fd=gate['fd'])
            validate_gate_ready(ready,digest)
            gate['files'].update({'ready.json':(ready_digest,ready_identity),'observation.json':(digest,identity)})
            gate['observation_sha256']=digest
            return self.observe_resume_gate(session,accepted,baseline,gate['path']/'observation.json',deadline)

    def release_resume_gate(self,command):
        gate=self.restore_gate
        if gate is None or gate['observation_sha256'] is None or gate['command_sent']:
            raise ValueError('one command for the original committed gate only')
        self._gate_files_unchanged()
        gate_publish(gate['fd'],'command.json',gate_command(
            self.lease.identity,gate['observation_sha256'],command))
        _,digest,identity=gate_json(gate['path']/'command.json',1024,directory_fd=gate['fd'])
        gate['files']['command.json']=(digest,identity)
        gate['command_sent']=True
        gate['command']=command

    def recover_resume_gate(self,receipt,deadline):
        gate=self.restore_gate
        self.release_resume_gate('continue')
        process=gate['host']
        if (process is not self.services.processes.children['host-0']
                or process.wait(timeout=remaining(deadline))!=86):
            raise ValueError('original activation manager did not exit at the bounded lost-receipt hook')
        self._gate_files_unchanged()
        if not original_process(receipt['candidate']):
            raise ValueError('gate candidate exited before original manager recovery')
        role=self.services.processes.bindings['host-0']
        if role_facts(Path(role['config']).parent,role['config'],uid=role['uid'],gid=role['gid'])!=self.configs['host-0']:
            raise ValueError('original product configuration changed across lost acknowledgement')
        # No initialize, new product config, inherited environment or fault
        # variables. The original Processes owner alone restarts this child.
        recovered=self.services.processes.start('host-0')
        if recovered is process or recovered.pid==process.pid:
            raise ValueError('original fault host was not replaced by an actual fresh manager')
        self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)
        gate['exit86_observed']=True
        return dict(exit_code=86,fault_manager_pid=process.pid,recovered_manager_pid=recovered.pid,
            same_product_configuration=True,fault_environment_removed=True,
            observation_sha256=gate['observation_sha256'],public_authorization_proven=False)

    def fail_resume_gate(self):
        gate=self.restore_gate
        if gate is not None and gate['observation_sha256'] is not None and not gate['command_sent']:
            self.release_resume_gate('fail')
            return 'original observation rejected; fail command published; outcome remains unknown'
        return 'no new gate command; original responsibility preserved'

    def cleanup_restore_gate(self):
        """Only after product settlement: delete exactly our four sealed files."""
        gate=self.restore_gate
        if gate is None:
            return None
        if (not self.physical_complete or not self.gc_complete
                or gate.get('exit86_observed') is not True or gate.get('command')!='continue'):
            raise ValueError('original exit86 and settled product resources required before gate cleanup')
        if not gate['command_sent'] or set(gate['files'])!={
                'config.json','observation.json','ready.json','command.json'}:
            raise ValueError('complete original activation gate required for cleanup')
        self._gate_files_unchanged()
        if set(os.listdir(gate['fd']))!=set(gate['files']):
            raise ValueError('unknown or partial gate entries retain original responsibility')
        control=gate['path'].parent
        parent_fd=os.open(control,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC)
        try:
            meta=os.fstat(parent_fd)
            if (meta.st_dev,meta.st_ino)!=gate['control_identity']:
                raise ValueError('original gate control parent changed')
            for name in ('command.json','ready.json','observation.json','config.json'):
                self._gate_directory()
                _,digest,identity=gate_json(gate['path']/name,
                    16384 if name=='observation.json' else 1024,directory_fd=gate['fd'])
                if (digest,identity)!=gate['files'][name]:
                    raise ValueError('gate file replacement forbids exact cleanup')
                file_fd=os.open(name,os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK|os.O_CLOEXEC,
                    dir_fd=gate['fd'])
                try:
                    opened=os.fstat(file_fd)
                    held=(opened.st_dev,opened.st_ino,opened.st_size,opened.st_mtime_ns,
                        opened.st_ctime_ns,opened.st_mode,opened.st_uid,opened.st_nlink)
                    if held!=identity:
                        raise ValueError('original gate FD changed before exact unlink')
                    # Producer exited86 and the original managers are stopped;
                    # only this supervisor writes the root-private directory.
                    os.unlink(name,dir_fd=gate['fd'])
                    if os.fstat(file_fd).st_nlink!=0:
                        raise ValueError('exact original gate FD was not unlinked')
                    os.fsync(gate['fd'])
                finally:
                    os.close(file_fd)
            if os.listdir(gate['fd']):
                raise ValueError('activation gate acquired an unknown entry during cleanup')
            self._gate_directory()
            os.rmdir(gate['path'].name,dir_fd=parent_fd)
            os.fsync(parent_fd)
        finally:
            os.close(parent_fd)
        gate['cleaned']=True
        return dict(exact_original_files=4,partial_stage_files_absent=True,
            private_token_record_removed=True,directory_removed=True)

    def close_restore_gate(self):
        if self.restore_gate is not None and self.restore_gate['fd'] is not None:
            os.close(self.restore_gate['fd']);self.restore_gate['fd']=None

    def observe_resume_gate(self,session,accepted,baseline,record_path,deadline):
        """Independent E7 physical oracle; never a public readiness assertion."""
        remaining(deadline)
        record_path=private_path(record_path)
        control=private_path(self.quota['deployment_control'],directory=True)
        if not record_path.is_relative_to(control):
            raise ValueError('gate material must belong to this private run control')
        state=self.restore_gate
        if state is None or record_path!=state['path']/'observation.json':
            raise ValueError('only the original selected activation gate observation is admissible')
        self._gate_files_unchanged()
        gate,digest,identity=gate_json(record_path,16384,directory_fd=state['fd'])
        if (digest,identity)!=state['files']['observation.json']:
            raise ValueError('original observation does not match committed ready digest')
        native=self.services.native_pools[0]
        database=private_path(native['database'])
        with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,
                timeout=min(2,remaining(deadline)))) as conn:
            conn.execute('BEGIN')
            sources={'records':checked_source_records(conn,self.services.host_configs[0],deadline)}
        claims=[row['restore'] for row in sources['records'] if row['restore'] is not None
            and row['restore']['intent']['request']['binding']['session']==session
            and row['restore']['intent']['request']['binding']['key']==accepted['key']
            and row['restore']['intent']['accepted_version']==accepted['accepted_version']]
        if len(claims)!=1 or claims[0]['activation'] is None:
            raise ValueError('one durable original MayHaveExecuted claim required')
        claim=claims[0]
        if (claim['intent']['request']['binding']['node']!=self.services.host_configs[0]['inbox']['node']
                or any(claim['local'][key]!=self.runs[session]['local'][key] for key in ('sandbox','owner'))):
            raise ValueError('original current global/native restore identity changed')
        action=claim['activation']
        if (action['identity']['sandbox']!=state['config']['sandbox']
                or action['identity']['generation']!=state['config']['generation']):
            raise ValueError('actual independent activation differs from selected gate target')
        command=action['command']['logical']['command']['ActivateVm']
        if (command['sandbox']!=claim['local']['sandbox']
                or command['generation']!=claim['generation']
                or action['identity']['generation']!=claim['generation']):
            raise ValueError('original activated native generation differs')
        resource='vm-'+native['namespace']+'-'+str(command['vm'])
        original=original_json(Path(native['chroot_base'])/'.claims'/resource/'record.json',16384)
        source={key:original[key] for key in ('resource_id','pid','start_time_ticks','boot_id')}
        if source['resource_id']!=resource or not original_process(source):
            raise ValueError('original activated VMM identity is not live')
        namespace=Path('/run/netns')/resource
        meta=namespace.lstat()
        network=original_json(Path(native['chroot_base'])/'.claims'/resource/'network.json',16384)
        if (not stat.S_ISREG(meta.st_mode) or meta.st_uid!=0
                or network['resource_id']!=resource or network['namespace_inode']!=meta.st_ino
                or network['namespace_device']!=meta.st_dev):
            raise ValueError('original protected network namespace required')
        expected=dict(action=action,candidate=source,
            namespace=dict(name=resource,device=meta.st_dev,inode=meta.st_ino))
        validate_restore_gate(gate,expected,self.lease.identity)
        from lifecycle_unload_scenario import wait_witness_generation
        def read():
            if ((namespace.lstat().st_dev,namespace.lstat().st_ino)!=(meta.st_dev,meta.st_ino)
                    or not original_process(source)):
                raise ValueError('candidate identity changed while awaiting MMDS observation')
            return namespace_witness(gate,min(10,remaining(deadline)))
        witness=wait_witness_generation(read,baseline,deadline)
        if (namespace.lstat().st_dev,namespace.lstat().st_ino)!=(meta.st_dev,meta.st_ino) or not original_process(source):
            raise ValueError('candidate identity changed during fixed read observation')
        receipt=dict(session=session,operation=accepted['key'],candidate=source,
            namespace=expected['namespace'],
            action_sha256=hashlib.sha256(json.dumps(action,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
            ram_nonce_sha256=hashlib.sha256(witness['nonce'].encode()).hexdigest(),
            activation_nonce_sha256=hashlib.sha256(witness['activation_nonce'].encode()).hexdigest(),
            guest_pid=witness['pid'],guest_start_time_ticks=witness['start_time_ticks'],
            counter=witness['counter'],metadata_generation=witness['metadata_generation'],
            public_authorization_proven=False)
        # Seal the observation before any caller releases the original gate.
        write_once(control/'restore-gate-observed.json',receipt)
        self._gate_files_unchanged()
        # Producer evidence, including the secret, remains private for exact
        # supervisor cleanup. No unlink/glob or generic evidence export here.
        return receipt

    def destroy_evidence(self,session,deadline):
        """Observe physical exit and exact adopted paths after public Cleaned.

        Historical metadata may remain for its original result-retention period;
        this does not require deleting that refusal frontier or old receipts.
        """
        remaining(deadline)
        original=self.runs[session]
        self.services.processes.stop('host-0')
        try:
            audit=self.services.processes.audit('host-0')
            local=original['local']
            rows=[row for row in audit['native'] if row['sandbox']==local['sandbox']]
            if (any(not row['terminal'] or row['current_vm'] is not None for row in rows)
                    or any(phase!='Free' for context,phase in audit['host_slots']
                           if context['binding']['sandbox']==local['sandbox'])
                    or original_process(original['source'])):
                raise ValueError('Destroy has not released the original running resources')
            paths=sorted(self.material_paths.get(session,set()))
            if any(Path(path).exists() or Path(path).is_symlink() for path in paths):
                raise ValueError('Destroy has not collected the exact original material paths')
            return dict(session=session,original_source_exited=True,position_released=True,
                adopted_material_paths_absent=paths,historical_metadata_may_be_retained=True)
        finally:
            self.services.processes.start('host-0')
            self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)

    def settlement_evidence(self,deadline):
        """Original accounting, with historical result retention left intact."""
        remaining(deadline)
        self.services.processes.stop('center')
        self.services.processes.stop('host-0')
        try:
            center=self.services.processes.audit('center')
            if center['reserved_resources']!={
                    'instances':0,'cpu':0,'memory_bytes':0,'disk_bytes':0}:
                raise ValueError('original center resource reservations remain after final Destroy')
            host=self.services.processes.audit('host-0')
            if any(phase!='Free' for _context,phase in host['host_slots']):
                raise ValueError('original running slot remains occupied after final Destroy')
            native=self.services.native_pools[0]
            database=private_path(native['database'])
            identity=(database.stat().st_dev,database.stat().st_ino)
            with closing(sqlite3.connect(database.as_uri()+'?mode=ro',uri=True,
                    timeout=min(2,remaining(deadline)))) as conn:
                conn.execute('BEGIN')
                sources={'records':checked_source_records(conn,self.services.host_configs[0],deadline)}
            if (database.stat().st_dev,database.stat().st_ino)!=identity:
                raise ValueError('original SQLite replaced during settlement audit')
            charges=[0 if record['capture'] is None else record['capture']['reserved_material_bytes']
                for record in sources['records']]
            pins=[False if record['restore'] is None else record['restore']['material_use']
                for record in sources['records']]
            if (any(type(value) is not int or value<0 for value in charges)
                    or any(type(value) is not bool for value in pins)):
                raise ValueError('complete original material accounting fields required')
            reserved=sum(charges)
            uses=sum(pins)
            if reserved!=0 or uses!=0:
                raise ValueError('original material charge or live backing pin was not settled')
            store=private_path(Path(self.captures[0]['layout_parent'])/'s',directory=True)
            if any(store.iterdir()):
                raise ValueError('original material/staging filesystem has not been collected')
            self.gc_complete=True
            return dict(center_reserved_resources=center['reserved_resources'],
                material_reserved_bytes=reserved,material_use_pins=uses,
                source_history_records=len(sources['records']),material_directory_empty=True,
                result_retention_not_bypassed=True)
        finally:
            self.services.processes.start('host-0')
            self.services.processes.start('center')
            self.services.wait_ready(timeout=min(remaining(deadline),900),for_admission=False)

    def restart_managers(self,deadline,*,original_unload=None):
        remaining(deadline)
        before=self.services.processes.observe()
        redis=self.services.processes.children['redis']
        self.services.processes.stop('center')
        self.services.processes.stop('host-0')
        operation_evidence=None
        if original_unload is not None:
            acceptance,unloaded=original_unload
            operation_evidence=original_unload_operation(
                self.services.processes.audit('center'),acceptance,unloaded)
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
        return dict(before=before,after=after,original_configs=self.configs,redis_preserved=True,
            original_unload_operation=operation_evidence)

    def finish(self,report):
        # Only product GC and original accounting success permit stopping
        # test managers. An unknown result retains exact resources and lease;
        # neither this path nor failure cleanup removes broad directories.
        if not report.get('complete') or not self.physical_complete or not self.gc_complete:
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
        report['activation_gate_cleanup']=self.cleanup_restore_gate()
        report['cleanup']='product-gc-settled-exact-runtime-free-managers-exited'
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
    if (re.fullmatch(r'[0-9a-f]{64}',identity) is None
            or quota['ci_state_root']!=str(args.ci_state_root)
            or not Path(quota['deployment_control']).is_relative_to(Path(quota['mount']))):
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
    quota=original_json(receipt_file,65536)
    deployment_file=private_path(args.deployment_receipt)
    deployment=original_json(deployment_file,65536)
    if bytes(checked_file(deployment_file,65536)['sha256']).hex()!=args.deployment_receipt_sha256:
        raise ValueError('static deployment differs from independently approved fingerprint')
    from lifecycle_unload_quota import validate_run_receipt
    from tools.capture_acceptance import validate_settings
    captures=validate_settings(captures,1)
    validate_run_receipt(quota,deployment,captures,args.deployment_receipt_sha256)
    control=private_path(quota['deployment_control'],directory=True)
    for name,digest in quota['raw_output_sha256'].items():
        if bytes(checked_file(control/(name+'.txt'),65536)['sha256']).hex()!=digest:
            raise ValueError('original per-run quota output changed')
    if (type(captures) is not list or len(captures)!=1 or set(fixture)!={
            'native','catalog','registry','activation_ceiling_bytes',
            'startup_timeout_ms','timeout_ms','max_lifetime_ms'}):
        raise ValueError('one exact current bounded lifecycle fixture required')
    # Prepared empty layout parent and quota readback predate all product processes. No
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
