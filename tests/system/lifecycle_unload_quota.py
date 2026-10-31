"""Read back the approved current XFS material projects; never enable/change a quota.

Run only after independently reviewed first-deployment provisioning. Preserve
raw bounded tool outputs beside the no-clobber private receipt. This does not
start product services, authorize a VM, validate a snapshot or prove progress.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import hashlib
import json
import os
import re
from pathlib import Path
import stat
import selectors
import time
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
if str(ROOT) not in sys.path:
    sys.path.insert(0,str(ROOT))
from tools.check_kvm_host import trusted_path
from tools.ci_e2e_prepare import bounded_json_command
from tools.system_acceptance import private_path,json_file,write_once

ENV={'PATH':'/usr/bin:/bin','LANG':'C'}


def project_hard_limit(text, project):
    # No -h/-H: xfs_quota report uses exact 1-KiB blocks (-b), or inode
    # counts (-i). Explicit numeric project (-n) avoids name-service aliases.
    matches=[]
    for line in text.splitlines():
        fields=line.split()
        if fields and fields[0] in (str(project),'#'+str(project)):
            if len(fields)<4 or any(not value.isdecimal() for value in fields[1:4]):
                raise ValueError('incomplete numeric original quota row')
            if int(fields[2])!=0:
                raise ValueError('expected fixed disabled soft limit')
            matches.append(int(fields[3]))
    if len(matches)!=1 or matches[0]<=0:
        raise ValueError('one exact nonzero project hard limit required')
    return matches[0]


def capture(tool,command,root):
    # Existing fixed tool only, no shell/user-supplied command program.
    process=subprocess.Popen([str(tool),'-x','-c',command,str(root)],stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=ENV)
    output=bytearray()
    deadline=time.monotonic()+10
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout,selectors.EVENT_READ)
            selector.register(process.stderr,selectors.EVENT_READ)
            while selector.get_map():
                remaining=deadline-time.monotonic()
                if remaining<=0:
                    raise TimeoutError('original quota readback deadline')
                events=selector.select(remaining)
                if not events:
                    raise TimeoutError('original quota readback deadline')
                for key,_ in events:
                    data=os.read(key.fileobj.fileno(),min(4096,65537-len(output)))
                    if not data:
                        selector.unregister(key.fileobj)
                    elif key.fileobj is process.stderr:
                        raise ValueError('quota diagnostic output is not successful readback')
                    else:
                        output.extend(data)
                        if len(output)>65536:
                            raise ValueError('quota readback exceeds original output bound')
        if process.wait(timeout=max(.001,deadline-time.monotonic()))!=0:
            raise ValueError('quota readback command failed')
        return output.decode('ascii')
    finally:
        process.stdout.close();process.stderr.close()
        if process.poll() is None:
            process.kill();process.wait(timeout=3)



def validate_deployment(deployment):
    """Static independently approved facts must not depend on a future run."""
    required={'schema_version','phase','mount','filesystem_device','source','filesystem_uuid',
              'projects','xfs_quota_sha256','native_lock_identity'}
    run_fields={'run_identity','host_lease_identity','ci_state_root',
                'quota_receipt_sha256','capture_settings_sha256'}
    if (type(deployment) is not dict or not required<=set(deployment)
            or run_fields&set(deployment) or type(deployment['schema_version']) is not int
            or deployment['schema_version']!=1 or deployment['phase']!='quota-provisioned'):
        raise ValueError('static provisioned quota receipt without run-dependent fields required')
    mount=Path(deployment['mount'])
    if (not mount.is_absolute() or '..' in mount.parts or str(mount)!=deployment['mount']
            or type(deployment['filesystem_device']) is not int or deployment['filesystem_device']<=0
            or any(type(deployment[key]) is not str or not deployment[key]
                   for key in ('source','filesystem_uuid'))
            or type(deployment['xfs_quota_sha256']) is not str
            or re.fullmatch(r'[0-9a-f]{64}',deployment['xfs_quota_sha256']) is None):
        raise ValueError('complete static filesystem and fixed quota tool identity required')
    projects=deployment['projects']
    if type(projects) is not list or not projects:
        raise ValueError('nonempty approved material project inventory required')
    ids=set()
    for row in projects:
        if (type(row) is not dict or set(row)!={'project_id','hard_bytes','hard_inodes'}
                or any(type(value) is not int or value<=0 for value in row.values())
                or row['project_id']>2**32-1 or row['hard_bytes']>=2**63
                or row['hard_inodes']>=2**63 or row['hard_bytes']%1024
                or row['project_id'] in ids):
            raise ValueError('distinct exact positive kernel project limits required')
        ids.add(row['project_id'])
    lock=deployment['native_lock_identity']
    if (type(lock) is not dict or set(lock)!={'device','inode','owner_uid'}
            or any(type(value) is not int or value<0 for value in lock.values())
            or lock['device']==0 or lock['inode']==0):
        raise ValueError('original native lock identity required')
    return deployment


def validate_run_receipt(receipt, deployment, captures, deployment_sha256):
    """Consume the independent kernel readback, never configuration self-attestation."""
    validate_deployment(deployment)
    projects=[{key:entry['material_project'][key] for key in ('project_id','hard_bytes','hard_inodes')}
              for entry in captures]
    if (receipt.get('schema_version')!=1
            or any(receipt.get(key)!=deployment[key] for key in
                   ('filesystem_device','filesystem_uuid','mount','source',
                    'native_lock_identity','xfs_quota_sha256'))
            or receipt.get('deployment_receipt_sha256')!=deployment_sha256
            or receipt.get('projects')!=projects or any(row not in deployment['projects'] for row in projects)
            or any(receipt.get(key) is not True for key in
                   ('accounting','enforcement','actual_readback_verified'))
            or re.fullmatch(r'[0-9a-f]{64}',receipt.get('host_lease_identity','')) is None):
        raise ValueError('exact approved projects and independent current kernel readback required')
    for key in ('ci_state_root','deployment_control'):
        path=Path(receipt.get(key,''))
        if not path.is_absolute() or '..' in path.parts or str(path)!=receipt[key]:
            raise ValueError('canonical private per-run evidence and admission paths required')
    hashes=receipt.get('raw_output_sha256',{})
    if (set(hashes)!={'state','blocks','inodes'}
            or any(type(value) is not str or re.fullmatch(r'[0-9a-f]{64}',value) is None
                   for value in hashes.values())):
        raise ValueError('complete original readback output digests required')
    return receipt


def readback(args):
    """Recheck the static deployment under this execution's original lease."""
    if os.geteuid()!=0 or re.fullmatch(r'[0-9a-f]{64}',args.run_identity) is None:
        raise ValueError('protected deployment root and exact current run identity required')
    os.umask(0o077)
    deployment_path=private_path(args.deployment_receipt)
    raw=deployment_path.read_bytes()
    if not 0<len(raw)<=65536 or hashlib.sha256(raw).hexdigest()!=args.deployment_receipt_sha256:
        raise ValueError('original independently approved deployment receipt changed')
    from tools.check_kvm_host import unique_keys
    deployment=validate_deployment(json.loads(raw,object_pairs_hook=unique_keys))
    root=trusted_path(Path(deployment['mount']))
    mounted=root.stat()
    if (not stat.S_ISDIR(mounted.st_mode) or mounted.st_uid!=0
            or mounted.st_mode&0o7777!=0o711 or mounted.st_dev!=deployment['filesystem_device']):
        raise ValueError('original protected quota mount changed')
    for name in ('paused','exclusive-host'):
        if json_file(private_path(args.ci_state_root/name))!={'e2e':args.run_identity}:
            raise ValueError('current original HostLease must be exclusive')
    output=private_path(args.output,directory=True)
    if (any(output.iterdir()) or output.stat().st_dev!=mounted.st_dev
            or output==root or not output.is_relative_to(root)
            or output==Path(deployment.get('deployment_control',''))):
        raise ValueError('fresh per-run readback directory on the original filesystem required')
    from tools.capture_acceptance import validate_settings
    captures=json_file(private_path(args.capture_settings),limit=65536)
    captures=validate_settings(captures,len(captures) if type(captures) is list else 0)
    projects=[{key:value['material_project'][key] for key in ('project_id','hard_bytes','hard_inodes')}
              for value in captures]
    if any(row['hard_bytes']%1024 or row not in deployment['projects'] for row in projects):
        raise ValueError('current projects must match approved exact kernel block limits')
    tool=trusted_path(Path('/usr/sbin/xfs_quota'))
    original=tool.stat();tool_hash=hashlib.sha256(tool.read_bytes()).hexdigest()
    if (tool_hash!=deployment['xfs_quota_sha256'] or original.st_mode&0o022
            or not stat.S_ISREG(original.st_mode) or original.st_uid!=0
            or not 0<original.st_size<=16*1024**2):
        raise ValueError('fixed quota tool differs from independent approval')
    rows=bounded_json_command(['/usr/bin/findmnt','--json','--target',str(root),
        '--output','TARGET,SOURCE,FSTYPE,OPTIONS,UUID'],ENV,timeout=10,max_bytes=16384)['filesystems']
    if (len(rows)!=1 or rows[0]['target']!=str(root) or rows[0]['fstype']!='xfs'
            or rows[0]['source']!=deployment['source']
            or rows[0]['uuid']!=deployment['filesystem_uuid']
            or not {'pquota','prjquota'}&set(rows[0]['options'].split(','))):
        raise ValueError('original filesystem identity or project quota mount changed')
    reports={name:capture(tool,command,root) for name,command in (
        ('state','state -p'),('blocks','report -p -b -N -n'),('inodes','report -p -i -N -n'))}
    state=reports['state']
    if (state.count('Project quota state on ')!=1 or str(root) not in state
            or state.count('Accounting: ON')!=1 or state.count('Enforcement: ON')!=1):
        raise ValueError('actual project accounting/enforcement not confirmed ON')
    for row in projects:
        if (project_hard_limit(reports['blocks'],row['project_id'])*1024!=row['hard_bytes']
                or project_hard_limit(reports['inodes'],row['project_id'])!=row['hard_inodes']):
            raise ValueError('actual kernel hard limits differ from protected capture budget')
    stamp=lambda m:(m.st_dev,m.st_ino,m.st_size,m.st_mtime_ns,m.st_ctime_ns,m.st_mode,m.st_uid,m.st_nlink)
    if stamp(tool.stat())!=stamp(original) or hashlib.sha256(tool.read_bytes()).hexdigest()!=tool_hash:
        raise ValueError('original quota tool changed during readback')
    descriptor=os.open(args.native_lock,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC)
    try:
        lock=os.fstat(descriptor)
        if not stat.S_ISREG(lock.st_mode) or lock.st_nlink!=1 or lock.st_mode&0o077:
            raise ValueError('original private native lock required')
    finally:
        os.close(descriptor)
    lock_identity=dict(device=lock.st_dev,inode=lock.st_ino,owner_uid=lock.st_uid)
    if lock_identity!=deployment['native_lock_identity']:
        raise ValueError('original installation/native coordination identity differs')
    evidence={}
    for name,text in reports.items():
        path=output/(name+'.txt')
        with path.open('x') as stream:
            stream.write(text);stream.flush();os.fsync(stream.fileno())
        evidence[name]=hashlib.sha256(path.read_bytes()).hexdigest()
    receipt=dict(schema_version=1,filesystem_device=mounted.st_dev,
        filesystem_uuid=deployment['filesystem_uuid'],mount=str(root),source=deployment['source'],
        projects=projects,accounting=True,enforcement=True,actual_readback_verified=True,
        native_lock_identity=lock_identity,xfs_quota_sha256=tool_hash,raw_output_sha256=evidence,
        host_lease_identity=args.run_identity,ci_state_root=str(args.ci_state_root),
        deployment_control=str(output),deployment_receipt_sha256=args.deployment_receipt_sha256)
    validate_run_receipt(receipt,deployment,captures,args.deployment_receipt_sha256)
    write_once(output/'quota-receipt.json',receipt)
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('capture-settings','native-lock','output','deployment-receipt','ci-state-root'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--deployment-receipt-sha256',required=True)
    parser.add_argument('--run-identity',required=True)
    readback(parser.parse_args())
    print('original material quota readback complete; no VM/system scenario executed')


if __name__=='__main__':
    main()
