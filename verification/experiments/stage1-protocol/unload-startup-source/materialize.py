"""Materialize the fixed first-Unload Lean sources into an isolated project."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
REUSED = {
    "SandboxModel/ProofAuditCommand.lean": "SandboxModel/ProofAuditCommand.lean",
    "SandboxModel/Foundations/ByteArray.lean": "SandboxModel/Foundations/ByteArray.lean",
    "SandproofFoundations/TypesExternal.lean":
        "verification/aeneas/foundations/SandproofFoundations/TypesExternal.lean",
    "SandproofFoundations/IdentifierLibrary.lean":
        "verification/aeneas/foundations/SandproofFoundations/IdentifierLibrary.lean",
    "SandproofDigestArray32/Types.lean":
        "verification/experiments/stage1-protocol/standard-digest/reuse/"
        "verification/aeneas/stage1-protocol/digest-array32/SandproofDigestArray32/Types.lean",
    **{
        f"SandboxModel/Lifecycle/Protocol/{name}.lean":
        f"verification/experiments/stage1-protocol/standard-digest/reuse/{name}.lean.in"
        for name in (
            "Sha256Bytes", "Sha256BytesChecks", "DigestArray32Representation", "DigestByteBridge"
        )
    },
    **{
        f"{name}.lean": f"verification/experiments/stage1-protocol/original-source-rows/{name}.lean.in"
        for name in ("OriginalRowsDriver", "OriginalRowsInvariant")
    },
    "lean-toolchain": "lean-toolchain",
    "lake-manifest.json": "lake-manifest.json",
}
SQLITE_INPUTS = (
    "OriginalSourceSqliteOps.lean",
    "SourceStdTypes.lean",
    "verification/aeneas/original-source-rows/SandproofOriginalSourceRows/TypesExternal.lean",
    "verification/aeneas/create-transaction-link/SandproofLoadBlob/ReadValueData.lean",
    "verification/aeneas/create-transaction-link/SandproofLoadBlob/TypesExternal.lean",
    "verification/aeneas/create-transaction-link/SandproofSqliteSession/Data.lean",
    "verification/aeneas/create-transaction-link/SandproofSqliteSession/ReadOps.lean",
    "verification/aeneas/sqlite-statements/SandproofSqliteStatements/TypesExternal.lean",
    "verification/aeneas/sqlite/SandproofSqlite/TypesExternal.lean",
)


def materialize(output):
    shared = json.loads((HERE.parent / "original-source-rows/execution-inputs.json").read_text())
    reused = dict(REUSED)
    for name in SQLITE_INPUTS:
        entry = shared["materialize"][name]
        source = ROOT / entry["source"]
        if hashlib.sha256(source.read_bytes()).hexdigest() != entry["sha256"]:
            raise ValueError(f"#770 SQLite input changed: {entry['source']}")
        reused[str(Path(*Path(name).parts[-2:]))] = entry["source"]
    output.mkdir(mode=0o700)
    for relative, source in reused.items():
        target = output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / source, target)
    for source in sorted((HERE / "proof-inputs").rglob("*.lean.in")):
        target = output / source.relative_to(HERE / "proof-inputs").with_suffix("")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    shutil.copyfile(HERE / "lakefile.toml.in", output / "lakefile.toml")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory for the isolated Lean project")
    materialize(parser.parse_args().output)
