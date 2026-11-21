"""Stage the fixed restore read roots and separate private Lean views."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
INPUTS = json.loads((HERE / "inputs.json").read_text())
RESTORE = Path("controller-service/src/controller/lifecycle_inbox/source_work/restore.rs")
TRANSACTION = Path("verification/experiments/create-transaction-link/current")
WORK = Path("verification/experiments/stage1-protocol/restore-work")


def git_show(revision, path):
    return subprocess.check_output(["git", "-C", REPO, "show", f"{revision}:{path}"])


def checked(data, expected, label):
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise SystemExit(f"{label} changed: {actual} != {expected}")
    return data


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def template(name):
    path = HERE / f"{name}.lean.in"
    return checked(path.read_bytes(), INPUTS["templates"][path.name], path)


def replace_once(data, old, new):
    if data.count(old) != 1:
        raise SystemExit(f"expected one occurrence of {old!r}")
    return data.replace(old, new, 1)


def stage_rust(project, mode):
    path = project / RESTORE
    source = checked(path.read_bytes(), INPUTS["restore_source_sha256"], path)
    permit_body = source.split(b"    pub fn lifecycle_restore_activation_permit(", 1)[1]
    return_body = b"        tx.commit()?;\n        Ok(permit)\n"
    if mode == "work":
        signature = b"""
