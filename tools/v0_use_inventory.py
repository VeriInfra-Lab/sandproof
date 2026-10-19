"""Source census, not a Rust interpreter or a semantic call-graph proof.

Keep complete compiler output. The line index is only a way to find evidence:
it never resolves trait dispatch or invents cancellation/drop-glue semantics.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
CONFIG = "verification/use-inventory/config.json"
COMPILER = "2d8144b7880597b6e6d3dfd63a9a9efae3f533d3"
TARGET = "x86_64-unknown-linux-gnu"
ROOT_IDS = ["center", "host", "window-reducer", "guest", "vm-cli"]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path):
    def unique(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate key: " + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def write_json(path, data):
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")


def sources(root):
    names = subprocess.check_output(
        ["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    return {name: sha(root / name) for name in sorted(names) if name and (
        name in {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"}
        or (name.split("/", 1)[0].startswith(("sandbox-", "controller-"))
            and (name.endswith(".rs") or name.endswith("/Cargo.toml"))))}


def index_mir(path):
    """Index literal lines without rewriting or interpreting their operands."""
    lines = path.read_text().splitlines()
    if not lines or not lines[0].startswith("// WARNING: This output format"):
        raise ValueError("unreviewed MIR format")
    bodies, current = [], None
    for number, line in enumerate(lines, 1):
        if line and not line[0].isspace() and not line.startswith("//") and line != "}":
            if current is not None:
                raise ValueError("unterminated declaration")
            kind = next((k for k in ("fn", "const", "static", "alloc")
                         if line.startswith(k + " ") or (k == "alloc" and line.startswith(k))), None)
            if kind is None or not line.endswith("{"):
                raise ValueError("unreviewed top-level MIR declaration")
            current = dict(kind=kind, line=number, header=line, locals=[], terminators=[])
        elif current is not None:
            if line == "}":
                current["end"] = number
                current["sha256"] = hashlib.sha256(
                    ("\n".join(lines[current["line"] - 1:number]) + "\n").encode()).hexdigest()
                bodies.append(current)
                current = None
            elif re.match(r"\s+let (?:mut )?_\d+:", line):
                current["locals"].append(dict(line=number, text=line))
            elif " -> " in line:
                # Includes calls, drop, assert and switchInt. Do not classify
                # an unknown terminator as harmless or silently discard it.
                current["terminators"].append(dict(line=number, text=line))
    if current is not None or not any(b["kind"] == "fn" for b in bodies):
        raise ValueError("missing complete declarations")
    return dict(sha256=sha(path), bytes=path.stat().st_size, lines=len(lines), bodies=bodies)


def compiler_wrapper(args):
    # Cargo also asks rustc for target information; those invocations are not
    # compilation units. No flags, inputs or environment values are rewritten.
    if "--crate-name" not in args or not any(x.endswith(".rs") for x in args):
        return subprocess.call(args)
    output = Path(os.environ["SANDPROOF_V0_MIR_OUTPUT"])
    name = args[args.index("--crate-name") + 1]
    folder = Path(tempfile.mkdtemp(prefix=name + "-", dir=output))
    mir = folder / "complete.mir"
    actual = args + ["--emit=mir=" + str(mir)]
    result = subprocess.call(actual)
    write_json(folder / "command.json", dict(original=args, actual=actual, exit=result))
    return result


def option(args, name, default=None):
    return args[args.index(name) + 1] if name in args else default


def unit_data(record, output, before):
    command = read_json(record)
    args = command["original"]
    mir = record.parent / "complete.mir"
    if command["exit"] != 0 or command["actual"] != args + ["--emit=mir=" + str(mir)]:
        raise ValueError("failed or altered compiler invocation")
    source = [x for x in args if x.endswith(".rs")]
    if len(source) != 1:
        raise ValueError("ambiguous source input")
    source_path = Path(source[0])
    if source_path.is_absolute():
        source_path = source_path.relative_to(ROOT)
    if str(source_path) not in before:
        raise ValueError("untracked workspace input: " + str(source_path))
    features = sorted(args[i + 1] for i, arg in enumerate(args[:-1]) if arg == "--cfg")
    indexed = index_mir(mir)
    key = dict(crate=option(args, "--crate-name"), source=str(source_path), cfg=features,
               codegen=[args[i + 1] for i, a in enumerate(args[:-1]) if a == "-C"])
    unit = dict(**key, raw=mir.relative_to(output).as_posix(), command_sha256=sha(record),
                sha256=indexed["sha256"], bytes=indexed["bytes"], lines=indexed["lines"],
                functions=sum(b["kind"] == "fn" for b in indexed["bodies"]),
                declarations=len(indexed["bodies"]),
                local_types=sum(len(b["locals"]) for b in indexed["bodies"]),
                terminators=sum(len(b["terminators"]) for b in indexed["bodies"]),
                drop_functions=[b["header"] for b in indexed["bodies"]
                                if b["kind"] == "fn" and "::drop(" in b["header"]],
                drop_sites=sum(t["text"].lstrip().startswith("drop(")
                               for b in indexed["bodies"] for t in b["terminators"]))
    return unit, dict(**key, **indexed)


def check_build_coverage(output, receipt):
    metadata = read_json(output / "metadata.json")
    if sha(output / "metadata.json") != receipt["metadata_sha256"]:
        raise ValueError("changed Cargo metadata")
    members = {p["id"] for p in metadata["packages"]}
    if members != set(metadata["workspace_members"]):
        raise ValueError("workspace membership changed")
    used, packages = set(), set()
    for stage in receipt["stages"]:
        artifacts = []
        for line in (output / stage["log"]).read_text().splitlines():
            if not line.startswith("{"):
                continue  # Literal Cargo progress remains in the full log.
            message = json.loads(line)
            if message.get("reason") == "compiler-artifact" and message["package_id"] in members:
                artifacts.append(message)
        if not artifacts:
            raise ValueError("no own compiler artifacts for production root")
        for artifact in artifacts:
            source = Path(artifact["target"]["src_path"]).relative_to(ROOT).as_posix()
            profile = artifact["profile"]
            if profile["test"]:
                raise ValueError("test artifact cannot satisfy production census")
            matching = []
            for i, unit in enumerate(receipt["units"]):
                features = sorted(v[len('feature="'):-1] for v in unit["cfg"]
                                  if v.startswith('feature="') and v.endswith('"'))
                opt = next((x.split("=", 1)[1] for x in unit["codegen"]
                            if x.startswith("opt-level=")), "0")
                if (unit["source"] == source and features == sorted(artifact["features"])
                        and unit["crate"] == artifact["target"]["name"].replace("-", "_")
                        and opt == profile["opt_level"]):
                    matching.append(i)
            if not matching:
                raise ValueError("compiled own artifact has no complete MIR: " + source)
            used.update(matching)
            packages.add(artifact["package_id"])
    if used != set(range(len(receipt["units"]))) or packages != members:
        raise ValueError("missing or unexplained own compilation unit/package")


def capture(output):
    config = read_json(ROOT / CONFIG)
    if (config["target"] != TARGET or config["compiler_commit"] != COMPILER
            or [r["id"] for r in config["roots"]] != ROOT_IDS
            or config["profiles"] != ["dev", "release"]):
        raise ValueError("unreviewed compiler/configuration")
    before = sources(ROOT)
    if output.exists():
        raise ValueError("output already exists; observe the original run")
    output.mkdir(parents=True)
    rustc = subprocess.check_output(["rustc", "-vV"], text=True)
    if ("commit-hash: " + COMPILER not in rustc.splitlines()
            or "host: " + TARGET not in rustc.splitlines()):
        raise ValueError("fixed production compiler and Linux target required")
    (output / "rustc.txt").write_text(rustc)
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(output / "target"), CARGO_INCREMENTAL="0",
               RUSTC_WORKSPACE_WRAPPER=str(ROOT / "scripts/v0-mir-wrapper.sh"),
               SANDPROOF_V0_MIR_OUTPUT=str(output / "mir"))
    for profile in ("DEV", "RELEASE"):
        env["CARGO_PROFILE_" + profile + "_OPT_LEVEL"] = "0" if profile == "DEV" else "3"
        env["CARGO_PROFILE_" + profile + "_OVERFLOW_CHECKS"] = "true"
        env["CARGO_PROFILE_" + profile + "_DEBUG_ASSERTIONS"] = (
            "true" if profile == "DEV" else "false")
    (output / "mir").mkdir()
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--no-deps"],
        cwd=ROOT, env=env))
    members = {p["id"]: p for p in metadata["packages"]}
    if set(members) != set(metadata["workspace_members"]):
        raise ValueError("workspace membership changed")
    write_json(output / "metadata.json", metadata)
    stages = []
    for profile in config["profiles"]:
        for entry in config["roots"]:
            name = profile + "-" + entry["id"]
            command = ["cargo", "build", "--locked", "--offline", "--target", TARGET,
                       "--profile", profile, "--message-format=json"] + entry["cargo"]
            log = output / (name + ".jsonl")
            with log.open("wb") as stream:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream,
                                        stderr=subprocess.STDOUT)
            stage = dict(id=name, command=command, exit=result.returncode, log=log.name,
                         sha256=sha(log))
            stages.append(stage)
            write_json(output / "stages.json", stages)
            print(json.dumps(stage), flush=True)
            if result.returncode:
                raise ValueError("build failed: " + name + "; preserve " + str(log))
    if sources(ROOT) != before:
        raise ValueError("tracked production source changed during capture")
    units, full = [], []
    for record in sorted((output / "mir").glob("*/command.json")):
        unit, indexed = unit_data(record, output, before)
        units.append(unit)
        full.append(indexed)
    write_json(output / "index.json", full)
    receipt = dict(schema=1, source_commit=subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        compiler=rustc, config_sha256=sha(ROOT / CONFIG), sources=before,
        stages=stages, units=units, index_sha256=sha(output / "index.json"),
        metadata_sha256=sha(output / "metadata.json"))
    write_json(output / "receipt.json", receipt)
    check_build_coverage(output, receipt)
    return receipt


def check(output):
    receipt = read_json(output / "receipt.json")
    config = read_json(ROOT / CONFIG)
    if receipt["config_sha256"] != sha(ROOT / CONFIG) or receipt["sources"] != sources(ROOT):
        raise ValueError("source/configuration census is stale or incomplete")
    expected = [p + "-" + r["id"] for p in config["profiles"] for r in config["roots"]]
    if [s["id"] for s in receipt["stages"]] != expected:
        raise ValueError("missing, reordered or extra production root")
    for stage in receipt["stages"]:
        if stage["exit"] or sha(output / stage["log"]) != stage["sha256"]:
            raise ValueError("failed or changed build evidence")
    if sha(output / "index.json") != receipt["index_sha256"]:
        raise ValueError("changed full index")
    full = read_json(output / "index.json")
    if len(full) != len(receipt["units"]) or not full:
        raise ValueError("missing compiler unit")
    paths = [unit["raw"] for unit in receipt["units"]]
    if len(paths) != len(set(paths)):
        raise ValueError("duplicate compilation unit")
    for unit, indexed in zip(receipt["units"], full, strict=True):
        actual, actual_index = unit_data(
            (output / unit["raw"]).parent / "command.json", output, receipt["sources"])
        if actual != unit or actual_index != indexed:
            raise ValueError("changed or truncated complete MIR")
    check_build_coverage(output, receipt)
    # No dispatch resolution, coroutine destructor reconstruction or library
    # correctness claim is obtained by this evidence consistency check.
    return receipt


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "rustc":
        return compiler_wrapper(sys.argv[2:])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("capture", "check"))
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    result = capture(args.output.resolve()) if args.mode == "capture" else check(args.output.resolve())
    print(json.dumps(dict(units=len(result["units"]), sources=len(result["sources"]))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
