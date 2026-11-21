"""Stage the original restore identity checks alongside the original source-query view."""
import argparse
import json
from pathlib import Path
import re
import subprocess

from stage import HERE, INPUTS, RESTORE, checked, git_show, lake_library, replace_once, stage_query, write

CASE = HERE / "claim-source"
FIXED = json.loads((CASE / "inputs.json").read_text())
QUERY_CASE = Path("verification/experiments/restore-claim/stage_source_query.py")
QUERY_LIB = Path("verification/aeneas/restore-source-query/SandproofRestoreSourceQuery")
FOUNDATIONS = Path("verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean")
REQUEST_MODELS = Path("verification/aeneas/stage1-protocol/request/SandproofStage1Request/FunsExternal.lean")


def identity_roots(source):
    checked(source, INPUTS["restore_source_sha256"], RESTORE)
    validator = source.index(b"pub(super) fn validate_record(")
    start = source.index(b"    let capture = record.capture.as_ref()", validator)
    end = source.index(b"    let key = core::OperationKey", start)
    prefix = checked(source[start:end], FIXED["original_prefix_sha256"], "identity prefix")
    start = prefix.index(b"    if !matches!(original.request.operation, ")
    end = prefix.index(b"\n", start)
    line = prefix[start:end]
    guard = b"""
fn proof_original_unload_guard(
    original: &AcceptedLifecycleIntent,
    source_incarnation: sandproof_protocol::lifecycle::IncarnationId,
    id: sandproof_protocol::lifecycle::CaptureId,
) -> bool {
    """ + line.removeprefix(b"    if !") + b"\n}\n"
    signature = b"""
fn proof_validate_claim_source(
    record: &SourceWorkRecord,
    claim: &RestoreClaim,
) -> Result<(), JournalError> {
"""
    cut = replace_once(prefix, line, b"    if !proof_original_unload_guard(original, source_incarnation, id)")
    return (
        checked(guard, FIXED["guard_sha256"], "original pattern guard"),
        checked(signature + cut + b"    Ok(())\n}\n", FIXED["staged_prefix_sha256"], "validator cut"),
    )


def stage_rust(project, native):
    path = project / RESTORE
    source = path.read_bytes()
    guard, prefix = identity_roots(source)
    if native:
        fixture = CASE / "validate-claim-source-test.rs.in"
        write(path.parent / "tests/validate_claim_source.rs",
              checked(fixture.read_bytes(), FIXED["native_parity"]["fixture_sha256"], fixture))
        with path.with_name("tests.rs").open("ab") as tests:
            tests.write(b"\nmod validate_claim_source;\n")
        prefix = replace_once(prefix, b"fn proof_validate_claim_source(",
                              b"#[cfg(test)]\npub(super) fn proof_validate_claim_source(")
    else:
        subprocess.run(["python3", project / QUERY_CASE, project], check=True)
    path.write_bytes(source + guard + prefix)


def declaration(source, name):
    (body,) = re.findall(rb"^def " + re.escape(name.encode()) + rb"(?=\s|$).*?(?=\n\n)", source, re.M | re.S)
    return body


def stage_lean(project, original_query, joint_query):
    generated = joint_query / QUERY_LIB.name
    files = {
        name: checked((generated / name).read_bytes(), digest, generated / name)
        for name, digest in FIXED["exact_guard_shared_query_cut"]["generated"].items()
    }
    stage_query(project, original_query)
    for name in ("Types.lean", "Funs.lean"):
        write(project / QUERY_LIB / name, files[name])
    standard = (project / FOUNDATIONS).read_bytes()
    option_eq = git_show(INPUTS["production"], REQUEST_MODELS)
    bindings = [declaration(standard, name) for name in (
        "core.option.Option.as_ref",
        "core.result.Result.ok",
        "alloc.string.String.Insts.CoreCmpPartialEqString.eq",
    )]
    bindings.append(declaration(option_eq, "core.option.Option.Insts.CoreCmpPartialEqOption.eq"))
    external = project / QUERY_LIB / "FunsExternal.lean"
    extended = (external.read_bytes() + b"\nnamespace SandproofRestoreSourceQueryRust\n\n"
                + b"\n\n".join(bindings) + b"\n\nend SandproofRestoreSourceQueryRust\n")
    write(external, checked(extended, FIXED["bound_funs_external_sha256"], external))
    proof = CASE / "ClaimSourceIdentity.lean.in"
    write(project / "ClaimSourceIdentity.lean", checked(proof.read_bytes(), FIXED["proof_sha256"], proof))
    lake_library(project / "lakefile.toml", "ClaimSourceIdentity")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    rust = modes.add_parser("rust")
    rust.add_argument("project", type=Path)
    rust.add_argument("--native", action="store_true")
    lean = modes.add_parser("lean")
    lean.add_argument("project", type=Path)
    lean.add_argument("original_query", type=Path)
    lean.add_argument("joint_query", type=Path)
    options = vars(parser.parse_args())
    {"rust": stage_rust, "lean": stage_lean}[options.pop("mode")](**options)
