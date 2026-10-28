"""Extract only the unchanged production budget constants with fixed tools."""
from pathlib import Path
import subprocess,json,hashlib
p=Path.cwd();raw=p/'artifacts/envelope-budget.llbc'
assert not raw.exists()
subprocess.run(['.tools/aeneas/charon/bin/charon','cargo','--preset=aeneas','--error-on-warnings','--sysroot','default','--include','sandproof_internal','--include','sandproof_protocol','--start-from','crate::request_body_bytes','--start-from','crate::max_options_bytes','--dest-file','../../../../artifacts/envelope-budget.llbc','--','--manifest-path','verification/experiments/stage1-protocol/envelope-budget/Cargo.toml','--lib','--locked','--offline'],check=True)
x=json.loads(raw.read_text());assert x['has_errors'] is False
assert {t['key'] for t in x['translated']['target_information']}=={'x86_64-unknown-linux-gnu'}
subprocess.run(['.tools/aeneas/bin/aeneas','-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json','-namespace','SandproofEnvelopeBudgetRust','-subdir','SandproofEnvelopeBudget','-dest','verification/aeneas/stage1-protocol/envelope-budget','artifacts/envelope-budget.llbc'],check=True)
print(json.dumps({'complete': True, 'llbc_sha256': hashlib.sha256(raw.read_bytes()).hexdigest()}))
