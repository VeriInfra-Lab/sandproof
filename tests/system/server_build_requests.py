"""Opt-in real-KVM build inbox -> isolated worker -> publication -> SDK probe.

Synthetic offline Dockerfile fixture only. No public registry or model access.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request

from e2b import Sandbox
from sdk_server_smoke import options
from sdk_c02_acceptance import processes_in_uid_range, sha256_file
from server_process_crash import fetch, free_loopback_port, start_server, stop_server, wait_ready
from server_templates import descriptor
from server_vm_id_capacity import check_no_resources
from template_build import BUILDKIT_HASHES, FIXTURE, checked, upload

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.template_supervisor import Queue, run_one

MIB=1024*1024


def exercise(args):
    if os.geteuid()!=0 or sys.platform!="linux":
        raise RuntimeError("isolated root KVM host required")
    checkout=Path(__file__).resolve().parents[2]
    if subprocess.check_output(["git","-C",str(checkout),"status","--porcelain"]).strip():
        raise RuntimeError("clean exact commit required")
    commit=subprocess.check_output(["git","-C",str(checkout),"rev-parse","HEAD"],text=True).strip()
    if processes_in_uid_range(args.uid_base,args.uid_base+10):
        raise RuntimeError("UID range occupied")
    if any(p.is_dir() for p in args.cgroup_parent.iterdir()) or (args.cgroup_parent/"cgroup.procs").read_text().strip():
        raise RuntimeError("empty delegated group required")
    for name,digest in BUILDKIT_HASHES.items():
        assert sha256_file(args.tools/name)==digest,name
    inputs={str(p):sha256_file(p) for p in (args.kernel,args.rootfs,args.server)}
    root=Path(tempfile.mkdtemp(prefix="bq-",dir=args.scratch_parent))
    for name in ("db","state","jails","builds","registry","registry/inbox","registry/artifacts"):
        (root/name).mkdir(mode=0o700)
    namespace=secrets.token_hex(8)
    print(json.dumps({"root":str(root),"namespace":namespace,"commit":commit}),flush=True)
    keys={i:secrets.token_hex(32) for i in (1,2,99)}
    base=descriptor("build-base",args.kernel,args.rootfs,2,2048)
    base["owner"]=99
    url=f"http://127.0.0.1:{free_loopback_port()}"
    settings={"namespace":namespace,"database":str(root/"db/controller.db"),"node_state_dir":str(root/"state"),
        "jailer":str(args.jailer),"firecracker":str(args.firecracker),"chroot_base":str(root/"jails"),
        "kernel":str(args.kernel),"rootfs":str(args.rootfs),"uid_base":args.uid_base,"max_vm_id":10,
        "vcpu_count":2,"memory_mib":2048,
        "host_resources":{"cgroup_parent":str(args.cgroup_parent),"memory_max_bytes":2304*MIB},
        "file_policy_digest":1,"bind":url.removeprefix("http://"),"domain":"example.invalid",
        "credentials":[{"tenant":i,"secret":key} for i,key in keys.items()],
        "max_active_global":2,"max_active_per_tenant":2,
        "max_disk_bytes_global":24*1024*MIB,"max_disk_bytes_per_tenant":24*1024*MIB,
        "cleanup_reserve_bytes":256*MIB,"max_timeout_secs":1200,"startup_timeout_secs":120,"expiry_scan_seconds":2,
        "templates":{"templates":[base],"aliases":{"base":"build-base"},"compute_limits":{
            "max_vcpus_global":4,"max_vcpus_per_tenant":4,"max_memory_bytes_global":4608*MIB,"max_memory_bytes_per_tenant":4608*MIB}},
        "template_registry":{"root":str(root/"registry"),"max_templates_global":4,"max_templates_per_tenant":2,
            "max_artifact_bytes_global":6*1024*MIB,"max_artifact_bytes_per_tenant":3*1024*MIB,"validation_timeout_secs":120},
        "template_builds":{"root":str(root/"builds"),"max_jobs_global":4,"max_jobs_per_tenant":2,
            "max_input_bytes_global":64*MIB,"max_input_bytes_per_tenant":32*MIB,"max_log_bytes":MIB},
        "data_transfer":{"file_upload_bytes":512*MIB,"file_download_bytes":2*1024*MIB,
            "command_input_bytes":MIB,"command_output_bytes":4*MIB,"rpc_request_bytes":MIB,"rpc_response_bytes":MIB,
            "max_transfers":8,"max_transfers_per_vm":4,"control_reserve":1,
            "request_idle_seconds":180,"file_idle_seconds":180,"write_idle_seconds":180}}
    config=root/"config.json";config.write_text(json.dumps(settings));config.chmod(0o600)
    profile={name:{"path":str(args.tools/name),"sha256":digest} for name,digest in BUILDKIT_HASHES.items()}
    opener=urllib.request.build_opener(urllib.request.ProxyHandler({}))
    owner=options(keys[1],url,url);owner["request_timeout"]=180
    builder=options(keys[99],url,url);builder["request_timeout"]=180
    process=None;active=[];results=[]
    def no_resources():
        check_no_resources(root,namespace,args.firecracker,args.uid_base)
        assert not processes_in_uid_range(args.uid_base,args.uid_base+10)
        assert not any(p.is_dir() for p in args.cgroup_parent.iterdir())
    def submit(build_id,content):
        query=urllib.parse.urlencode({"build_id":build_id,"kind":"dockerfile","disk_mib":1024,
            "min_free_mib":64,"cpu_count":1,"memory_mib":256,"host_memory_mib":512})
        request=urllib.request.Request(url+"/sandproof/template-builds?"+query,data=content,method="POST",
            headers={"X-API-Key":keys[1],"Content-Type":"application/octet-stream"})
        with opener.open(request,timeout=60) as response:
            assert response.status==202
            return json.load(response)
    try:
        process=start_server(args.server,config,root/"server.log",None)
        wait_ready(opener,url,keys[1],process)
        fixture=Sandbox.create(timeout=600,allow_internet_access=False,**builder);active.append(fixture.sandbox_id)
        checked(fixture,"mkdir -p /input/tools")
        upload(fixture,args.tools/"busybox","/input/tools/busybox")
        fixture.files.write("/input/fixture.py",FIXTURE,user="root")
        checked(fixture,"python3 /input/fixture.py")
        source=bytes(fixture.files.read("/input/context.tar",format="bytes",user="root"))
        checked(fixture,"printf '\\nRUN exit 17\\n' >> /input/context/Dockerfile && tar -C /input/context -cf /input/failure.tar .")
        failure=bytes(fixture.files.read("/input/failure.tar",format="bytes",user="root"))
        assert len(source)<16*MIB and len(failure)<16*MIB
        assert fixture.kill();active.remove(fixture.sandbox_id);no_resources()
        for kind,content in (("success",source),("failure",failure)):
            build_id=secrets.token_hex(16)
            accepted=submit(build_id,content);assert accepted["state"]=="queued"
            assert submit(build_id,content)==accepted
            assert fetch(opener,url,"GET","/sandproof/template-builds/"+build_id,keys[2])[0]==404
            stop_server(process);process=start_server(args.server,config,root/"server.log",None)
            wait_ready(opener,url,keys[1],process)
            queue=Queue(root/"builds",MIB);job=queue.claim();assert job["id"]==build_id
            print("running isolated build "+kind,flush=True)
            run_one(queue,job,settings,profile,99,"build-base",url)
            no_resources()
            code,body=fetch(opener,url,"GET","/sandproof/template-builds/"+build_id,keys[1]);assert code==200
            view=json.loads(body);assert view["inputSha256"]==hashlib.sha256(content).hexdigest()
            code,body=fetch(opener,url,"GET","/sandproof/template-builds/"+build_id+"/logs?limit=65536",keys[1]);assert code==200
            logs=json.loads(body);assert logs["bytes"]
            assert submit(build_id,content)==view
            assert queue.claim() is None
            queue.db.close()
            if kind=="failure":
                assert view["state"]=="failed" and view["candidateId"] is None
                results.append({"kind":kind,"build_id":build_id,"failure":view["failure"],"log_bytes":len(logs["bytes"])});continue
            assert view["state"]=="candidate"
            request=urllib.request.Request(url+"/sandproof/template-publications",
                data=json.dumps({"candidateID":view["candidateId"]}).encode(),method="POST",
                headers={"X-API-Key":keys[1],"Content-Type":"application/json"})
            with opener.open(request,timeout=180) as response:
                assert response.status==200
                published=json.load(response)
            assert published["state"]=="ready",published
            no_resources()
            template=published["template"]["templateId"]
            guest=Sandbox.create(template=template,timeout=180,allow_internet_access=False,**owner);active.append(guest.sandbox_id)
            result=guest.commands.run("/bin/busybox cat /workspace/built && /bin/busybox cat /workspace/payload && test ! -e /workspace/removed && test ! -e /tmp/injected")
            assert result.exit_code==0 and "build-ok" in result.stdout and "copied-standard-context" in result.stdout
            guest.files.write("test.txt","build-inbox-sdk")
            assert guest.files.read("test.txt")=="build-inbox-sdk"
            assert guest.kill();active.remove(guest.sandbox_id);no_resources()
            results.append({"kind":kind,"build_id":build_id,"template_id":template,"log_bytes":len(logs["bytes"]),"sdk":"passed"})
        stop_server(process);process=None;no_resources()
        for path,digest in inputs.items():assert sha256_file(Path(path))==digest
        report={"commit":commit,"namespace":namespace,"root":str(root),"inputs":inputs,"results":results,"cleanup":"passed",
            "scope":"offline synthetic Dockerfile build submission, duplicate/restart, isolated supervisor, publication, SDK, failed build"}
        (root/"report.json").write_text(json.dumps(report,indent=2))
        print(json.dumps(report,indent=2),flush=True)
    finally:
        for sandbox_id in active:
            for key in (keys[1],keys[99]):
                try: Sandbox.kill(sandbox_id,**options(key,url,url))
                except Exception: pass
        stop_server(process)


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ("server","jailer","firecracker","kernel","rootfs","scratch-parent","cgroup-parent","tools"):
        parser.add_argument("--"+name,type=Path,required=True)
    parser.add_argument("--uid-base",type=int,required=True)
    exercise(parser.parse_args())
