#!/usr/bin/env bash
# Invoke inside the existing formal host wrapper. The materialized project
# and retained object inputs are separate from the original build directories.
set -euo pipefail
umask 077
[[ ${SANDPROOF_ACCEPTANCE_ROLE:-} == formal ]] || {
  printf '%s\n' 'Formal host role required' >&2
  exit 2
}
cd "$1"
[[ -r .lake/packages/aeneas/lean-toolchain ]] ||
  [[ -d .lake/packages/aeneas ]] || {
    printf '%s\n' 'Attach the approved fixed Lake package directory first' >&2
    exit 2
  }
export LEAN_NUM_THREADS=4 LEAN_STACK_SIZE_KB=65536
lake env python3 - <<'PY'
import hashlib
import json
import os
from pathlib import Path
import subprocess

root = Path.cwd()
plan = json.loads((root / "plan.json").read_bytes())
objects = root / "objects"
env = dict(os.environ)
env["LEAN_PATH"] = os.pathsep.join(filter(None, (str(objects), env.get("LEAN_PATH"))))
for name in plan["build_order"]:
    relative = Path(*name.split("."))
    obj = objects / relative.with_suffix(".olean")
    source = root / relative.with_suffix(".lean")
    if name in plan["retained"]:
        if hashlib.sha256(obj.read_bytes()).hexdigest() != plan["retained"][name]:
            raise ValueError(f"retained object changed: {name}")
        print(f"REUSE {name}", flush=True)
        continue
    if obj.exists():
        raise ValueError(f"unexpected existing object for {name}; use a new project")
    if os.statvfs(root).f_bavail * os.statvfs(root).f_frsize < 64 * 1024**3:
        raise RuntimeError("original 64 GiB proof floor")
    obj.parent.mkdir(exist_ok=True, parents=True)
    print(f"RUN {name}", flush=True)
    process = subprocess.run(["lean", f"--root={root}", "-o", str(obj), str(source)],
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             check=False, timeout=900)
    print(process.stdout.decode(errors="replace"), end="", flush=True)
    process.check_returncode()
    if name == plan["root"] and b"actual_first_work_uses_refound_world_record_and_sealed_plan: 3 standard logical dependencies" not in process.stdout:
        raise ValueError("root did not emit the expected strict audit")
PY
