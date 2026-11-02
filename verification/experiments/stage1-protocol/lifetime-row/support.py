"""Source and same-property mutation admission for the actual lifetime validator."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'
SOURCE = 'controller-service/src/controller/admission.rs'
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
def mode(path): return '100755' if path.stat().st_mode & 0o111 else '100644'
def require(condition, message):
    if not condition: raise ValueError(message)
def module(name,path):
    spec=importlib.util.spec_from_file_location(name,path);value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
base=module('lifetime_row_original_inputs',HERE.parent/'schema-leaves/formal.py')
registry=module('lifetime_row_original_registry',HERE.parent/'envelope-writer/native.py');registry.HERE=Path('.')
def source_guard():
    head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    require(subprocess.run(['git','merge-base','--is-ancestor',BASE,head],cwd=ROOT,check=False).returncode==0,'source base differs')
    names=subprocess.check_output(['git','diff','--name-only','-z',BASE],cwd=ROOT).decode().split('\0')
    names+=subprocess.check_output(['git','ls-files','--others','--exclude-standard','-z'],cwd=ROOT).decode().split('\0')
    require(all(n==SOURCE or n.startswith(('verification/experiments/stage1-protocol/lifetime-row/','verification/aeneas/stage1-protocol/lifetime-row/')) for n in filter(None,names)),'unowned source changed')
    data=json.loads((HERE/'inventory.json').read_text())
    for section in ('inputs','providers','templates','drivers'):
        for n,d in data[section].items():require(sha(ROOT/n)==d,'reviewed input changed: '+n)
    return head

def mutate(source, view):
    head,tail=source.split('pub(super) fn lifetime_from_row(',1)
    if view=='correct':return source
    if view=='skip-owner':
        before='    if stored_owner != expected_owner\n        || deadline_ms < 0'
        require(tail.count(before)==1,'owner comparison changed')
        tail=tail.replace(before,'    if deadline_ms < 0').replace('expected_owner','_expected_owner').replace('stored_owner','_stored_owner')
    elif view=='skip-startup':
        before='        || startup_deadline_ms < 0\n';require(tail.count(before)==1,'startup guard changed');tail=tail.replace(before,'')
    elif view=='lost-start':
        before='    Ok(Lifetime {\n        started_at_ms,';require(tail.count(before)==1,'start output changed');tail=tail.replace(before,'    Ok(Lifetime {\n        started_at_ms: None,',1)
    else:raise ValueError('unregistered view')
    return head+'pub(super) fn lifetime_from_row('+tail
