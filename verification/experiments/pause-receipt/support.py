"""This receipt slice's archived projects and original captures; not a cache.

Private exploration results never become public proof receipts. Only original
capture bytes with unchanged compiler inputs can feed the current full replay.
Lean's own cache is retained in explicitly supplied operator-owned projects.
"""
from pathlib import Path
import hashlib
import json
import os
import shutil
import stat
import subprocess


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare_project(destination, contents, modes, *, existing=False, history=None):
    destination = Path(destination)
    if (destination.is_symlink() or not destination.is_absolute() or '..' in destination.parts
            or any(parent.is_symlink() for parent in destination.parents)):
        raise ValueError('Receipt project must be a real absolute private directory')
    if existing:
        if not destination.is_dir():
            raise ValueError('Existing receipt project is missing')
    else:
        destination.mkdir(mode=0o700)
    previous = None
    if (destination/'.git').is_dir():
        previous = subprocess.check_output(['git','rev-parse','HEAD'],cwd=destination,text=True).strip()
    changed = []
    for name,data in contents.items():
        path=destination/name
        for parent in path.parents:
            if parent==destination: break
            if parent.is_symlink(): raise ValueError('Linked archived receipt parent: '+name)
        if path.exists() or path.is_symlink():
            if not stat.S_ISREG(path.lstat().st_mode):
                raise ValueError('Nonregular archived receipt input: '+name)
        path.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
        if not path.exists() or path.read_bytes()!=data:
            if path.exists() and history is not None:
                saved=Path(history)/name
                saved.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
                shutil.copy2(path,saved)
            path.write_bytes(data)
            changed.append(name)
        path.chmod(modes[name])
    expected=set(contents)
    for parent,dirs,names in os.walk(destination,followlinks=False):
        dirs[:]=[n for n in dirs if n not in ('.git','.tools','.lake','artifacts','target')]
        for name in names:
            path=Path(parent)/name
            rel=path.relative_to(destination)
            if ((path.suffix=='.rs' or path.name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')
                 or '.cargo' in rel.parts) and str(rel) not in expected):
                raise ValueError('Unarchived receipt compiler input: '+str(rel))
    return dict(existing_project=existing,previous_git_head=previous,
                source_archive_prepared=True,changed_inputs=changed,Lean_cache_deleted=False)


def link_seed(path, seed):
    if path.exists() or path.is_symlink():
        if not path.is_dir() or (path.stat().st_dev,path.stat().st_ino)!=(seed.stat().st_dev,seed.stat().st_ino):
            raise ValueError('Existing receipt tool/package mapping differs: '+str(path))
    else:
        path.symlink_to(seed,target_is_directory=True)


