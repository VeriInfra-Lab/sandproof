"""Read back one approved XFS material quota; never enable/change a quota.

Run only after independently reviewed first-deployment provisioning. Preserve
raw bounded tool outputs beside the no-clobber private receipt. This does not
start product services, authorize a VM, validate a snapshot or prove progress.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_runpy.run_path(str(_GuardPath(__file__).with_name('_assertions_required.py')))['require_assertions']()

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

PROJECT=63801
ENV={'PATH':'/usr/bin:/bin','LANG':'C'}


def project_hard_limit(text):
    # No -h/-H: xfs_quota report uses exact 1-KiB blocks (-b), or inode
    # counts (-i). Explicit numeric project (-n) avoids name-service aliases.
    matches=[]
    for line in text.splitlines():
        fields=line.split()
        if fields and fields[0] in (str(PROJECT),'#'+str(PROJECT)):
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



def readback(args):
    if os.geteuid()!=0:
        raise ValueError('protected deployment root required')
    os.umask(0o077)
    root=trusted_path(args.mount)
    mounted=root.stat()
    # Same traversable protected parent as original staged-role fixtures;
    # host evidence/runtime and each service role retain their private children.
    if not stat.S_ISDIR(mounted.st_mode) or mounted.st_uid!=0 or mounted.st_mode&0o7777!=0o711:
        raise ValueError('exact protected traversable deployment mount required')
    output=private_path(args.output,directory=True)
    if any(output.iterdir()):
        raise ValueError('fresh private readback evidence directory required')
    captures=json_file(private_path(args.capture_settings),limit=65536)
    if type(captures) is not list or len(captures)!=1:
        raise ValueError('one original protected capture profile required')
    deployment_path=private_path(args.deployment_receipt)
    deployment_raw=deployment_path.read_bytes()
    if len(deployment_raw)>65536 or hashlib.sha256(deployment_raw).hexdigest()!=args.deployment_receipt_sha256:
        raise ValueError('original installation receipt differs from independent approval')
    deployment=json.loads(deployment_raw)
    if (deployment['phase']!='layout-provisioned' or deployment['mount']!=str(root)
            or deployment['filesystem_uuid']!=args.filesystem_uuid
            or deployment['deployment_control']!='/var/lib/sp638u-control'
            or re.fullmatch(r'sp638u-[0-9a-f]{16}',deployment['host_lease_identity']) is None):
        raise ValueError('one exact approved installation and original HostLease required')
    for name in ('paused','exclusive-host'):
        if json_file(private_path(Path(deployment['ci_state_root'])/name))!={'e2e':deployment['host_lease_identity']}:
            raise ValueError('original deployment does not retain exclusive CI admission')
    settings=captures[0]['material_project']
    if settings['project_id']!=PROJECT or settings['hard_bytes']%1024:
        raise ValueError('fixed project and exact block-representable hard limit required')
    tool=trusted_path(Path('/usr/sbin/xfs_quota'))
    original=tool.stat()
    tool_hash=hashlib.sha256(tool.read_bytes()).hexdigest()
    if (tool_hash!=args.xfs_quota_sha256 or original.st_mode&0o022
            or not stat.S_ISREG(original.st_mode) or original.st_uid!=0 or not 0<original.st_size<=16*1024**2):
        raise ValueError('fixed quota tool differs from independent approval')
    mount=bounded_json_command(['/usr/bin/findmnt','--json','--target',str(root),
        '--output','TARGET,SOURCE,FSTYPE,OPTIONS,UUID'],ENV,timeout=10,max_bytes=16384)
    rows=mount['filesystems']
    if (len(rows)!=1 or rows[0]['target']!=str(root) or rows[0]['fstype']!='xfs'
            or rows[0]['source']!=deployment['source']):
        raise ValueError('one exact original private XFS mount required')
    view=rows[0]
    if not {'pquota','prjquota'}&set(view['options'].split(',')) or view['uuid']!=args.filesystem_uuid:
        raise ValueError('original filesystem identity or project-quota mount option differs')
    reports={name:capture(tool,command,root) for name,command in (
        ('state','state -p'),('blocks','report -p -b -N -n'),('inodes','report -p -i -N -n'))}
    state=reports['state']
    if (state.count('Project quota state on ')!=1 or str(root) not in state
            or state.count('Accounting: ON')!=1 or state.count('Enforcement: ON')!=1):
        raise ValueError('actual project accounting/enforcement not confirmed ON')
    hard_bytes=project_hard_limit(reports['blocks'])*1024
    hard_inodes=project_hard_limit(reports['inodes'])
    if hard_bytes!=settings['hard_bytes'] or hard_inodes!=settings['hard_inodes']:
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
    if dict(device=lock.st_dev,inode=lock.st_ino,owner_uid=lock.st_uid)!=deployment['native_lock_identity']:
        raise ValueError('original installation/native coordination identity differs')
    evidence={}
    for name,text in reports.items():
        path=output/(name+'.txt')
        with path.open('x') as stream:
            stream.write(text);stream.flush();os.fsync(stream.fileno())
        evidence[name]=hashlib.sha256(path.read_bytes()).hexdigest()
    receipt=dict(schema=1,filesystem_device=root.stat().st_dev,filesystem_uuid=view['uuid'],
        mount=str(root),source=view['source'],project_id=PROJECT,hard_bytes=hard_bytes,
        hard_inodes=hard_inodes,accounting=True,enforcement=True,actual_readback_verified=True,
        native_lock_identity=dict(device=lock.st_dev,inode=lock.st_ino,owner_uid=lock.st_uid),
        xfs_quota_sha256=tool_hash,raw_output_sha256=evidence,
        host_lease_identity=deployment['host_lease_identity'],ci_state_root=deployment['ci_state_root'],
        deployment_control=deployment['deployment_control'],
        deployment_receipt_sha256=args.deployment_receipt_sha256)
    write_once(output/'quota-receipt.json',receipt)
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('mount','capture-settings','native-lock','output','deployment-receipt'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--deployment-receipt-sha256',required=True)
    parser.add_argument('--filesystem-uuid',required=True)
    parser.add_argument('--xfs-quota-sha256',required=True)
    args=parser.parse_args()
    readback(args)
    print('original material quota readback complete; no VM/system scenario executed')


if __name__=='__main__':
    main()
