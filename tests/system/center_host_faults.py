"""Mandatory same-candidate DK01–DK06 before the two-task promotion gate.

Run only through the approved isolated Linux/KVM supervisor. The outer test
launcher prepares ONE original service cgroup; the worker, fixed mTLS relay,
center, Redis and host inherit it. Original RuntimeServices owns every product
start/restart/audit. Failed or unknown runs retain their exact bounded evidence.
No paid agent, task network, public host shortcut or fallback PASS.
"""
import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()

import argparse
import importlib.metadata
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
from tools import ci_e2e_artifacts, service_cgroup
from tools.check_kvm_host import trusted_path
from tools.system_acceptance import digest, json_file, private_path, write_once

GROUP_SECONDS=7200


def parser():
    from _runtime_services import add_arguments
    value=argparse.ArgumentParser(description=__doc__)
    add_arguments(value)
    for flag in ('jailer','firecracker','kernel','rootfs','scratch-parent','cgroup-parent',
                 'service-cgroup-parent','service-profile','run-root','stale-probe'):
        value.add_argument('--'+flag,type=Path,required=True)
    value.add_argument('--stale-probe-sha256',required=True)
    value.add_argument('--uid-base',type=int,required=True)
    value.add_argument('--run-identity',required=True)
    value.add_argument('--rounds',type=int,required=True)
    value.add_argument('--wait-seconds',type=int,default=90)
    # Private test launcher protocol, NOT another product entry or deployment.
    value.add_argument('--worker',action='store_true',help=argparse.SUPPRESS)
    return value


def validate(args):
    if sys.platform!='linux' or os.geteuid()!=0:
        raise ValueError('isolated root Linux/KVM environment required')
    if importlib.metadata.version('e2b')!='2.51.0':
        raise ValueError('fixed offline SDK 2.51.0 required')
    if (not re.fullmatch(r'[a-z0-9-]{8,64}',args.run_identity)
            or not re.fullmatch(r'[a-f0-9]{64}',args.stale_probe_sha256)
            or type(args.rounds) is not int or args.rounds!=6
            or not 30<=args.wait_seconds<=120
            or not 1000<=args.uid_base<2**32-2):
        raise ValueError('fixed six-round fault-group identities/deadlines required')
    parent=private_path(args.scratch_parent,directory=True)
    if args.run_root.parent!=parent or args.service_cgroup_record!=args.run_root/'service-cgroup.json':
        raise ValueError('fresh exact run root and original service cgroup record required')
    for field in ('center','host','kernel','rootfs','jailer','firecracker','stale_probe'):
        trusted_path(getattr(args,field))
    if digest(args.stale_probe)!=args.stale_probe_sha256:
        raise ValueError('approved same-candidate original late-action helper changed')
    if args.cgroup_parent==args.service_cgroup_parent:
        raise ValueError('VM pool and service cgroup parents must remain distinct')
    # Original policy quantities, not a second capacity model. Physical disk
    # declaration is independently checked by the original renderer and Rust.
    profile=json_file(private_path(args.service_profile))
    if profile.get('schema_version')!=1 or set(profile)!={'schema_version','runtime','builder','host'}:
        raise ValueError('original approved acceptance service profile required')
    host=profile['host']
    if set(host)!={'control_cpu_allowance','cpu_headroom','control_memory_allowance_bytes','memory_headroom_bytes'}:
        raise ValueError('original finite service budget fields required')
    cpu,memory=host['control_cpu_allowance'],host['control_memory_allowance_bytes']
    if type(cpu) is not int or type(memory) is not int:
        raise ValueError('original numeric service cgroup budget required')
    service_cgroup.limits(cpu*100000,memory)
    return dict(cpu_quota_us=cpu*100000,memory_bytes=memory)