// Private extraction view of the complete original return after check_identity.
fn proof_return_permit_after_identity(
    tx: rusqlite::Transaction<'_>,
    permit: RestoreActivationPermit,
) -> Result<RestoreActivationPermit, JournalError> {
"""
        carrier = HERE.parent / "carrier-root.rs.in"
        write(project / "controller-service/src/proof_restore_activation_carrier.rs", carrier.read_bytes())
        with (project / "controller-service/src/lib.rs").open("ab") as original:
            original.write(b'\n#[cfg(feature = "durable")]\npub mod proof_restore_activation_carrier;\n')
    else:
        return_body = b"        self.committed_world.check_identity(&tx)?;\n" + return_body
        signature = b"""
// Private view: the journal receiver is the original self value.
fn proof_finalize_read_permit(
    journal: &ControllerJournal,
    tx: rusqlite::Transaction<'_>,
    permit: RestoreActivationPermit,
) -> Result<RestoreActivationPermit, JournalError> {
"""
    if return_body + b"    }\n" not in permit_body.split(b"\n    }\n", 1)[0] + b"\n    }\n":
        raise SystemExit("original permit return suffix changed")
    path.write_bytes(source + signature + return_body.replace(b"self.", b"journal.", 1) + b"}\n")


def verify_generated(mode, root):
    entries = INPUTS["generated"][mode]
    for name, expected in entries.items():
        if name != "module":
            path = root / entries["module"] / name
            checked(path.read_bytes(), expected, path)


def provider_materializer(project):
    revision = INPUTS["transaction_provider"]
    manifest = json.loads(git_show(revision, TRANSACTION / "materialization.json"))["modules"]
    copied = set()

    def original(module):
        entry = manifest[module]
        return checked(git_show(revision, entry["source"]), entry["sha256"], module)

    def stage(module):
        if module in copied or module.split(".")[0] in {"Aeneas", "Lean", "Mathlib", "Std", "Init", "Batteries"}:
            return
        data = original(module)
        copied.add(module)
        write(project / manifest[module]["path"], data)
        for dependency in re.findall(rb"^import ([A-Za-z0-9_.]+)$", data, re.M):
            stage(dependency.decode())

    stage("SandproofParentCommit.FunsExternal")
    stage("SandproofSqliteSession.RecordQueryProviders")
    return original


def lake_library(config, name, src=None):
    with config.open("a") as output:
        output.write(f'\n[[lean_lib]]\nname = "{name}"\n')
        if src:
            output.write(f'srcDir = "{src}"\n')


def stage_work(project, generated):
    parent = Path("verification/aeneas/stage1-protocol/restore-work/correct")
    generated = generated / "SandproofRestoreWork"
    library = project / parent / generated.name
    for name in ("Types", "Funs"):
        write(library / f"{name}.lean", (generated / f"{name}.lean").read_bytes())
    types = (project / WORK / "TypesExternal.lean.in").read_bytes()
    types = replace_once(types, b"import SandproofBoundedWriter.TypesExternal", b"import SandproofBoundedWriter.TypesExternal\nimport SandproofParentCommit.Types")
    types = replace_once(types, b"abbrev rusqlite.error.Error := SandproofBoundedWriterRust.rusqlite.error.Error",
                         b"abbrev rusqlite.error.Error := SandproofParentCommitRust.rusqlite.error.Error\n"
                         b"abbrev rusqlite.transaction.Transaction := SandproofParentCommitRust.rusqlite.transaction.Transaction")
    write(library / "TypesExternal.lean", types)
    funs = (project / WORK / "FunsExternal.lean.in").read_bytes()
    funs = replace_once(funs, b"import Aeneas\n", b"import Aeneas\nimport SandproofParentCommit.FunsExternal\n")
    funs = replace_once(funs, b"end SandproofRestoreWorkRust",
                        b"abbrev rusqlite.transaction.Transaction.commit := SandproofParentCommitRust.rusqlite.transaction.Transaction.commit\n"
                        b"end SandproofRestoreWorkRust")
    write(library / "FunsExternal.lean", funs)
    proof = project / "SandboxModel/Lifecycle/Protocol/RestoreWork"
    for name in ("Spec", "Cloning", "Core"):
        write(proof / f"{name}.lean", (project / WORK / f"RestoreWork{name}.lean.in").read_bytes())
    recipe = json.loads((project / WORK / "reuse.json").read_text())
    data = checked((project / recipe["provider"]).read_bytes(), recipe["provider_sha256"], recipe["provider"])
    for before, after in recipe["replacements"]:
        if before.encode() not in data:
            raise SystemExit(f"original cloning source no longer contains {before}")
        data = data.replace(before.encode(), after.encode())
    write(project / recipe["rendered_module"], data)
    lake_library(project / "lakefile.toml", generated.name, parent)


def stage_identity(project, generated, provider):
    module = INPUTS["generated"]["identity"]["module"]
    for name in ("Types", "Funs"):
        write(project / module / f"{name}.lean", (generated / module / f"{name}.lean").read_bytes())
    for name in ("TypesExternal", "FunsExternal"):
        write(project / module / f"{name}.lean", template(f"Identity{name}"))
    guards = provider("SandproofStage1Request.FunsExternal")
    (body,) = re.findall(rb"(^def core\.cell\.RefCell\.borrow_mut .*?)(?=^-- Only the two reviewed opaque)", guards, re.M | re.S)
    write(project / "verification/aeneas/stage1-protocol/request/SandproofStage1Request/PermitIdentityOps.lean",
          b"import SandproofStage1Request.TypesExternal\nopen Aeneas Aeneas.Std\n"
          b"namespace SandproofStage1RequestRust\n\n" + body + b"end SandproofStage1RequestRust\n")
    storage = (project / "verification/aeneas/storage/SandproofStorage/TypesExternal.lean").read_bytes()
    (file_type,) = re.findall(rb"^structure std\.fs\.File where\n  identity : Nat$", storage, re.M)
    write(project / "SandproofStorage/PermitFileType.lean",
          b"namespace SandproofStorageRust\n\n" + file_type + b"\n\nend SandproofStorageRust\n")
    lake_library(project / "lakefile.toml", module)
    lake_library(project / "lakefile.toml", "SandproofStorage")


def stage_lean(project, work, identity):
    verify_generated("work", work)
    verify_generated("identity", identity)
    provider = provider_materializer(project)
    lakefile = project / "lakefile.toml"
    lakefile.write_bytes(git_show(INPUTS["transaction_provider"], TRANSACTION / "lakefile.toml.in"))
    stage_work(project, work)
    stage_identity(project, identity, provider)
    for name in ("PermitReadWork", "PermitReadReturn", "PermitReadIdentity"):
        write(project / f"{name}.lean", template(name))
        lake_library(lakefile, name)


def stage_query(project, generated):
    verify_generated("query", generated)
    a_stager = project / "verification/experiments/restore-claim/stage_source_query.py"
    subprocess.run(["python3", a_stager, project, "--generated", generated], check=True)
    write(project / "PermitReadSelection.lean", template("PermitReadSelection"))
    lake_library(project / "lakefile.toml", "PermitReadSelection")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name, arguments in (
        ("rust", ("project", "mode")),
        ("lean", ("project", "work", "identity")),
        ("query", ("project", "generated")),
    ):
        command = commands.add_parser(name)
        for argument in arguments:
            if argument == "mode":
                command.add_argument(argument, choices=("work", "identity"))
            else:
                command.add_argument(argument, type=Path)
    args = vars(parser.parse_args())
    {"rust": stage_rust, "lean": stage_lean, "query": stage_query}[args.pop("command")](**args)
