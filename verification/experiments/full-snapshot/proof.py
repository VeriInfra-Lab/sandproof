"""Replay real file-guard extraction and check every actual Lean source via the standard service."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import traceback

HERE=Path(__file__).resolve().parent
ROOT=HERE.parents[2]
MODEL=Path("SandboxModel/HostSnapshot/FilePolicy.lean")
LIBRARY=Path("verification/aeneas/full-snapshot")

def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output",type=Path,required=True)
    args=parser.parse_args()
    if sys.platform!="linux" or not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("fixed isolated formal Linux runtime required")
    frozen=json.loads((HERE/"policy-frozen.json").read_text())
    if sha(ROOT/"sandbox-runtime/src/snapshot_policy.rs")!=frozen["policy_source_sha256"]:
        raise ValueError("production guard differs")
    out=args.output.resolve();out.mkdir(parents=True,mode=0o700,exist_ok=False)
    start=time.monotonic();record={"complete":False,"files":[],"commands":[]}
    env=dict(os.environ,PYTHONDONTWRITEBYTECODE="1",LEAN_NUM_THREADS="4",LEAN_STACK_SIZE_KB="65536")
    def run(label,command,cwd=ROOT):
        began=time.monotonic();log=out/(label+".log")
        with log.open("xb") as stream:
            code=subprocess.run(command,cwd=cwd,env=env,stdout=stream,stderr=subprocess.STDOUT,timeout=1200).returncode
        record["commands"].append({"label":label,"exit_code":code,"seconds":time.monotonic()-began,"log_sha256":sha(log)})
        if code:raise ValueError(label+" failed")
    try:
        run("extraction",[sys.executable,str(HERE/"extract.py"),"--output",str(out/"extraction"),"--expected-source-sha256",frozen["policy_source_sha256"]])
        sys.path.insert(0,str(ROOT))
        from tools.check_foundations_extraction import llbc_digest
        for variant,expected in frozen["variants"].items():
            fresh=out/"extraction"/variant
            if llbc_digest(json.loads((fresh/"raw/actual.llbc").read_text()))!=llbc_digest(json.loads((ROOT/LIBRARY/variant/"original.llbc").read_text())):
                raise ValueError("fresh complete IR differs; preserve both originals")
            for relative,digest in expected["generated"].items():
                if "_Template" in relative:raise ValueError("guard must not depend on an extern template")
                if sha(fresh/"generated"/relative)!=digest or sha(ROOT/LIBRARY/variant/relative)!=digest:
                    raise ValueError("unmodified generated Lean differs")
        view=out/"proof-view";view.mkdir(mode=0o700)
        files=[ROOT/MODEL,*sorted((ROOT/LIBRARY).rglob("*.lean"))]
        for source in [*files,*[ROOT/n for n in ("SandboxModel/ProofAuditCommand.lean","lean-toolchain","lake-manifest.json","tools/lean_session.py","scripts/lean-mcp-server.sh")]]:
            target=view/source.relative_to(ROOT);target.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(source,target)
        shutil.copyfile(HERE/"lakefile.toml",view/"lakefile.toml")
        (view/".tools").symlink_to((ROOT/".tools").resolve(strict=True),target_is_directory=True)
        (view/".lake").mkdir();(view/".lake/packages").symlink_to((ROOT/".lake/packages").resolve(strict=True),target_is_directory=True)
        run("build",["lake","build","SandboxModel.HostSnapshot.FilePolicy"],view)
        spec=importlib.util.spec_from_file_location("snapshot_policy_real_service",ROOT/"verification/experiments/schema-domains/replay.py")
        protocol=importlib.util.module_from_spec(spec);spec.loader.exec_module(protocol)
        for index,source in enumerate(files):
            relative=str(source.relative_to(ROOT))
            actual=protocol.diagnostic(view,relative,out/("actual-file-"+str(index)),env)
            if sha(source)!=actual["source_sha256"]:raise ValueError("proof input changed")
            record["files"].append(actual)
        if sum(row["audits"] for row in record["files"])!=6:raise ValueError("six exact audits required")
        record["complete"]=True
    except BaseException as error:record.update(error=repr(error),traceback=traceback.format_exc())
    record["seconds"]=time.monotonic()-start;(out/"result.json").write_text(json.dumps(record,indent=2))
    print(json.dumps(record));raise SystemExit(0 if record["complete"] else 1)

if __name__=="__main__": main()
