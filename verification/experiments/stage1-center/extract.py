"""Fixed original-tool feasibility of actual center roots; not a proof result."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
HERE = ROOT / "verification/experiments/stage1-center"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def extract(output, profile, tool_root):
    output.mkdir(mode=0o700, parents=True)
    record = {"kind": "stage1-center-fixed-tool-preflight", "profile": profile, "steps": []}
    started = time.monotonic()
    inputs = [p for name in ("sandbox-center-core", "sandbox-protocol")
              for p in (ROOT / name).rglob("*.rs")]
    inputs += [HERE / "src/lib.rs", HERE / "Cargo.toml", HERE / "Cargo.lock",
               HERE / "extract.py", ROOT / "verification/foundations-environment.json"]
    record["inputs"] = {str(p.relative_to(ROOT)): digest(p) for p in sorted(inputs)}

    def run(name, argv, cwd):
        now = time.monotonic()
        with (output / (name + ".log")).open("xb") as log:
            result = subprocess.run(argv, cwd=cwd, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT, timeout=900)
        record["steps"].append({"name": name, "argv": argv, "cwd": str(cwd),
                                "exit_code": result.returncode,
                                "seconds": time.monotonic() - now,
                                "log_sha256": digest(output / (name + ".log"))})
        (output / "progress.json").write_text(json.dumps(record, indent=2) + "\n")
        if result.returncode:
            raise RuntimeError(name + " failed")

    try:
        expected = json.loads((ROOT / "verification/foundations-environment.json").read_text())
        charon = tool_root / "aeneas/charon/bin/charon"
        aeneas = tool_root / "aeneas/bin/aeneas"
        for name, path in (("charon", charon), ("aeneas", aeneas)):
            if digest(path) != expected[name]["binary_sha256"]:
                raise ValueError("fixed tool binary mismatch: " + name)
        record["tools"] = {name: expected[name] for name in ("charon", "aeneas")}
        raw = ROOT / ("artifacts/stage1-center/raw/" + profile + ".llbc")
        raw.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        if raw.is_symlink():
            raise ValueError("raw destination is a symlink")
        if raw.exists():
            shutil.copy2(raw, output / "previous.llbc")
            raw.unlink()
        destination = "../../../artifacts/stage1-center/raw/" + profile + ".llbc"
        root = {"version": "version_next", "replay": "replay", "accept": "accept"}[profile]
        run("charon", [str(charon), "cargo", "--preset=aeneas", "--error-on-warnings",
            "--sysroot", "default", "--include", "sandproof_center_core", "--include", "sandproof_protocol",
            "--start-from", "crate::" + root, "--dest-file", destination, "--", "--manifest-path",
            "Cargo.toml", "--lib", "--locked", "--offline", "--target", expected["target"]], HERE)
        if not raw.is_file():
            raise ValueError("tool did not generate the actual new raw file")
        shutil.copy2(raw, output / "original.llbc")
        ir = json.loads(raw.read_text())
        if ir.get("has_errors") is not False:
            raise ValueError("incomplete Charon artifact")
        record["llbc_sha256"] = digest(raw)
        generated = output / "generated"
        generated.mkdir()
        run("aeneas", [str(aeneas), "-backend", "lean", "-abort-on-error", "-warnings-as-errors",
            "-split-files", "-emit-json", "-namespace", "SandproofStage1CenterRust", "-subdir",
            "SandproofStage1Center", "-dest", str(generated), str(output / "original.llbc")], ROOT)
        record["generated"] = {str(p.relative_to(generated)): digest(p)
                               for p in sorted(generated.rglob("*")) if p.is_file()}
        if record["inputs"] != {str(p.relative_to(ROOT)): digest(p) for p in sorted(inputs)}:
            raise ValueError("source changed during extraction")
        record["success"] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record["seconds"] = time.monotonic() - started
    (output / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    return record


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--profile", choices=("version", "replay", "accept"), required=True)
    parser.add_argument("--tool-root", required=True, type=Path)
    args = parser.parse_args()
    result = extract(args.output, args.profile, args.tool_root)
    print(json.dumps({k: v for k, v in result.items() if k not in ("inputs", "generated")}, indent=2))
    raise SystemExit(0 if result["success"] else 1)
