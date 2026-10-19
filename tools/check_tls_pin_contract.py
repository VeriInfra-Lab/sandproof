"""Audit the frozen TLS candidate, complete Rust bodies and explicit A6/B4 surface."""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import subprocess

try:
    from tools.check_foundations_extraction import llbc_digest
    from tools.check_first_chain_extraction import check_tools, ENVIRONMENT as TOOL_REGISTRY
except ModuleNotFoundError:
    from check_foundations_extraction import llbc_digest
    from check_first_chain_extraction import check_tools, ENVIRONMENT as TOOL_REGISTRY

ROOT = Path(__file__).resolve().parents[1]
ENVIRONMENT = "verification/tls-pin-environment.json"
SOURCE = "sandbox-internal/src/tls.rs"
PATCH = "verification/fixtures/tls-pin-candidate.patch"
LLBC = "verification/aeneas/llbc/sandproof_tls.llbc"
DEST = "verification/aeneas/tls"
PREFIX = "sandproof_internal::"
FIRST = "core::slice::{[T]}::first"
LAST = "core::slice::{[T]}::last"
VERIFY = PREFIX + "tls::verify_pin_bytes"
LEAF = PREFIX + "tls::leaf_pin_bytes"
EQUALITY = (PREFIX + "tls::{impl core::cmp::PartialEq<sandproof_internal::tls::CertificatePin>"
            " for sandproof_internal::tls::CertificatePin}::eq")
OWNED = {VERIFY, LEAF, EQUALITY}
LAWS = {
    "allow_iff", "peer_acceptance_iff", "byte_ne", "all_pairs", "pin_eq", "any_pins",
    "contains_pin", "actual_schema", "actual_correct", "actual_safe",
    "no_certificate_rejects", "empty_chain_rejects", "same_first_is_enough",
    "hash_failure_preserved", "hash_divergence_preserved", "leaf_schema", "leaf_correct",
    "linked_refusal_preserved", "linked_correct", "linked_matches_original",
    "accepts_registered_first", "returns_registered_first",
}
NAMESPACE = "SandboxModel.Boundaries.TlsPin."


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def identifiers(name):
    return [part["Ident"][0] for part in name if "Ident" in part]


def source_files(root):
    paths = subprocess.check_output(
        ["git", "-C", str(root), "ls-files", "-z", "--", "sandbox-internal"], text=True)
    files = {name for name in paths.split("\0") if name}
    files.update({"Cargo.toml", "Cargo.lock"})
    for path in (root / "sandbox-internal").rglob("*.rs"):
        relative = path.relative_to(root).as_posix()
        if relative not in files:
            raise ValueError("untracked Rust source in the actual TLS package: " + relative)
    return sorted(files)


def check_sources(root, environment):
    files = source_files(root)
    if files != sorted(environment["source_files"]):
        raise ValueError("actual package source inventory changed; review the call boundary")
    for name, digest in environment["source_files"].items():
        if sha(root / name) != digest:
            raise ValueError("TLS source or caller binding changed: " + name)
    if sha(root / PATCH) != environment["candidate_patch_sha256"]:
        raise ValueError("candidate patch changed")
    if sha(root / SOURCE) != environment["original_tls_sha256"]:
        raise ValueError("original production TLS input changed")
    if environment["scope"] != "isolated-refactor-candidate":
        raise ValueError("a candidate cannot be relabelled as original production refinement")
    if environment["tool_registry"] != str(TOOL_REGISTRY):
        raise ValueError("unreviewed shared tool registry")
    check_laws(root)


def check_laws(root):
    proof_files = sorted((root / "SandboxModel/Boundaries/TlsPin").glob("*.lean"))
    declarations = []
    audits = []
    for path in proof_files:
        text = path.read_text()
        if re.search(r"\b(sorry|admit|axiom)\b", text):
            raise ValueError("forbidden proof placeholder")
        declarations.extend(re.findall(r"^theorem ([A-Za-z0-9_]+)", text, re.M))
        audits.extend(re.findall(r"^#audit_controller_proof ([A-Za-z0-9_.]+)$", text, re.M))
    if Counter(declarations) != Counter(LAWS):
        raise ValueError("missing, duplicate or unreviewed TLS theorem")
    if Counter(audits) != Counter(NAMESPACE + name for name in LAWS):
        raise ValueError("TLS theorem audit omitted or duplicated")