def admit_capture(origin, destination, project, *, role, roots, native, checks):
    origin=Path(origin)
    if origin.is_symlink() or not origin.is_dir():
        raise ValueError('Original receipt capture directory is missing/linked')
    record=json.loads(checks.regular(origin/'result.json').read_text())
    if (record.get('complete') is not True or record.get('scope_admitted') is not True
            or record.get('capture_complete') is not True or record.get('translation_complete') is not True
            or record.get('ordinary_tests_run') is not False or record.get('roots')!=list(roots)):
        raise ValueError('Original complete twenty-root receipt capture is required')
    source_rows=[x for x in record['commands'] if x['label']=='checkout-candidate']
    if len(source_rows)!=1:
        raise ValueError('Original receipt capture source identity is missing')
    original=Path(source_rows[0]['cwd'])
    front=[x for x in record['commands'] if Path(x['argv'][0]).name=='charon']
    back=[x for x in record['commands'] if Path(x['argv'][0]).name=='aeneas']
    if len(front)!=1 or len(back)!=1:
        raise ValueError('One original frontend and backend command are required')
    expected=[front[0]['argv'][0],'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default']
    for name in roots: expected += ['--start-from','crate::'+name]
    for name in ('sandbox_controller_core','core::option::*::and_then','core::option::*::as_ref'):
        expected += ['--include',name]
    expected += ['--dest-file',str(origin/'capture_handoff.llbc'),'--','--manifest-path',
                 'Cargo.toml','--lib','--locked','--offline']
    expected_backend=[back[0]['argv'][0],'-backend','lean','-abort-on-error','-warnings-as-errors',
        '-split-files','-emit-json','-namespace','CaptureCallerRust.sandbox_controller_core',
        '-subdir','CaptureCaller','-dest',str(origin/'lean'),str(origin/'capture_handoff.llbc')]
    if (front[0]['argv']!=expected or front[0]['cwd']!=str(original/'controller-core')
            or back[0]['argv']!=expected_backend or back[0]['cwd']!=str(original)):
        raise ValueError('Original receipt compiler flags, roots or namespace changed')
    expected_outputs={'lean/translation.json',*('lean/CaptureCaller/'+name+'.lean'
        for name in ('Types','Funs','FunsExternal_Template'))}
    if set(record['outputs'])!=expected_outputs:
        raise ValueError('Original receipt complete output inventory changed')
    head=record['head']
    tree=subprocess.check_output(['git','rev-parse',head+'^{tree}'],cwd=original,text=True).strip()
    if tree!=record['tree']:
        raise ValueError('Original capture source tree identity differs')
    # Exact real local crate and inherited compiler configuration. Its frozen
    # Cargo.toml has no local dependency; changing it rejects this admission.
    names=subprocess.check_output(['git','ls-tree','-r','--name-only',head],cwd=original,text=True).splitlines()
    old_modes={}
    for item in subprocess.check_output(['git','ls-tree','-r','-z',head],cwd=original).split(b'\0'):
        if item:
            metadata,name=item.split(b'\t',1)
            old_modes[name.decode()]=metadata.split()[0].decode()
    def compiled(name):
        return (name.startswith(('controller-core/','.cargo/'))
                or name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml','rust-toolchain'))
    names={n for n in names if compiled(n)}
    current={str(p.relative_to(project)) for folder in ('controller-core','.cargo')
             if (project/folder).exists() for p in (project/folder).rglob('*')
             if p.is_file() and not any(x in ('.git','target','.lake')for x in p.relative_to(project).parts)}
    current.update(n for n in ('Cargo.toml','Cargo.lock','rust-toolchain.toml','rust-toolchain')if (project/n).is_file())
    if names!=current:
        raise ValueError('Receipt compiler file set changed since original capture')
    compiler_inputs={}
    for name in sorted(names):
        data=subprocess.check_output(['git','show',head+':'+name],cwd=original)
        expected=hashlib.sha256(data).hexdigest()
        if role=='wrong' and name==str(native.SOURCE):
            expected=record.get('wrong_effect_sha256')
        path=checks.regular(project/name)
        if (sha(path)!=expected
                or bool(path.stat().st_mode&0o111)!=(old_modes[name]=='100755')):
            raise ValueError('Receipt capture compiler input changed: '+name)
        compiler_inputs[name]=expected
    if role=='wrong' and sha(project/native.SOURCE)!=record.get('wrong_effect_sha256'):
        raise ValueError('Actual native/formal receipt mutation differs')
    if role=='correct' and sha(project/native.SOURCE)!=record.get('effect_sha256'):
        raise ValueError('Actual correct receipt source differs')
    for name,digest in record['outputs'].items():
        if sha(checks.regular(origin/name))!=digest:
            raise ValueError('Original receipt capture output changed: '+name)
    for row in record['commands']:
        if row.get('exit_code')!=0 or sha(checks.regular(origin/(row['label']+'.log')))!=row['log_sha256']:
            raise ValueError('Original receipt producer command is incomplete/corrupt')
    if sha(checks.regular(origin/'capture_handoff.llbc'))!=record['llbc_sha256']:
        raise ValueError('Original receipt intermediate representation changed')
    destination.mkdir(mode=0o700)
    for name in ('capture_handoff.llbc','result.json'):
        shutil.copy2(origin/name,destination/name)
    for row in record['commands']:
        shutil.copy2(origin/(row['label']+'.log'),destination/(row['label']+'.log'))
    for name in record['outputs']:
        target=destination/name
        target.parent.mkdir(mode=0o700,parents=True,exist_ok=True)
        shutil.copy2(origin/name,target)
    checks.verify_generation(destination,'CaptureCaller','CaptureCallerRust.sandbox_controller_core',
        checks.literal_constant(project/'verification/experiments/capture-handoff/caller_replay.py','EXTERNALS'))
    return dict(kind='original_capture_reused_not_a_proof_receipt',original_head=head,original_tree=tree,
                original_result_sha256=sha(origin/'result.json'),compiler_inputs=compiler_inputs,
                original_LLBC_sha256=record['llbc_sha256'],role=role)
