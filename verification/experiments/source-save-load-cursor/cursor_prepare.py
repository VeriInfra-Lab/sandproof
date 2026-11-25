"""Assemble actual ordered cursor views from the current admitted Rust declarations."""

import json
import re
import shutil

from cursor_inputs import HERE, retention
from rows_prepare import declarations
from load_prepare import read_exact


def cursor_view(old, generated):
    funs = declarations(generated / 'Funs.lean')
    root='controller.lifecycle_inbox.source_work.codec'
    mapper=root+'.read_source_columns'
    scan=root+'.scan_loaded_records'
    iterator=root+'.SourceRows.Insts.CoreIterTraitsIteratorIteratorResultTupleVecU8VecU8OptionVecU8Error'
    ns='SandboxModel.Stage1Protocol.SaveLoadCursorView'
    header=f"""import SaveLoadRowsComposition
import Aeneas.Std.VecIter

open Aeneas Aeneas.Std Result
open SandproofSaveRetentionRows hiding toStr
open SandboxModel.Stage1Protocol.SaveLoadRowsView (Returned)
set_option autoImplicit false
set_option linter.style.longLine false
set_option linter.style.whitespace false

namespace {ns}
noncomputable section

private abbrev toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  SandproofSaveRetentionRows.toStr text bounded

abbrev Bytes := alloc.vec.Vec U8
abbrev Columns := Bytes × Bytes × Option Bytes
abbrev SqlError := rusqlite.error.Error
abbrev CursorItem := core.result.Result Columns SqlError
abbrev Cursor := alloc.vec.into_iter.IntoIter CursorItem
abbrev Reader := core.iter.traits.iterator.Iterator Cursor CursorItem → Cursor → Result Returned
abbrev rowIterator := core.iter.traits.iterator.IteratorVecIntoIter CursorItem
"""
    body=old[old.index('abbrev RawRow'):old.index('local notation "alloc.vec.VecU8.Insts.')]
    body=body.replace('SandproofSourceRowSqlSelect.ReadValue','SandproofSaveRetentionRows.ReadValue')
    dictionaries=old[old.index('local notation "alloc.vec.VecU8.Insts.'):old.index('def controller.lifecycle_inbox.source_work.codec.read_source_columns')]
    dictionaries=dictionaries.replace('SandproofSourceRowSqlSelect.core.option','SandproofSaveRetentionRows.core.option')
    dictionaries=dictionaries.replace('local notation "JournalError.Insts.CoreConvertFromError" => SourceLoadView.fromSql\n','')
    tail=f"""

section
variable (library : Library) (reader : Reader)
local notation "rusqlite.Connection.prepare" => prepareScan
local notation "{root}.query_source_rows" => openScan library
local notation "{root}.read_loaded_rows" => reader

"""+funs[scan]+f"""

end

abbrev mapper := {mapper}
abbrev scan := @{scan}
abbrev blobDictionary := alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql
abbrev nullableDictionary := core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql blobDictionary
abbrev cursorDictionary := {iterator}
def sourceSelect : Str := toStr
  "SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id"

end
end {ns}
"""
    return header + body + dictionaries + funs[mapper] + tail


def wrong_view(generated):
    funs = declarations(generated / 'Funs.lean')
    root = 'controller.lifecycle_inbox.source_work.codec.read_source_columns'
    ns = 'SandboxModel.Stage1Protocol.SaveLoadCursorWrong'
    header=f"""import SaveLoadCursorView

open Aeneas Aeneas.Std Result SandproofSaveRetentionRows
set_option autoImplicit false
set_option linter.style.longLine false

namespace {ns}

open SaveLoadCursorView

local notation "alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql" => blobDictionary
local notation "core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql" =>
  SaveLoadCursorView.core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql

"""
    return header + funs[root] + f'\n\nabbrev mapper := {root}\n\nend {ns}\n'


def materialize(out, parent, reference, good, bad):
    from prepare_world_begin import closure

    out.mkdir(mode=0o700)
    for relative in ('.lake/packages', '.tools/lean-mcp'):
        link = out / relative
        link.parent.mkdir()
        link.symlink_to((parent / relative).resolve(strict=True))
    prior = json.loads((parent / 'plan.json').read_bytes())
    for name in prior['files']:
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(parent, prior, name))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        path = out / name
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(parent / name, path)
    baseline = json.loads((reference / 'plan.json').read_bytes())
    old = read_exact(reference, baseline, 'SourceSqlCursorView.lean').decode()
    files = {
        'SaveLoadCursorView': cursor_view(old, good),
        'SaveLoadCursorWrong': wrong_view(bad),
    }
    for name, source in files.items():
        (out / (name + '.lean')).write_text(source)
    local = ('SaveLoadCursorProof', 'SaveLoadCursorComposition', 'SaveLoadCursorWitness')
    for name in local:
        fname = name + '.lean'
        shutil.copyfile(HERE / fname, out / fname)
    lake = (parent / 'lakefile.toml').read_text()
    lake += ''.join(f'\n[[lean_lib]]\nname = "{name}"\n' for name in (*files, *local))
    (out / 'lakefile.toml').write_text(lake)
    root = 'SaveLoadCursorWitness'
    files = closure(out, [root + '.lean'], active_view='lakefile.toml')
    audits = sorted({name for file in files for name in re.findall(
        r'^#audit_controller_proof (\S+)$', (out / file).read_text(), re.M)})
    plan = dict(root=root, source=retention.SOURCE, files=files, strict_files=sorted(files),
                modules={name: {'sha256': digest} for name, digest in files.items()}, audits=audits,
                build_order=[name.removesuffix('.lean').replace('/', '.') for name in files],
                public_acceptance=False)
    retention.write_json(out / 'plan.json', plan)
    return plan