def bootstrap(args, argv, budget):
    if args.run_root.exists() or args.run_root.is_symlink():
        raise ValueError('never adopt or overwrite another fault-group run')
    args.run_root.mkdir(mode=0o700)
    private_path(args.run_root,directory=True)
    write_once(args.run_root/'dk-launch.json',dict(schema_version=1,run_identity=args.run_identity,
        candidate_manifest_sha256=digest(args.candidate_manifest),producer_sha256=digest(Path(__file__)),
        rounds=args.rounds,wait_seconds=args.wait_seconds,rootfs_sha256=digest(args.rootfs),
        kernel_sha256=digest(args.kernel),stale_probe_sha256=args.stale_probe_sha256))
    service_cgroup.prepare(args.service_cgroup_parent,'svc-'+os.urandom(16).hex(),
                           args.service_cgroup_record,**budget)
    command=[*service_cgroup.prefix(args.service_cgroup_record),sys.executable,str(Path(__file__)),*argv,'--worker']
    # Only this reviewed worker's bounded summary is emitted. Product logs are
    # declared/observed under the original service roots; pathname sampling is
    # not a hard log limit or a replacement for bounded deployment collection.
    with (args.run_root/'dk-worker.log').open('xb') as output:
        os.chmod(args.run_root/'dk-worker.log',0o600)
        process=subprocess.Popen(command,stdin=subprocess.DEVNULL,stdout=output,stderr=output,
            env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'},start_new_session=True)
        try:
            code=process.wait(timeout=GROUP_SECONDS)
        except (subprocess.TimeoutExpired,KeyboardInterrupt):
            # Signal only the owned worker; its handler invokes the same
            # original child drain. Unknown survivors keep the exact leaf.
            process.terminate()
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)
            raise
    if (args.run_root/'dk-worker.log').stat().st_size>64*1024:
        raise ValueError('bounded fault-worker evidence output exceeded')
    # A failed/unknown worker may have left a real VM in its separate native
    # cgroup. Preserve this exact service leaf/receipt so the original role
    # entries can be restarted for recovery; empty service membership alone
    # is not the original VM's terminal evidence.
    if type(code) is not int or code!=0:
        raise RuntimeError('mandatory fault group failed or unknown; retain original recovery identity')
    report=json_file(private_path(args.run_root/'report.json'),limit=4*1024**2)
    manifest=json_file(private_path(args.candidate_manifest),limit=16384)
    ci_e2e_artifacts.require_service_boundary(report,manifest)
    execution=ci_e2e_artifacts.require_execution_manifest(report.get('execution_manifest'),manifest,
        expected=json_file(private_path(args.run_root/'execution-manifest.json'),limit=32768))
    ci_e2e_artifacts.require_process_evidence(report.get('service_processes'),execution)
    ci_e2e_artifacts.require_dk(report['dk'],candidate=report['services']['candidate'])
    if (execution['run_id']!=args.run_identity
            or execution['candidate_manifest_sha256']!=digest(args.candidate_manifest)
            or report['dk']['execution_manifest_sha256']!=digest(args.run_root/'execution-manifest.json')):
        raise ValueError('successful DK does not bind the original execution/candidate receipts')
    # Only complete original success evidence permits removing this leaf;
    # the original primitive still independently refuses any live descendant.
    cleanup=service_cgroup.remove(args.service_cgroup_record)
    write_once(args.run_root/'service-cleanup.json',cleanup)
    return report


def worker(args):
    launch=json_file(private_path(args.run_root/'dk-launch.json'))
    expected=dict(schema_version=1,run_identity=args.run_identity,
        candidate_manifest_sha256=digest(args.candidate_manifest),producer_sha256=digest(Path(__file__)),
        rounds=args.rounds,wait_seconds=args.wait_seconds,rootfs_sha256=digest(args.rootfs),
        kernel_sha256=digest(args.kernel),stale_probe_sha256=args.stale_probe_sha256)
    if launch!=expected:
        raise ValueError('original test-launch intent mismatch; never reprovision')
    service_cgroup.observe(args.service_cgroup_record,pids=[os.getpid()])
    from _center_fault_harness import Harness
    from _center_fault_cases import dk01,dk02,dk03,dk04,dk05
    from server_bounded_churn import exercise_on_services
    harness=Harness(args)
    successful=False
    try:
        harness.start()
        slots=harness.services.host_configs[0]['inbox']['max_sandboxes']
        if args.rounds<slots+2:
            raise ValueError('configured churn must actually pass the retained slot count')
        for case in (dk01,dk02,dk03,dk04,dk05):
            case(harness)
        details=exercise_on_services(harness.services,harness.key,harness.foreign_key,
            evidence_root=harness.evidence_root,rounds=args.rounds,wait_seconds=args.wait_seconds,
            stale_probe=args.stale_probe,stale_probe_sha256=args.stale_probe_sha256)
        harness.complete_case('DK06',details)
        report=harness.finish()
        successful=True
    finally:
        harness.stop(successful=successful)
    write_once(args.run_root/'report.json',report)
    return report


def main(argv=None):
    argv=list(sys.argv[1:] if argv is None else argv)
    args=parser().parse_args(argv)
    budget=validate(args)
    if args.worker:
        def interrupted(_number,_frame):
            raise KeyboardInterrupt('owned fault worker interrupted')
        signal.signal(signal.SIGTERM,interrupted)
        return worker(args)
    return bootstrap(args,argv,budget)


def failure_summary(error):
    """Bounded source locations only, never exception text, locals or secrets."""
    frames=[]
    traceback=error.__traceback__
    while traceback is not None:
        code=traceback.tb_frame.f_code
        path=Path(code.co_filename)
        if (path.is_absolute() and path.is_relative_to(ROOT)
                and re.fullmatch(r'[a-zA-Z0-9_]+\.py',path.name)
                and re.fullmatch(r'[a-zA-Z0-9_]+|<module>',code.co_name)):
            frames.append(dict(file=path.name,line=traceback.tb_lineno,function=code.co_name))
            frames=frames[-12:]
        traceback=traceback.tb_next
    return dict(status='failed',failure_type=type(error).__name__,frames=frames)


if __name__=='__main__':
    try:
        report=main()
    except (Exception,KeyboardInterrupt) as error:
        # Do not print credentials, arbitrary errors, private paths or logs.
        print(json.dumps(failure_summary(error)),flush=True)
        raise SystemExit(1) from None
    print(json.dumps(dict(status='passed',candidate=report['services']['candidate'],
                         mandatory_cases=[case['id'] for case in report['dk']['cases']])),flush=True)
