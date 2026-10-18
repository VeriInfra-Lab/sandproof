"""Admit the complete frozen SQLite core/wrapper and real theorem diagnostics.

This checks correspondence inputs. It does not prove RefCell, rusqlite, erased
destructors, the full validators or the original/candidate program equivalence.
"""
import argparse
from collections import Counter
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

try:
    from tools.check_foundations_extraction import llbc_digest
    from tools.check_first_chain_extraction import check_tools
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest
    from check_first_chain_extraction import check_tools

ASSETS = "verification/experiments/sqlite-commit"
LIBRARY = "verification/aeneas/sqlite"
AUDIT = LIBRARY + "/SandproofSqlite/Audit.lean"
OWN = "sandproof_controller_service"


def hashed(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def nodes(value, path=()):
    if isinstance(value, dict):
        yield value, path
        for key, child in value.items():
            yield from nodes(child, path + (key,))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from nodes(child, path + (index,))


def name(item):
    return "::".join(part["Ident"][0] for part in item["item_meta"]["name"] if "Ident" in part)


def surface(llbc):
    translated = llbc["translated"]
    functions = {f["def_id"]: f for f in translated["fun_decls"] if f is not None}
    if len(functions) != sum(f is not None for f in translated["fun_decls"]):
        raise ValueError("duplicate function id")
    declarations = []
    for f in functions.values():
        declarations.append(dict(
            def_id=f["def_id"], name=f["item_meta"]["name"], label=name(f),
            opacity=f["item_meta"]["opacity"],
            body="Structured" if isinstance(f["body"], dict) else f["body"],
            src=f["src"], declaration_sha256=hashed(f),
        ))
    sites = []
    groups = {}
    for f in functions.values():
        for node, path in nodes(f["body"]):
            if "Drop" not in node:
                continue
            drop = node["Drop"]
            kind = drop["fn_ptr"]["kind"]
            target = functions[kind["Fun"]]["item_meta"]["name"] if "Fun" in kind else kind
            site = dict(function=f["def_id"], path=list(path), unwind="on_unwind" in path,
                        target=target, type=drop["place"]["ty"], drop_sha256=hashed(drop))
            sites.append(site)
            key = (f["def_id"], json.dumps(target, sort_keys=True), site["unwind"])
            groups.setdefault(key, []).append(site)
    return dict(
        declarations=declarations, drop_sites=len(sites), all_drop_sites_sha256=hashed(sites),
        drop_groups=[dict(function=key[0], target=json.loads(key[1]), unwind=key[2],
                          count=len(items), sites_sha256=hashed(items))
                     for key, items in groups.items()],
    )


def audit_surface(root):
    directory = root / LIBRARY / "SandproofSqlite"
    expected = set()
    for file in directory.glob("*.lean"):
        text = file.read_text()
        declarations = re.findall(r"^theorem ([A-Za-z0-9_]+)", text, re.M)
        if declarations:
            spaces = re.findall(r"^namespace ([A-Za-z0-9_.]+)$", text, re.M)
            if len(spaces) != 1:
                raise ValueError("unreviewed theorem namespace: " + file.name)
            expected.update(spaces[0] + "." + theorem for theorem in declarations)
    actual = re.findall(r"^#audit_controller_proof ([A-Za-z0-9_.]+)$", (root / AUDIT).read_text(), re.M)
    if not expected or len(actual) != len(set(actual)) or set(actual) != expected:
        raise ValueError("missing, duplicate or extra SQLite theorem audit")
    extension = '\n[[lean_lib]]\nname = "SandproofSqlite"\nsrcDir = "verification/aeneas/sqlite"\n'
    if (root / "lake-sqlite.toml").read_text().rstrip() != (
            (root / "lake-first-chain.toml").read_text().rstrip() + "\n" + extension).rstrip():
        raise ValueError("SQLite Lake view must preserve the full Linux/base configuration")
    return expected


def check_pair(root, core, wrapper):
    registry = read_json(root / ASSETS / "environment.json")
    profile = read_json(root / ASSETS / "candidate.json")
    for role, data in (("core", core), ("wrapper", wrapper)):
        if data.get("has_errors") is not False:
            raise ValueError("missing/erroneous " + role + " output")
        translated = data["translated"]
        if translated["options"] != registry[role]["charon_options"]:
            raise ValueError("changed " + role + " options")
        if {x["key"] for x in translated["target_information"]} != {profile["target"]}:
            raise ValueError("changed target")
        if surface(data) != registry[role]["surface"]:
            raise ValueError("unreviewed " + role + " declaration/Drop/exception surface")
        if llbc_digest(data) != registry[role]["llbc_digest"]:
            raise ValueError("complete " + role + " AST changed")
        own = [f for f in translated["fun_decls"] if f and name(f).startswith(OWN + "::")]
        if any(not isinstance(f["body"], dict) for f in own):
            raise ValueError("owned SQLite body hidden")
    inventory = read_json(root / LIBRARY / "translation.json")
    if inventory != registry["translation"]:
        raise ValueError("generated function/type/trait inventory changed")
    functions = inventory["functions"]
    if [f["rust_name"] for f in functions if f["is_opaque"]] != [
            "core::result::{core::result::Result<T, E>}::is_err"]:
        raise ValueError("unreviewed opaque core call")
    for item in functions:
        text = (root / LIBRARY / ("SandproofSqlite/FunsExternal.lean" if item["is_opaque"]
                                  else "SandproofSqlite/Funs.lean")).read_text()
        lean = item["lean_name"].removeprefix("SandproofSqliteRust.")
        if not re.search(r"\bdef\s+" + re.escape(lean) + r"(?![\w.])", text):
            raise ValueError("generated or modeled declaration missing")
    for relative, expected in profile["source_files"].items():
        if sha(root / relative) != expected:
            raise ValueError("production input changed: " + relative)
    if sha(root / ASSETS / "candidate.patch") != profile["patch_sha256"]:
        raise ValueError("candidate patch changed")
    if sha(root / ASSETS / "wrapper-roots.rs") != profile["wrapper_probes_sha256"]:
        raise ValueError("wrapper probe changed")
    return audit_surface(root)


def check_libraries(root, tool_root):
    check_tools(root, tool_root)
    registry = read_json(root / ASSETS / "environment.json")
    backend = root / ".lake/packages/aeneas"
    if subprocess.check_output(["git", "-C", str(backend), "status", "--porcelain",
                                "--untracked-files=no"], text=True):
        raise ValueError("Lean backend tracked sources changed")
    charon = tool_root / "aeneas/charon/bin/charon"
    production = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    extraction = subprocess.check_output([str(charon), "toolchain-path"], text=True).strip()
    for role, sysroot in (("production", production), ("extraction", extraction)):
        for relative, expected in registry["library_sources"][role].items():
            if sha(Path(sysroot) / "lib/rustlib/src/rust/library" / relative) != expected:
                raise ValueError("standard-library source changed: " + role + "/" + relative)
    cargo_home = Path(os.environ["CARGO_HOME"])
    for relative, expected in registry["registry_sources"].items():
        files = list((cargo_home / "registry/src").glob("*/" + relative))
        if len(files) != 1 or sha(files[0]) != expected:
            raise ValueError("SQLite/rusqlite source changed: " + relative)


def proof_sources_digest(root):
    fingerprint = hashlib.sha256()
    sources = sorted((root / "SandboxModel").rglob("*.lean"))
    sources += sorted((root / "verification/aeneas").rglob("*.lean"))
    for source in sources:
        relative = str(source.relative_to(root)).encode()
        fingerprint.update(len(relative).to_bytes(8, "big"))
        fingerprint.update(relative)
        fingerprint.update(hashlib.sha256(source.read_bytes()).digest())
    return fingerprint.hexdigest()


def check_record(root, record):
    expected = audit_surface(root)
    digest = sha(root / AUDIT)
    request = record["request"]
    if (request.get("tool") != "lean_diagnostic_messages"
            or request.get("arguments") != {"file_path": AUDIT}
            or request.get("expected_sha256") != digest):
        raise ValueError("exact unfiltered physical SQLite Audit request required")
    envelope = record["result"]
    if set(envelope) != {"result"}:
        raise ValueError("service envelope contains an error or unreviewed content")
    result = envelope["result"]
    revision = record["revision"]
    versions = read_json(root / ASSETS / "environment.json")["service_versions"]
    if any(revision.get(key) != value for key, value in versions.items()):
        raise ValueError("Lean service/client version changed")
    if (record.get("stale") is not False or result.get("success") is not True
            or result.get("timed_out") is not False or result.get("partial") is not False
            or result.get("still_elaborating_lines") not in (None, [])
            or result.get("failed_dependencies") not in (None, [])
            or revision["file_sha256"] != digest
            or revision["sources_sha256"] != proof_sources_digest(root)
            or revision["toolchain"] != (root / "lean-toolchain").read_text().strip()
            or revision["lakefile_sha256"] != sha(root / "lakefile.toml")
            or revision["manifest_sha256"] != sha(root / "lake-manifest.json")):
        raise ValueError("stale/incomplete/unsuccessful service result or different project")
    items = result["items"]
    if any(x["severity"] in {"error", "warning"} or "unsolvedGoals" in (x.get("lean_tags") or [])
           for x in items):
        raise ValueError("service reported an error, warning or unsolved goal")
    found = [re.match(r"^Checked ([A-Za-z0-9_.]+): [0-3] standard logical dependencies$", x["message"])
             for x in items]
    names = [m.group(1) for m in found if m]
    if len(names) != len(expected) or set(names) != expected:
        raise ValueError("service omitted or duplicated an actual theorem dependency audit")
    return len(expected)


def check_service(root, path, mutations):
    records = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            record = json.loads(line)
            if record.get("request", {}).get("arguments", {}).get("file_path") == AUDIT:
                records.append(record)
    if len(records) != 1:
        raise ValueError("one actual physical Audit service record required")
    count = check_record(root, records[0])
    if mutations:
        changes = [
            lambda r: r.update(stale=True),
            lambda r: r["revision"].update(server_version="changed"),
            lambda r: r["revision"].update(client_version="changed"),
            lambda r: r["revision"].update(mcp_version="changed"),
            lambda r: r["request"].update(tool="lean_run_code"),
            lambda r: r["request"]["arguments"].update(severity="info"),
            lambda r: r["result"].update(isError=True),
            lambda r: r["result"]["result"].update(partial=True),
            lambda r: r["result"]["result"].update(timed_out=True),
            lambda r: r["result"]["result"].update(success=False),
            lambda r: r["revision"].update(sources_sha256="changed"),
            lambda r: r["revision"].update(file_sha256="changed"),
            lambda r: r["result"]["result"]["items"].pop(),
            lambda r: r["result"]["result"]["items"].append(copy.deepcopy(r["result"]["result"]["items"][0])),
            lambda r: r["result"]["result"]["items"][0].update(severity="warning"),
        ]
        for change in changes:
            bad = copy.deepcopy(records[0])
            change(bad)
            try:
                check_record(root, bad)
            except (ValueError, KeyError, TypeError, IndexError):
                continue
            raise ValueError("invalid service receipt accepted")
    return count


def check_mutations(root, core, wrapper):
    variants = []
    for role, source in (("core", core), ("wrapper", wrapper)):
        bad = copy.deepcopy(source)
        f = next(f for f in bad["translated"]["fun_decls"] if f and name(f).startswith(OWN + "::"))
        f["body"] = "Opaque"
        variants.append((role, bad))
        bad = copy.deepcopy(source)
        f = next(f for f in bad["translated"]["fun_decls"]
                 if f and any("Drop" in n for n, _ in nodes(f["body"])))
        next(n for n, _ in nodes(f["body"]) if "Drop" in n)["Drop"]["kind"] = "Unconditional"
        variants.append((role, bad))
        bad = copy.deepcopy(source)
        bad["translated"]["options"]["start_from"] = []
        variants.append((role, bad))
    for role, bad in variants:
        try:
            check_pair(root, bad if role == "core" else core, bad if role == "wrapper" else wrapper)
        except (ValueError, KeyError, TypeError, IndexError):
            continue
        raise ValueError("missing root/owned body/Drop mutation accepted")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path)
    parser.add_argument("--service-log", type=Path)
    parser.add_argument("--check-mutations", action="store_true")
    args = parser.parse_args()
    try:
        core = read_json(args.evidence / "sandproof_sqlite_core.llbc")
        wrapper = read_json(args.evidence / "sandproof_sqlite_wrapper.llbc")
        expected = check_pair(args.root, core, wrapper)
        if args.tool_root:
            check_libraries(args.root, args.tool_root)
        if args.check_mutations:
            check_mutations(args.root, core, wrapper)
        if args.service_log:
            check_service(args.root, args.service_log, args.check_mutations)
    except (OSError, ValueError, KeyError, TypeError, IndexError, subprocess.SubprocessError) as error:
        print("SQLite admission failed: " + str(error), file=sys.stderr)
        return 1
    print(f"SQLite: complete core/wrapper ASTs and all Drop sites admitted; {len(expected)} theorem audits.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
