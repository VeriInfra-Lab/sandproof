"""Install the original generated two-BLOB call and the existing SQL binding."""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import shutil

ROOT = Path(__file__).resolve().parents[5]
HERE = Path('verification/experiments/stage1-protocol/standard-save-digest/blob')
GEN = Path('verification/aeneas/stage1-protocol/standard-save-digest/blob/SandproofInboxSaveBlob')
PROOF = Path('SandboxModel/Lifecycle/Protocol/InboxSaveBlob.lean')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def install(project):
    project = project.resolve(strict=True)
    if project == ROOT:
        raise ValueError('only an independent private proof source may be materialized')
    data = json.loads((ROOT/HERE/'inventory.json').read_text())
    for section in ('sources','providers','templates','generated'):
        for name,digest in data[section].items():
            if (ROOT/name).is_symlink() or sha(ROOT/name) != digest:
                raise ValueError('reviewed source/provider changed: '+name)
    changed = {}
    def write(relative, value):
        target = project/relative
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_bytes(value)
        changed[str(relative)] = sha(target)
    for name in ('Types.lean','Funs.lean'):
        write(GEN/name,(ROOT/GEN/name).read_bytes())
    for name in ('TypesExternal','FunsExternal'):
        write(GEN/(name+'.lean'),(ROOT/HERE/(name+'.lean.in')).read_bytes())
    provider = ROOT/'verification/experiments/stage1-protocol/create-lifetime-sql/materialize.py'
    spec=importlib.util.spec_from_file_location('blob_exact_original_sql_fragments',provider)
    original=importlib.util.module_from_spec(spec);spec.loader.exec_module(original)
    if original.hashes(ROOT) != data['exact_common_fragments']:
        raise ValueError('the adopted common SQL fragments changed')
    pieces=original.fragments(ROOT)
    common='''import SandproofInboxSaveBlob.Types
import SandproofFoundations.IdentifierLibrary
open Aeneas Aeneas.Std
namespace SandproofSqliteStatementsRust
abbrev rusqlite.params.Params := SandproofInboxSaveBlobRust.rusqlite.params.Params

'''+ '\n'.join(pieces[n]for n in ('sql_to_str','sql_string_deref','sql_bind_values','sql_execute'))+'\nend SandproofSqliteStatementsRust\n'
    write(GEN/'SqliteCommonBinding.lean',common.encode())
    write(PROOF,(ROOT/HERE/'InboxSaveBlob.lean.in').read_bytes())
    lake=project/'lakefile.toml';text=lake.read_text()
    name='SandproofInboxSaveBlob'
    if 'name = "'+name+'"' not in text:
        text+='\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "'+str(GEN.parent)+'"\nleanOptions = { weak.linter.style.nameCheck = false }\n'
        write(Path('lakefile.toml'),text.encode())
    return dict(changed=changed,inventory_sha256=sha(ROOT/HERE/'inventory.json'),
        source_admission_only=True,formal_acceptance=False,
        seed='SandboxModel.Lifecycle.Protocol.InboxSaveBlob')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--project',type=Path,required=True)
    args=parser.parse_args()
    print(json.dumps(install(args.project),indent=2))