def check_artifacts(root, llbc=None, inventory=None, *, mutant=False, frozen=True):
    environment = json.loads((root / ENVIRONMENT).read_text())
    if llbc is None:
        llbc = json.loads((root / LLBC).read_text())
    if inventory is None:
        inventory = json.loads((root / DEST / "translation.json").read_text())
    if llbc.get("has_errors") is not False:
        raise ValueError("missing or failed Charon extraction")
    translated = llbc["translated"]
    if translated["options"] != environment["charon_options"]:
        raise ValueError("unreviewed extraction option or output location")
    if any(translated["options"][key] for key in
           ("skip_borrowck", "no_typecheck", "no_normalize", "no_reorder_decls", "no_serialize")):
        raise ValueError("required compiler/translation checks disabled")
    if {target["key"] for target in translated["target_information"]} != {"x86_64-unknown-linux-gnu"}:
        raise ValueError("unreviewed target")
    if inventory.get("crate") != "sandproof_internal":
        raise ValueError("not the actual internal transport crate")
    expected = OWNED | {FIRST, LAST} if mutant else OWNED | {FIRST}
    functions = inventory["functions"]
    by_name = {item["rust_name"]: item for item in functions}
    if len(by_name) != len(functions) or set(by_name) != expected:
        raise ValueError("missing or additional actual root/library function")
    if len({item["def_id"] for item in functions}) != len(functions):
        raise ValueError("duplicate function id")
    opaque = {item["rust_name"] for item in functions if item["is_opaque"]}
    if opaque != ({FIRST, LAST} if mutant else {FIRST}):
        raise ValueError("owned decision body opaque or unreviewed external function")
    if any(item["is_local"] != (item["rust_name"] in OWNED) for item in functions):
        raise ValueError("function ownership changed")
    declarations = {item["def_id"]: item for item in translated["fun_decls"] if item}
    own_ids = {item["def_id"] for item in declarations.values()
               if identifiers(item["item_meta"]["name"])[:1] == ["sandproof_internal"]}
    if own_ids != {by_name[name]["def_id"] for name in OWNED}:
        raise ValueError("actual owned body omitted from the inventory")
    ordered = []
    for group in translated["ordered_decls"]:
        if "Fun" in group:
            functions_here = group["Fun"]
            if set(functions_here) == {"NonRec"}:
                ordered.append(functions_here["NonRec"])
            elif set(functions_here) == {"Rec"}:
                ordered.extend(functions_here["Rec"])
            else:
                raise ValueError("unknown function declaration group")
    if len(ordered) != len(set(ordered)) or not own_ids.issubset(ordered):
        raise ValueError("owned body missing or duplicated in declaration order")
    if inventory["globals"] or len(inventory["trait_impls"]) != 1:
        raise ValueError("unreviewed global or concrete trait implementation")
    if {item["rust_name"] for item in inventory["trait_decls"]} != {PREFIX + "tls::CertificateDigest"}:
        raise ValueError("unreviewed local trait dictionary")
    types = {item["rust_name"] for item in inventory["types"]}
    if types != {PREFIX + "TransportError", PREFIX + "tls::CertificatePin"}:
        raise ValueError("unreviewed value representation")
    type_text = (root / DEST / "SandproofTls/Types.lean").read_text()
    if "def tls.CertificatePin := Array Std.U8 32#usize" not in type_text:
        raise ValueError("pin representation must preserve all 32 bytes")
    traits = [item for item in translated["trait_decls"] if item
              and identifiers(item["item_meta"]["name"]) == ["sandproof_internal", "tls", "CertificateDigest"]]
    if len(traits) != 1:
        raise ValueError("missing explicit digest dictionary")
    trait = traits[0]
    if len(trait["methods"]) != 1:
        raise ValueError("unreviewed digest interface")
    method = trait["methods"][0]["skip_binder"]
    if method["name"] != "digest" or method["default"] is not None or method["signature"]["is_unsafe"]:
        raise ValueError("digest must remain an explicit safe input, not a default implementation")
    trait_values = {}
    for item in nodes(llbc):
        if "Value" in item and isinstance(item["Value"], list) and len(item["Value"]) == 2:
            key, value = item["Value"]
            if isinstance(value, dict) and set(value) == {"kind", "trait_decl_ref"}:
                if key in trait_values and trait_values[key] != value:
                    raise ValueError("ambiguous interned trait reference")
                trait_values[key] = value

    def label(function):
        if set(function) != {"Regular"}:
            raise ValueError("unreviewed indirect call")
        kind = function["Regular"]["kind"]
        if "Trait" in kind:
            reference, method_id = kind["Trait"]
            if "Deduplicated" in reference:
                reference = trait_values[reference["Deduplicated"]]
            else:
                reference = reference["Value"][1]
            if (reference["kind"] != {"Clause": {"Free": 0}} or method_id != 0
                    or reference["trait_decl_ref"]["skip_binder"]["id"] != trait["def_id"]):
                raise ValueError("digest call no longer uses the supplied original dictionary")
            return "digest"
        if set(kind) != {"Fun"}:
            raise ValueError("unknown call kind")
        name = identifiers(declarations[kind["Fun"]]["item_meta"]["name"])
        labels = {
            ("core", "slice", "first"): "first",
            ("core", "slice", "last"): "last",
            ("alloc", "vec", "deref"): "deref",
            ("core", "slice", "contains"): "contains",
            ("core", "array", "equality", "eq"): "array-eq",
        }
        if tuple(name) not in labels:
            raise ValueError("unreviewed call in an owned guard: " + "::".join(name))
        return labels[tuple(name)]

    expected_calls = {
        VERIFY: ["last" if mutant else "first", "deref", "digest", "contains"],
        LEAF: ["first", "deref", "digest"],
        EQUALITY: ["array-eq"],
    }
    for name in OWNED:
        item = by_name[name]
        declaration = declarations[item["def_id"]]
        body = declaration["body"]
        if not isinstance(body, dict) or set(body) != {"Structured"} or declaration["signature"]["is_unsafe"]:
            raise ValueError("owned body missing or unsafe")
        if item["source"]["file"] != SOURCE or item["lean_file"] != "SandproofTls/Funs.lean":
            raise ValueError("owned body is not the actual candidate file")
        calls = []
        for node in nodes(body):
            if any(key in node for key in ("Error", "Drop", "Loop")):
                raise ValueError("unreviewed error, observable Drop or loop")
            if "Call" in node:
                calls.append(label(node["Call"]["call"]["func"]))
        if calls != expected_calls[name]:
            raise ValueError("call order/parameter-provider surface changed: " + name)
        generated = (root / DEST / item["lean_file"]).read_text()
        short_name = item["lean_name"].removeprefix("SandproofTlsRust.")
        if not re.search(r"\bdef\s+" + re.escape(short_name) + r"\b", generated):
            raise ValueError("actual generated body missing")
    if frozen:
        if llbc_digest(llbc) != environment["llbc_sha256"]:
            raise ValueError("full canonical LLBC changed")
        for path, digest in environment["generated_files"].items():
            if sha(root / path) != digest:
                raise ValueError("generated input/model changed: " + path)
    return environment


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool-root", type=Path)
    parser.add_argument("--check-only", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    if not args.check_only and args.output is None:
        parser.error("--output is required for actual reproduction")
    try:
        environment = check_artifacts(ROOT)
        check_sources(ROOT, environment)
        if args.tool_root:
            check_tools(ROOT, args.tool_root.resolve())
        if args.check_only:
            print("TLS candidate source, complete bodies, explicit interface and all 22 audits admitted.")
            print("This does not establish original production or outer TLS-wrapper refinement.")
        else:
            try:
                from tools.tls_pin_reproduction import reproduce
            except ModuleNotFoundError:
                from tls_pin_reproduction import reproduce
            reproduce(ROOT, args.tool_root or ROOT / ".tools", args.output)
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, str(error) + "\n")


if __name__ == "__main__":
    main()

