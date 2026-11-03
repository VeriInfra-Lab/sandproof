"""Source and same-property mutation admission for the actual adopted SourceWork pause projection."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'
SOURCE = 'controller-service/src/controller/lifecycle_inbox/source_work/full.rs'
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def mode(path): return '100755' if path.stat().st_mode & 0o111 else '100644'
def require(condition, message):
    if not condition: raise ValueError(message)
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
base=module('pause_request_original_inputs',HERE.parent/'schema-leaves/formal.py')
registry=module('pause_request_original_registry',HERE.parent/'envelope-writer/native.py');registry.HERE=Path('.')
def source_guard():
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    require(subprocess.run(['git','merge-base','--is-ancestor',BASE,head],cwd=ROOT,check=False).returncode==0,'source base differs')
    names=subprocess.check_output(['git','diff','--name-only','-z','HEAD'],cwd=ROOT).decode().split('\0')
    names+=subprocess.check_output(['git','ls-files','--others','--exclude-standard','-z'],cwd=ROOT).decode().split('\0')
    require(all(n==SOURCE or n.startswith(('verification/experiments/stage1-protocol/pause-request/','verification/aeneas/stage1-protocol/pause-request/')) for n in filter(None,names)),'unowned source changed')
    data=json.loads((HERE/'inventory.json').read_text())
    for section in ('inputs','providers','templates','drivers'):
        for n,d in data[section].items():require(sha(ROOT/n)==d,'reviewed input changed: '+n)
    return head

MUTATIONS = json.loads((HERE/'mutations.json').read_text())
def mutate(source, view):
    if view=='correct':return source
    recipe=MUTATIONS[view]
    for edit in recipe['edits']:
        require(source.count(edit['before'])==1,'actual mutation site changed: '+view)
        source=source.replace(edit['before'],edit['after'])
    require(hashlib.sha256(source.encode()).hexdigest()==recipe['sha256'],'not the exact captured Rust wrong body')
    return source
