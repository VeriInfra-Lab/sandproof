"""One fresh center/Redis/private-host assembly for real system probes.

Only deployment test inputs are constructed here. The ORIGINAL Rust envelope,
SQLite validation, physical profile and authenticated cache/status ports remain
the authorities. There is no old public-host parser or alternate execution mode.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name('_assertions_required.py'))
)['require_assertions']
_require_assertions()

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0,str(Path(__file__).resolve().parents[2]))
from _center_host_services import (Processes, identities, local_binding, render,
    redis_configuration, role_facts, stage_role_directories, stage_tls, write_role,test_credentials,process_facts,
    validate_redis_operation_timeout_ms)
from tools.service_storage_observation import ServiceStoragePeaks
from server_process_crash import fetch, free_loopback_port
from tools import ci_e2e_artifacts as artifacts
from tools.benchmark_ci import remove_private_tree
from tools.check_kvm_host import trusted_path
from tools.ci_e2e_prepare import contract_versions
from tools.ci_redis import check_material
from tools.system_acceptance import digest, json_file, private_path
from tools.template_physical_storage import envelope
from tools.capture_acceptance import instantiate_copy_limits, require_capacity, validate_settings
from tools.runtime_acceptance_budget import profile_declaration as _profile_declaration, service_declarations

MIB = 1024**2
MAX_ACTIVATION_BYTES = 200 * 1024**3
RETIRED_NATIVE_FIELDS = frozenset({'bind','domain','credentials','request_admission',
    'template_registry','internal_build_vms','build_retry_windows'})


def add_arguments(parser, *, required=True):
    for name in ('center','host','candidate-manifest','redis-material','tls-material',
                 'service-staging-parent','service-cgroup-record','capture-settings'):
        parser.add_argument('--' + name, type=Path, required=required)
    for name in ('center-uid','center-gid','redis-uid','redis-gid'):
        parser.add_argument('--' + name, type=int, required=required)
    parser.add_argument('--redis-operation-timeout-ms',type=_redis_operation_timeout_argument,
        default=1000,help='Explicit deployment-only Redis wait budget, 1..60000 ms (default: 1000)')
    parser.add_argument('--physical-inodes',type=int,
        help='explicit complete filesystem inode provision; cannot reduce the original declaration')


def _redis_operation_timeout_argument(value):
    try:
        return validate_redis_operation_timeout_ms(int(value))
    except (TypeError,ValueError) as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def _mkdir(path, identity):
    path = Path(path)
    path.mkdir(mode=0o700)
    os.chown(path, *identity)
    return path


def _copy(source, target, *, identity, executable=False, expected=None):
    """Copy a protected operator artifact, preserving sparse allocation only.

    Sparse blocks never reduce the declared logical-byte physical reservation.
    Both source/target fingerprints are checked. Existing targets are rejected.
    """
    source = trusted_path(Path(source))
    before = source.lstat()
    if (source.resolve(strict=True) != source or not stat.S_ISREG(before.st_mode)
            or before.st_nlink < 1 or before.st_uid != 0 or before.st_mode & 0o022
            or target.exists() or target.is_symlink()):
        raise ValueError('fresh copy of exact protected operator artifact required')
    checksum = digest(source)
    if expected is not None and checksum != expected:
        raise ValueError('operator artifact differs from its original receipt')
    subprocess.run(['/usr/bin/cp','--reflink=auto','--sparse=always','--no-clobber',str(source),str(target)],
                   check=True, timeout=900, env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})
    after = source.lstat()
    # Cargo legitimately links its protected release output to deps/. A second
    # name does not confer write permission; observe link count and content as
    # part of the original sealed source identity instead of refusing Cargo.
    fingerprint = lambda item:(item.st_dev,item.st_ino,item.st_nlink,item.st_size,item.st_mtime_ns,item.st_ctime_ns)
    if (fingerprint(before) != fingerprint(after) or target.lstat().st_nlink != 1
            or digest(target) != checksum):
        raise ValueError('original operator artifact changed during exact copy')
    os.chown(target,*identity)
    target.chmod(0o500 if executable else 0o400)
    return target



def _wait_redis(config,process):
    """Bounded fixed-test AUTH/PING; not a Redis health/authority replacement."""
    host,port=config['address'].rsplit(':',1)
    password=config['password'].encode()
    request=(b'*2\r\n$4\r\nAUTH\r\n$'+str(len(password)).encode()+b'\r\n'+password+
             b'\r\n*1\r\n$4\r\nPING\r\n')
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if process.poll() is not None:
            raise RuntimeError('original fixed Redis exited before initialization')
        try:
            with socket.create_connection((host,int(port)),timeout=1) as connection:
                connection.sendall(request)
                with connection.makefile('rb') as incoming:
                    if incoming.readline(32)==b'+OK\r\n' and incoming.readline(32)==b'+PONG\r\n':
                        return
        except OSError:
            pass
        time.sleep(.1)
    raise RuntimeError('fixed Redis startup unobserved; no center initialized')



def _bind_profile(native_pools, catalogs, layout, *, registry, service_peaks,
                  activation_ceiling_bytes=None, physical_inodes=None):
    """Validate the full original physical admission before any large copy."""
    profile=_profile_declaration(native_pools,catalogs,layout,registry=registry,
        service_peaks=service_peaks,activation_ceiling_bytes=activation_ceiling_bytes,
        physical_inodes=physical_inodes)
    peak,inodes=envelope(profile)
    free=os.statvfs(layout['hosts'][0])
    if free.f_bavail*free.f_frsize<2*peak or free.f_favail<2*inodes:
        raise RuntimeError('insufficient complete physical bytes/inodes; system probe not executed')
    for native in native_pools:
        native['bounded_host']['physical_storage']=copy.deepcopy(profile)
    return profile


def _provision_host(binary, control, config, record):
    """One fresh layout and epoch; errors retain the exact partial deployment."""
    # Fresh provisioning is explicit and occurs before the final role
    # configuration is written. serve/restart never repeats it.
    provision=control/'host-provision.json'
    write_role(provision,config,uid=0,gid=0)
    from tools.service_cgroup import prefix
    from tools.ci_e2e_prepare import bounded_json_command
    # Only this fresh-run preparation creates the layout. Restart uses
    # the same final role config and never re-enters this method.
    with (control/'capture-provision.log').open('xb') as output:
        os.chmod(control/'capture-provision.log',0o600)
        subprocess.run([*prefix(record),str(binary),
            'provision-capture-layout','--config',str(provision)],check=True,timeout=30,
            stdout=output,stderr=output,env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})
    if (control/'capture-provision.log').stat().st_size>65536:
        raise ValueError('capture provision output exceeds its finite evidence budget')
    for child in ('v','s'):
        private_path(Path(config['capture']['layout_parent'])/child,directory=True)
    registration=bounded_json_command([*prefix(record),str(binary),
        'provision-bootstrap','--config',str(provision)],
        {'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'},timeout=30,max_bytes=4096)
    if (set(registration)!={'version','boot_id','policy'} or registration['version']!=1
            or registration['boot_id']!=Path('/proc/sys/kernel/random/boot_id').read_text().strip()):
        raise ValueError('actual original bootstrap provisioning not confirmed')
    config['inbox']['host_epoch']=registration
    path=control/'host.json'
    write_role(path,config,uid=0,gid=0)
    return path


class RuntimeServices:
    """One owned process assembly; every restart uses its original configuration."""
    @classmethod
    def prepare(cls,args,root,native_pools,registered_catalogs,key,public_address,record,
                retained_sessions,private_builds=None,registry_config=None,extra_credentials=None,activation_ceiling_bytes=None):
        redis_operation_timeout_ms=validate_redis_operation_timeout_ms(args.redis_operation_timeout_ms)
        if os.geteuid()!=0 or not 1<=len(native_pools)<=2 or len(native_pools)!=len(registered_catalogs):
            raise ValueError('explicit isolated root host cohort required')
        if registry_config is None or any(RETIRED_NATIVE_FIELDS & native.keys() for native in native_pools):
            raise ValueError('new native inputs and explicit registry declaration required')
        if type(retained_sessions) is not int or not 1<=retained_sessions<=2**16-1:
            raise ValueError('explicit finite retained-session count required')
        test_credentials(key,extra_credentials)
        root=private_path(root,directory=True)
        pools=copy.deepcopy(native_pools)
        ids=identities(args.center_uid,args.center_gid,args.redis_uid,args.redis_gid,
                       uid_base=min(native['uid_base'] for native in pools))
        paths=dict(center=Path(args.center),host=Path(args.host))
        manifest=json_file(private_path(args.candidate_manifest),limit=16384)
        candidate=subprocess.check_output(['git','-C',str(Path(__file__).resolve().parents[2]),
            'rev-parse','HEAD'],text=True).strip()
        versions=contract_versions(paths['center'],{'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})
        if contract_versions(paths['host'],{'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})!=versions:
            raise ValueError('actual service metadata disagrees')
        manifest=artifacts.load(Path(args.candidate_manifest),candidate=candidate,
            paths={role:str(path) for role,path in paths.items()},versions=versions)
        captures=validate_settings(json_file(private_path(args.capture_settings),limit=65536),len(pools))
        captures=[instantiate_copy_limits(capture,native,catalog)
                  for capture,native,catalog in zip(captures,pools,registered_catalogs)]
        for native,capture in zip(pools,captures):
            pool_root=Path(native['database']).parent.parent
            parent=Path(capture['layout_parent'])
            if parent!=pool_root/'p' or parent/'v'!=Path(native['chroot_base']):
                raise ValueError('capture profile must bind the exact fresh runtime sibling')
            require_capacity(capture,native,retained_sessions)
        redis_receipt=check_material(Path(args.redis_material),protected=True)
        hosts=[]
        for native,capture in zip(pools,captures):
            host_root=Path(native['database']).parent.parent
            if host_root not in (root,root/'0',root/'1'):
                raise ValueError('explicit original host roots must belong to this fresh evidence root')
            hosts.append(private_path(host_root,directory=True))
            for path in (Path(native['database']).parent,Path(native['node_state_dir']),Path(capture['layout_parent'])):
                private_path(path,directory=True)
                if any(path.iterdir()):
                    raise ValueError('never adopt existing native history or physical resources')
        layout=stage_role_directories(args.service_staging_parent,'sp292-'+secrets.token_hex(16),hosts,
                                      service_ids=ids)
        layout.update(center_identity=ids['center'],host_controls=[],logs=root/'control-logs')
        layout['center_control']=_mkdir(layout['center']/'control',ids['center'])
        layout['registry']=_mkdir(layout['center']/'registry',ids['center'])
        layout['assets']=_mkdir(layout['center']/'assets',ids['center'])
        layout['writer']=_mkdir(layout['center']/'writer',ids['center'])
        _mkdir(layout['logs'],(0,0))
        if private_builds is not None:
            layout['build_control']=_mkdir(root/'build-control',(0,0))
        for path in hosts:
            layout['host_controls'].append(_mkdir(path/'control',(0,0)))
        for path in (layout['registry']/'inbox',layout['registry']/'artifacts'):
            _mkdir(path,ids['center'])
        role_names=('runtime',) if len(hosts)==1 else ('runtime','builder')
        tls_root=private_path(args.tls_material,directory=True)
        tls_receipt={name:digest(private_path(tls_root/name)) for name in artifacts.tls_material_files(role_names)}
        stage_tls(tls_root,tls_receipt,center=layout['center_control'],hosts=layout['host_controls'],
                  center_identity=ids['center'])
        copied={}
        source_checksums={}
        pending_copies=[]
        catalogs=copy.deepcopy(registered_catalogs)
        asset_bytes=0
        for catalog in catalogs:
            for binding in catalog['templates']:
                for kind in ('kernel','rootfs'):
                    source=Path(binding[kind]['path'])
                    if str(source) in source_checksums and source_checksums[str(source)]!=binding[kind]['sha256']:
                        raise ValueError('same operator artifact has inconsistent catalog fingerprints')
                    source_checksums[str(source)]=binding[kind]['sha256']
                    if str(source) not in copied:
                        target=layout['assets']/('artifact-'+str(len(copied)))
                        pending_copies.append((source,target,binding[kind]['sha256']))
                        copied[str(source)]=target
                        asset_bytes+=source.stat().st_size
                    binding[kind]['path']=str(copied[str(source)])
        registry=copy.deepcopy(registry_config)
        registry['root']=str(layout['registry'])
        center_bin=layout['center_control']/'sandproof-center'
        host_bins=[control/'sandproof-host' for control in layout['host_controls']]
        redis_bin=layout['redis']/'redis-server'
        service_peaks=service_declarations(layout,ids,paths,asset_bytes,redis_receipt,private_builds,captures)
        profile=_bind_profile(pools,catalogs,layout,registry=registry,service_peaks=service_peaks,
                              activation_ceiling_bytes=activation_ceiling_bytes,
                              physical_inodes=getattr(args,'physical_inodes',None))
        # Complete original byte/inode activation is checked BEFORE any large
        # disk copy. Rust checks the same final profile independently at init.
        for source,target,checksum in pending_copies:
            _copy(source,target,identity=ids['center'],expected=checksum)
        _copy(paths['center'],center_bin,identity=ids['center'],executable=True,
              expected=manifest['artifacts']['center']['sha256'])
        for binary in host_bins:
            _copy(paths['host'],binary,identity=(0,0),executable=True,
                  expected=manifest['artifacts']['host']['sha256'])
        _copy(Path(args.redis_material)/'redis-server',redis_bin,identity=ids['redis'],
              executable=True,expected=redis_receipt['binary_sha256'])
        for cache in profile['host_template_caches']:
            cache_root=_mkdir(cache['root'],(0,0))
            for part in ('inbox','artifacts'):
                _mkdir(cache_root/part,(0,0))
            write_role(cache_root/'.cache.lock',b'',uid=0,gid=0)
        coordination=Path(profile['coordination_file'])
        if not coordination.exists():
            write_role(coordination,b'',uid=0,gid=0)
        elif private_path(coordination).stat().st_size!=0:
            raise ValueError('original physical coordination already activated; never reprovision')
        password=secrets.token_hex(32)
        host_addresses=['127.0.0.1:'+str(free_loopback_port()) for _ in pools]
        redis_port,artifact_port=free_loopback_port(),free_loopback_port()
        private_boundary=None
        if private_builds is not None:
            if len(pools)!=2 or profile['public_build'] is None:
                raise ValueError('private construction requires the exact approved builder pool/workspace')
            private_key=secrets.token_hex(32).encode()
            center_key=layout['center_control']/'private-build.key'
            layout['build_key_file']=layout['build_control']/'private-build.key'
            write_role(center_key,private_key,uid=ids['center'][0],gid=ids['center'][1])
            write_role(layout['build_key_file'],private_key,uid=0,gid=0)
            layout['private_address']='127.0.0.1:'+str(free_loopback_port())
            layout['helper_backend_address']='127.0.0.1:'+str(free_loopback_port())
            private_boundary=dict(bind=layout['private_address'],backend_address=layout['helper_backend_address'],
                credential_file=str(center_key),request_timeout_ms=86400000,
                max_request_bytes=pools[1]['data_transfer']['file_upload_bytes'],
                max_response_bytes=16*MIB)
        center,host_configs=render(pools,registered_catalogs=catalogs,
            deployment='sp292-'+secrets.token_hex(16),center_root=layout['center_control'],
            host_tls_roots=[control/'tls' for control in layout['host_controls']],
            host_addresses=host_addresses,public_address=public_address,
            artifact_address='127.0.0.1:'+str(artifact_port),redis_address='127.0.0.1:'+str(redis_port),
            password=password,key=key,retained_sessions=retained_sessions,registry=registry,versions=versions,
            capture_settings=captures,private_builds=private_builds,private_build_boundary=private_boundary,extra_credentials=extra_credentials,
            redis_operation_timeout_ms=redis_operation_timeout_ms)
        center['redis']['writer_root']=str(layout['writer'])
        write_role(layout['center_control']/'token.key',secrets.token_bytes(32),uid=ids['center'][0],gid=ids['center'][1])
        center_path=layout['center_control']/'center.json'
        write_role(center_path,center,uid=ids['center'][0],gid=ids['center'][1])
        host_paths=[]
        for control,config in zip(layout['host_controls'],host_configs):
            host_paths.append(_provision_host(control/'sandproof-host',control,config,record))
        redis_path=layout['redis']/'redis.conf'
        write_role(redis_path,redis_configuration(layout['redis'],port=redis_port,password=password).encode(),
                   uid=ids['redis'][0],gid=ids['redis'][1])
        value=cls()
        value.native_pools,value.host_configs,value.center_config=pools,host_configs,center
        value.layout,value.key,value.sdk_url=layout,key,'http://'+public_address
        value.manifest,value.redis_receipt,value.tls_receipt=manifest,redis_receipt,tls_receipt
        value.manifest_sha256=digest(args.candidate_manifest)
        value.staging_identity={name:(layout[name].stat().st_dev,layout[name].stat().st_ino)
                                for name in ('common','center','redis')}
        value.staging_owners=dict(center=ids['center'][0],redis=ids['redis'][0])
        value.expected_templates={node['registration']['identity']['node_id']:
            {item['template_id']+'-'+item['revision'] for item in catalog['templates']}
            for node,catalog in zip(center['nodes'],catalogs)}
        value.processes=Processes(center=dict(binary=center_bin,config=center_path,uid=ids['center'][0],gid=ids['center'][1]),
            hosts=[dict(binary=binary,config=path,uid=0,gid=0) for binary,path in zip(host_bins,host_paths)],
            redis=dict(binary=redis_bin,config=redis_path,uid=ids['redis'][0],gid=ids['redis'][1]),
            record=record,logs=layout['logs'])
        value.service_storage=ServiceStoragePeaks(profile['service_data'])
        return value

    def start(self, *, host_envs=None):
        self.service_storage.start()
        self.processes.start('redis')
        # PING only proves the original fixed Redis process accepts its own
        # generated key. Center initialize then independently checks AOF/health.
        _wait_redis(self.center_config['redis'],self.processes.children['redis'])
        for index in range(len(self.native_pools)):
            name='host-'+str(index)
            self.processes.initialize(name)
            self.processes.start(name,env=(host_envs or {}).get(name))
        self.processes.initialize('center')
        self.processes.start('center')

    def wait_ready(self, *, timeout=None, for_admission=True):
        if type(for_admission) is not bool:
            raise ValueError('explicit test readiness purpose required')
        if timeout is None:
            # Only template observation uses the original deployment's download
            # budget. VM startup, lifetime, grading and supervisor bounds stay
            # unchanged. Polling never resets this absolute observation bound.
            timeout=max(config['artifact_source']['download_timeout_ms']
                        for config in self.host_configs)/1000
        if isinstance(timeout,bool) or not isinstance(timeout,(int,float)) or not 0<timeout<=86400:
            raise ValueError('finite positive readiness observation deadline required')
        deadline=time.monotonic()+timeout
        # The old finite health lease survives a role restart. Require one
        # ORIGINAL authenticated status round begun after this observation;
        # healthy_until is the original round's before + fixed health window.
        fresh_until_ms=int(time.time()*1000)+self.center_config['node_observation']['health_window_ms']
        opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
        required={node['registration']['identity']['node_id']:node for node in self.center_config['nodes']}
        while time.monotonic()<deadline:
            if any(process.poll() is not None for process in self.processes.children.values()):
                raise RuntimeError('original service exited before authenticated readiness')
            try:
                # A public response alone proves neither cache nor host health.
                listening=fetch(opener,self.sdk_url,'GET','/v2/sandboxes',self.key,
                    timeout=min(5,max(.001,deadline-time.monotonic())))[0]==200
                view=self.processes.center_readiness(timeout=min(10,max(.001,deadline-time.monotonic())))
                ready=nodes_ready(view,required,self.expected_templates,
                                 now_ms=int(time.time()*1000),for_admission=for_admission,
                                 min_healthy_until_ms=fresh_until_ms)
                if listening and ready:
                    self.processes.observe()
                    if time.monotonic()<deadline:
                        return view
            except (OSError,ValueError,RuntimeError,KeyError,TypeError,subprocess.SubprocessError):
                pass  # Missing/unknown observations cannot authorize a create.
            time.sleep(min(.2,max(0,deadline-time.monotonic())))
        raise RuntimeError('authenticated host/cache readiness unobserved; no sandbox creation attempted')

    def planned_audit(self,name,*,ready_deadline=None):
        """Explicit test stop/read/restart of one ORIGINAL authority.

        Never silently stop a role as part of local_for, never initialize on a
        restart, and never stop another healthy role for this observation.
        Recovery loops can require a fresh online round before their next
        offline audit, under the SAME absolute recovery and download bounds.
        Otherwise repeated stop/read/restart can starve cache restoration and
        the original terminal-confirmation exchange.
        """
        if name not in self.processes.children or self.processes.children[name].poll() is not None:
            raise RuntimeError('planned offline audit requires the original running role')
        if ready_deadline is not None:
            import math
            if (type(ready_deadline) not in (int,float) or not math.isfinite(ready_deadline)):
                raise ValueError('finite original recovery deadline required')
            remaining=ready_deadline-time.monotonic()
            if remaining<=0:
                raise TimeoutError('original recovery deadline expired before planned audit')
            download_bound=max(config['artifact_source']['download_timeout_ms']
                               for config in self.host_configs)/1000
            self.wait_ready(timeout=min(remaining,download_bound),for_admission=False)
            if time.monotonic()>=ready_deadline:
                raise TimeoutError('original recovery deadline expired before planned audit')
        self.processes.stop(name)
        try:
            return self.processes.audit(name)
        finally:
            self.processes.start(name)

    def execution(self,run_id,budget_path,*,helper=None):
        """Bind actual role files/PIDs; no scoring or cleanup verdict here."""
        observed=self.processes.observe()
        roles={}
        for name,role in self.processes.bindings.items():
            facts=role_facts(Path(role['config']).parent,role['config'],uid=role['uid'],gid=role['gid'])
            if name.startswith('host-'):
                roles.setdefault('host',[]).append(facts)
            else:
                roles[name]=facts
        if helper is not None:
            roles['host'].append(role_facts(helper['config'].parent,helper['config'],uid=0,gid=0))
            observed['host-helper']=process_facts(helper['process'],helper['binary'],helper['config'],
                self.processes.record,uid=0,gid=0,argv=helper['argv'])
        public_tls={name:value for name,value in self.tls_receipt.items() if not name.endswith('-key.der')}
        tls_sha=hashlib.sha256(json.dumps(public_tls,sort_keys=True,separators=(',',':')).encode()).hexdigest()
        manifest=artifacts.execution_manifest(self.manifest,run_id=run_id,
            candidate_manifest_sha256=self.manifest_sha256,roles=roles,redis=self.redis_receipt,
            tls_sha256=tls_sha,budget_sha256=digest(budget_path),
            service_data=self.service_storage.declaration())
        return manifest,observed

    def local_for(self,sandbox,*,index=0):
        config=self.host_configs[index]
        process=self.processes.children.get('host-'+str(index))
        if process is not None and process.poll() is None:
            raise RuntimeError('stop exact host before its original offline identity audit')
        return local_binding(self.processes.audit('host-'+str(index)),sandbox,
                             expected_node=config['inbox']['node'])['sandbox']

    def native_id_for(self,sandbox,*,index=0):
        return 'sb-'+self.native_pools[index]['namespace']+'-'+str(self.local_for(sandbox,index=index))

    def stop_all(self):
        try:
            self.processes.stop_all()
        finally:
            self.service_storage.stop()

    def storage_peaks(self):
        """Original finite service roots, including AOF/temp/logs; not a quota."""
        return self.service_storage.report()

    def storage_diagnostics(self):
        """Original sampler history only; cannot authorize staging removal."""
        return self.service_storage.diagnostics()

    def cleanup_staging(self):
        """Remove only this generated staging AFTER independent VM cleanup.

        Caller must already have confirmed the original native physical
        cleanup. This method proves only child exit and exact staging identity;
        it never establishes Cleaned, closes a window or removes any VM lease.
        """
        if (set(self.processes.children)!=set(self.processes.bindings)
                or any(child.poll() is None for child in self.processes.children.values())):
            raise RuntimeError('every original configured service must have an exact confirmed exit')
        common=self.layout['common']
        info=common.lstat()
        if ((info.st_dev,info.st_ino)!=self.staging_identity['common']
                or not stat.S_ISDIR(info.st_mode) or info.st_uid!=0 or info.st_mode&0o7777!=0o711
                or {child.name for child in common.iterdir()}!={'center','redis'}):
            raise ValueError('original generated staging parent changed; retain evidence')
        for name in ('center','redis'):
            directory=self.layout[name]
            original=directory.lstat()
            if ((original.st_dev,original.st_ino)!=self.staging_identity[name]
                    or original.st_uid!=self.staging_owners[name] or original.st_mode&0o7777!=0o700):
                raise ValueError('original generated service role changed; retain evidence')
        if not self.service_storage.closed:
            raise RuntimeError('original service storage sampler must exit before removal')
        self.storage_peaks()  # Never erase failed/unknown final capacity evidence.
        for name in ('center','redis'):
            remove_private_tree(self.layout[name],list(self.staging_identity[name]),
                                owner_uid=self.staging_owners[name])
        current=common.lstat()
        if (current.st_dev,current.st_ino)!=self.staging_identity['common']:
            raise ValueError('generated staging parent replaced during cleanup')
        common.rmdir()


def nodes_ready(audit,registered,templates,*,now_ms,for_admission=True,min_healthy_until_ms=0):
    """A bounded check of ORIGINAL persisted observations, not a health setter."""
    observed=audit.get('nodes')
    if type(observed) is not list or len(observed)!=len(registered):
        return False
    seen=set()
    for node in observed:
        node_id=node['identity']['node_id']
        original=registered.get(node_id)
        if node_id in seen:
            return False
        seen.add(node_id)
        wanted=templates[node_id] if node_id in templates else set()
        catalog=audit.get('catalogs',{}).get(node_id)
        if type(catalog) is not list:
            return False
        expected={item['revision']:item['digest'] for item in catalog}
        available={item['revision']:item['digest'] for item in node['templates']}
        if any(type(value) is not list or len(value)!=32
               or any(type(byte) is not int or not 0<=byte<=255 for byte in value)
               for value in [*expected.values(),*available.values()]):
            return False
        if (len(expected)!=len(catalog) or len(available)!=len(node['templates'])
                or not wanted or not wanted<=expected.keys()
                or any(available.get(revision)!=expected[revision] for revision in wanted)):
            return False
        if original is None:
            return False
        bounds=original['registration']['limit']
        free=node['free']
        if (node['identity']!=original['registration']['identity']
                or node['role']!=original['registration']['role'] or node['mode']!='accepting'
                or node['protocol_version']!=original['registration']['protocol_version']
                or type(node.get('retention_slots')) is not int
                or node['retention_slots']!=original['registration']['retention_slots']
                or node['healthy_until_ms']<=max(now_ms,min_healthy_until_ms) or set(free)!=set(bounds)
                or any(type(value) is not int or not 0<=value<=bounds[name]
                       or (for_admission and value==0) for name,value in free.items())
                ):
            return False
    return True
