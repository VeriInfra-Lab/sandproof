"""Admit the fixed BTreeSet normal-value interface and exact proof receipts.

This checks sources and evidence, not the correctness of Rust Std or Aeneas.
Only the registered Global / str / (str,u16) uses may use this A6 instance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

try:
    from .check_foundations_extraction import llbc_digest
    from .check_loop_callbacks_contract import foreign_surface
except ImportError:
    from check_foundations_extraction import llbc_digest
    from check_loop_callbacks_contract import foreign_surface

ROOT = Path(__file__).resolve().parents[1]
EXPERIMENT = "verification/experiments/btree-set-binding/model"
ENVIRONMENT = EXPERIMENT + "/environment.json"
FREEZE = EXPERIMENT + "/inputs.sha256"
SOURCE = "verification/extract-btree-set-binding/src/lib.rs"
MANIFEST = "verification/extract-btree-set-binding/Cargo.toml"
LLBC = "verification/aeneas/llbc/sandproof_btree_set_binding.llbc"
GENERATED = "verification/aeneas/btree-set-binding"
PROOFS = "SandboxModel/Foundations/BtreeSetBinding"
ROOT_NAMES = ("insert_u64", "insert_str", "insert_lane", "repeat_str", "new_then_repeat_str")
POSITIVE = ["SandboxModel.Foundations.BtreeSetBinding.byte_compare_eq_iff","SandboxModel.Foundations.BtreeSetBinding.str_cmp_law","SandboxModel.Foundations.BtreeSetBinding.reference_cmp_law","SandboxModel.Foundations.BtreeSetBinding.member_by_value","SandboxModel.Foundations.BtreeSetBinding.insert_value","SandboxModel.Foundations.BtreeSetBinding.pair_cmp_law","SandboxModel.Foundations.BtreeSetBinding.inserted_members","SandboxModel.Foundations.BtreeSetBinding.inserted_valid","SandboxModel.Foundations.BtreeSetBinding.contract_from_value","SandboxModel.Foundations.BtreeSetBinding.insert_str_value","SandboxModel.Foundations.BtreeSetBinding.insert_str_contract","SandboxModel.Foundations.BtreeSetBinding.u16_cmp_law","SandboxModel.Foundations.BtreeSetBinding.insert_lane_value","SandboxModel.Foundations.BtreeSetBinding.insert_lane_contract","SandboxModel.Foundations.BtreeSetBinding.inserted_has_key","SandboxModel.Foundations.BtreeSetBinding.inserted_idempotent","SandboxModel.Foundations.BtreeSetBinding.repeat_str_value","SandboxModel.Foundations.BtreeSetBinding.new_then_repeat_str_value","SandboxModel.Foundations.BtreeSetBinding.sample_keys_distinct","SandboxModel.Foundations.BtreeSetBinding.nonempty_fresh_accepts","SandboxModel.Foundations.BtreeSetBinding.original_duplicate_keeps_view","SandboxModel.Foundations.BtreeSetBinding.lane_slot_distinguishes_keys"]
NEGATIVE = ["SandboxModel.Foundations.BtreeSetBinding.Negative.actual_inverted_empty",
            "SandboxModel.Foundations.BtreeSetBinding.Negative.actual_inverted_rejected"]
SCOPE = {
    "allocator": "Global",
    "key_types": ["&str", "(&str,u16)"],
    "control_type_only": "u64",
    "value_domain": "valid finite set, safe stable UTF-8 key borrows, representable values",
    "normal_allocation": "normal successful allocation only",
    "drop": "ordinary memory release only; no custom observable key/allocator Drop",
    "exception": "no panic/OOM/abort rollback or physical-release guarantee",
    "comparison": "fixed str byte order, reference forwarding, lexicographic tuple/U16",
    "excluded": ["iteration order", "tree shape", "addresses", "comparison trace",
                 "physical RAM bound", "unconditional termination", "whole validators"],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mutate(source, variant):
    if variant == "correct":
        return source
    if variant != "invert-flag":
        raise ValueError("Unknown BTreeSet variant")
    before = ("pub fn insert_str<'a>(seen: &mut BTreeSet<&'a str>, value: &'a str) -> bool {\n"
              "    seen.insert(value)\n}")
    after = before.replace("    seen.insert(value)", "    !seen.insert(value)")
    if source.count(before) != 1:
        raise ValueError("Ambiguous original BTreeSet insert_str delegate")
    return source.replace(before, after)


def check(root=ROOT):
    root = Path(root).resolve()
    env = json.loads((root / ENVIRONMENT).read_text())
    if env["kind"] != "fixed-Std-BTreeSet-normal-value-binding" or env["scope"] != SCOPE:
        raise ValueError("BTreeSet scope or trust boundary changed")
    rows = (root / FREEZE).read_text().splitlines()
    names = []
    for row in rows:
        expected, name = row.split("  ", 1)
        path = (root / name).resolve()
        if (not re.fullmatch("[a-f0-9]{64}", expected) or not path.is_relative_to(root)
                or not path.is_file() or sha(path) != expected):
            raise ValueError("Frozen BTreeSet input changed: " + name)
        names.append(name)
    if len(names) != len(set(names)) or set(names) != set(env["inputs"]):
        raise ValueError("Missing, duplicate or extra BTreeSet frozen input")
    mandatory = {SOURCE, MANIFEST, LLBC, ENVIRONMENT, "lean-toolchain", "lake-manifest.json",
                 "tools/lean_session.py", "scripts/lean-mcp-server.sh",
                 "SandboxModel/ProofAuditCommand.lean", "tools/check_btree_set_binding.py",
                 "tools/check_btree_set_binding_components.py",
                 EXPERIMENT + "/replay.py", EXPERIMENT + "/InvertFlag.lean",
                 "verification/extract-btree-set-binding/Cargo.lock",
                 "verification/extract-btree-set-binding/tests/insert_contract.rs",
                 "tools/check_foundations_extraction.py", "tools/check_loop_callbacks_contract.py"}
    mandatory.update(str(p.relative_to(root)) for p in (root / PROOFS).glob("*.lean"))
    mandatory.update(str(p.relative_to(root)) for p in (root / GENERATED).rglob("*") if p.is_file())
    if not mandatory <= set(names):
        raise ValueError("Incomplete BTreeSet source/model/tool input closure")
    if env["positive_audits"] != POSITIVE or env["negative_audits"] != NEGATIVE:
        raise ValueError("Expected exact original BTreeSet audit targets")
    actual = []
    for part in ("Theory", "Refinement", "Examples"):
        text = (root / PROOFS / (part + ".lean")).read_text()
        actual += ["SandboxModel.Foundations.BtreeSetBinding." + n
                   for n in re.findall(r"^theorem (\S+)", text, re.M)]
    if set(actual) != set(POSITIVE) or len(actual) != len(POSITIVE):
        raise ValueError("Missing, duplicate or extra normal-value theorem")
    for name, expected in [(PROOFS + "/Audit.lean", POSITIVE),
                           (EXPERIMENT + "/InvertFlag.lean", NEGATIVE)]:
        text = (root / name).read_text()
        observed = re.findall(r"^#audit_controller_proof (\S+)$", text, re.M)
        if observed != expected:
            raise ValueError("Audit must check exact existing declarations: " + name)
    for prefix in (PROOFS, GENERATED):
        for path in (root / prefix).rglob("*.lean"):
            if re.search(r"\b(?:sorry|admit|axiom|native_decide)\b", path.read_text()):
                raise ValueError("Placeholder/unchecked execution or external template in formal library")
    if sha(root / SOURCE) != env["correct_source_sha256"]:
        raise ValueError("Original transparent Std delegates changed")
    if hashlib.sha256(mutate((root / SOURCE).read_text(), "invert-flag").encode()).hexdigest() != env["wrong_source_sha256"]:
        raise ValueError("Wrong Rust source is not the registered actual mutation")
    data = json.loads((root / LLBC).read_text())
    inventory = json.loads((root / GENERATED / "translation.json").read_text())
    if data.get("has_errors") is not False or inventory["crate"] != "sandproof_btree_set_binding":
        raise ValueError("Missing successful original translation")
    translated = data["translated"]
    opts = translated["options"]
    if (translated["crate_name"] != "sandproof_btree_set_binding"
            or opts["start_from"] != ["crate::" + n for n in ROOT_NAMES]
            or opts["preset"] != "Aeneas" or opts["error_on_warnings"] is not True
            or any(opts[n] is not False for n in
                   ("skip_borrowck", "no_typecheck", "no_normalize", "no_reorder_decls"))):
        raise ValueError("BTreeSet roots/compiler safety configuration changed")
    owned = [d for d in translated["fun_decls"] if d is not None
             and d["item_meta"]["name"][0].get("Ident", [""])[0] == "sandproof_btree_set_binding"]
    if len(owned) != 5 or any(not isinstance(d["body"], dict) or set(d["body"]) != {"Structured"} for d in owned):
        raise ValueError("Every original owned delegate must remain transparent")
    if llbc_digest(data) != env["llbc_digest"] or foreign_surface(data) != env["opaque_surface"]:
        raise ValueError("Unreviewed raw LLBC body/signature/Drop change")
    for name, expected in env["generated_sha256"].items():
        if sha(root / name) != expected:
            raise ValueError("Original generated file changed: " + name)
    for name, expected in env["production_sources"].items():
        if name not in names or sha(root / name) != expected:
            raise ValueError("Actual borrowed-key source changed: " + name)
    return env


def source_fingerprint(root):
    """Use the original lean_session.py framing, including imported proof sources."""
    fingerprint = hashlib.sha256()
    sources = sorted((root / "SandboxModel").rglob("*.lean"))
    sources += sorted((root / "verification/aeneas").rglob("*.lean"))
    for source in sources:
        name = str(source.relative_to(root)).encode()
        fingerprint.update(len(name).to_bytes(8, "big"))
        fingerprint.update(name)
        fingerprint.update(hashlib.sha256(source.read_bytes()).digest())
    return fingerprint.hexdigest()


def service_records(log, root, expected_files, audits):
    """Require actual complete diagnostics, then return the original audit text."""
    root = Path(root)
    rows = []
    for line in Path(log).read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("request", {}).get("tool") == "lean_diagnostic_messages":
            rows.append(row)
    files, messages, names = [], [], []
    for row in rows:
        name = row["request"]["arguments"]["file_path"]
        revision = row["revision"]
        if (row["stale"] is not False or revision["file_sha256"] != sha(root / name)
                or revision["sources_sha256"] != source_fingerprint(root)
                or revision["toolchain"] != (root / "lean-toolchain").read_text().strip()
                or revision["lakefile_sha256"] != sha(root / "lakefile.toml")
                or revision["manifest_sha256"] != sha(root / "lake-manifest.json")):
            raise ValueError("Stale/wrong BTreeSet proof file or dependency/configuration context")
        item = row["result"]
        while "result" in item and "success" not in item:
            item = item["result"]
        if (item.get("partial") is not False or item.get("success") is not True
                or item.get("timed_out") is not False or item.get("failed_dependencies") != []
                or item.get("still_elaborating_lines") is not None):
            raise ValueError("Incomplete or failed actual BTreeSet service record")
        files.append(name)
        for diagnostic in item["items"]:
            text = diagnostic["message"]
            match = re.fullmatch(r"Checked (\S+): \d+ standard logical dependencies", text)
            if diagnostic["severity"] != "info" or match is None:
                raise ValueError("Unexpected BTreeSet service error/warning/message")
            names.append(match[1])
            messages.append(text)
    if (files != expected_files or len(names) != len(set(names))
            or set(names) != set(audits) or len(names) != len(audits)):
        raise ValueError("Missing, duplicate or extra actual files/audits")
    return messages


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args()
    result = check(args.root)
    print("BTreeSet: five original delegates; exact normal-value domain and 22+2 audit targets admitted.")
