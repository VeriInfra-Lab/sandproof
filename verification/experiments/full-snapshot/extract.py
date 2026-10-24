"""Extract the actual production file guard and a compilable hard-link mutant."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
POLICY = Path("sandbox-runtime/src/snapshot_policy.rs")

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--expected-source-sha256", required=True)
    args = parser.parse_args()
    if sys.platform != "linux" or not os.environ.get("CARGO_TARGET_DIR"):
        raise ValueError("fixed isolated Linux formal runtime required")
    if sha(ROOT / POLICY) != args.expected_source_sha256:
        raise ValueError("actual production policy source differs")
    out = args.output.resolve()
    out.mkdir(mode=0o700, parents=True, exist_ok=False)
    sys.path.insert(0, str(ROOT))
    from tools.check_first_chain_extraction import check_tools
    tools = (ROOT / ".tools").resolve()
    check_tools(ROOT, tools)
    record = {"source_sha256": sha(ROOT / POLICY), "variants": {}, "complete": False}
    began = time.monotonic()
    try:
        for variant in ("correct", "allow-two-links"):
            case = out / variant
            case.mkdir()
            source = case / "source"
            shutil.copytree(ROOT, source, ignore=shutil.ignore_patterns(
                ".git", ".tools", ".lake", "target", "artifacts", "__pycache__"))
            path = source / POLICY
            original = path.read_text()
            if variant != "correct":
                before, after = "facts.links == 1", "facts.links <= 2"
                if original.count(before) != 1:
                    raise ValueError("actual hard-link guard missing or ambiguous")
                path.write_text(original.replace(before, after))
            raw = case / "raw"
            raw.mkdir()
            llbc = raw / "actual.llbc"
            namespace = "SandproofSnapshotPolicy" + ("Bad" if variant != "correct" else "")
            env = dict(os.environ, CARGO_INCREMENTAL="0",
                RUSTFLAGS="--remap-path-prefix=" + str(source) + "=.")
            commands = [
                ("charon", [str(tools / "aeneas/charon/bin/charon"), "cargo", "--preset=aeneas",
                    "--error-on-warnings", "--sysroot", "default", "--targets", "x86_64-unknown-linux-gnu",
                    "--start-from", "crate::snapshot_policy::accepts", "--include", "crate::snapshot_policy",
                    "--dest-file", "../raw/actual.llbc", "--", "--manifest-path", "sandbox-runtime/Cargo.toml",
                    "--lib", "--locked", "--offline", "--ignore-rust-version"]),
                ("aeneas", [str(tools / "aeneas/bin/aeneas"), "-backend", "lean", "-abort-on-error",
                    "-warnings-as-errors", "-split-files", "-emit-json", "-namespace", namespace + "Rust",
                    "-subdir", namespace, "-dest", str(case / "generated"), str(llbc)])]
            entry = {"policy_sha256": sha(path), "commands": []}
            record["variants"][variant] = entry
            for label, command in commands:
                log = raw / (label + ".log")
                started = time.monotonic()
                with log.open("xb") as stream:
                    code = subprocess.run(command, cwd=source, env=env, stdout=stream,
                        stderr=subprocess.STDOUT, timeout=900).returncode
                entry["commands"].append({"label": label, "exit_code": code,
                    "seconds": time.monotonic() - started, "command": command, "log_sha256": sha(log)})
                (out / "progress.json").write_text(json.dumps(record, indent=2))
                if code:
                    raise ValueError(variant + " " + label + " failed; this is not a property counterexample")
                if label == "charon":
                    tree = json.loads(llbc.read_text())
                    declarations = tree["translated"]["fun_decls"]
                    roots = [f for f in declarations if f and f["item_meta"]["is_local"]
                        and f["item_meta"]["name"][-1] == {"Ident": ["accepts", 0]}]
                    if tree.get("has_errors") is not False or len(roots) != 1 or not isinstance(roots[0]["body"], dict) or "Structured" not in roots[0]["body"]:
                        raise ValueError("actual transparent Rust guard required")
                    entry["llbc_sha256"] = sha(llbc)
                    entry["declaration_sha256"] = hashlib.sha256(json.dumps(roots[0], sort_keys=True).encode()).hexdigest()
            if sha(path) != entry["policy_sha256"]:
                raise ValueError("source changed during extraction")
            entry["generated"] = {str(p.relative_to(case / "generated")): sha(p)
                for p in sorted((case / "generated").rglob("*")) if p.is_file()}
        record["complete"] = True
    finally:
        record["seconds"] = time.monotonic() - began
        (out / "result.json").write_text(json.dumps(record, indent=2))
    print(json.dumps({"complete": True, "seconds": record["seconds"]}))

if __name__ == "__main__":
    main()
