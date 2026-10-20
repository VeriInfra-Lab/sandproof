"""Admit a finite T7/T8 source review; never certifies program semantics."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tomllib
from tools import v0_use_inventory as census

ROOT=Path(__file__).resolve().parents[1]
CATALOGUE=Path('verification/os-boundary-catalogue.json')
IDS=('redis-session redis-result redis-scripts-codec writer-file-lock sqlite-config '
     'sqlite-update sqlite-commit-cache sqlite-ledger-recovery protected-fd file-staging-sync '
     'exact-cleanup disk-copy-space helper-child pidfd firecracker-api cgroup network-rules '
     'tls-config tls-original-provenance byte-stream guest-command-pty guest-files '
     'clock-deadlines crypto-bootstrap template-boundary').split()
STATUSES={'registered_contract','integrated_representative','business_proof','pending_617',
          'missing_public_binding','explicit_exclusion'}
EVIDENCE={'candidate-machine-proof','conditional-model','pending-candidate-machine-proof'}
DENIED={'catalogue_is_program_proof','all_runtime_calls_resolved','sql_success_is_durable_commit',
        'lua_error_rolls_back','kill_ack_is_exit','pipe_close_is_remote_eof',
        'tls_candidate_is_production_refinement','time_budget_is_memory_budget'}

def sha(data):return hashlib.sha256(data).hexdigest()
def read(path):return json.loads(path.read_text())
def safe(root,name):
    p=Path(name)
    if p.is_absolute() or '..' in p.parts or not (root/p).is_file():
        raise ValueError('missing or escaping source: '+name)
    return root/p

def validate(root,data):
    if (data.get('schema')!=1 or data.get('claim_kind')!='finite-source-and-contract-review'
            or data.get('new_program_theorems')!=0 or data.get('new_trusted_guarantees')!=0):
        raise ValueError('catalogue cannot grant trust or certify new program proofs')
    native=census.sources(root)
    digest=sha(json.dumps(native,sort_keys=True,separators=(',',':')).encode())
    if data.get('native_inputs_sha256')!=digest:
        raise ValueError('Rust/Cargo source changed; review scope again')
    versions=data['versions'];lock=tomllib.loads((root/'Cargo.lock').read_text())
    aliases={'libsqlite3_sys':'libsqlite3-sys','tokio_rustls':'tokio-rustls'}
    for name in ['redis_client','rusqlite','libsqlite3_sys','rustix','tokio','rustls','tokio_rustls','sha2','getrandom']:
        package='redis' if name=='redis_client' else aliases.get(name,name)
        if versions[name] not in {p['version'] for p in lock['package'] if p['name']==package}:
            raise ValueError('registered dependency version differs: '+name)
    pins=read(root/'references/v1-baseline.json')
    if (versions['rust']!=tomllib.loads((root/'rust-toolchain.toml').read_text())['toolchain']['channel']
            or versions['redis_server']!=pins['code_baseline']['center_storage']['redis_server']
            or versions['firecracker_and_jailer']!=pins['source_revisions']['firecracker_release']
            or versions['bundled_sqlite']!=read(root/'verification/experiments/sqlite-commit/environment.json')['runtime_sqlite'].split()[0]):
        raise ValueError('runtime/source pin version differs')
    scope=data['scope'];config=read(root/census.CONFIG);published=read(root/census.DIRECTORY/'catalogue.json')
    if (scope['roots']!=[r['id'] for r in config['roots']] or scope['profiles']!=config['profiles']
            or scope['target']!=config['target'] or len(scope['limits'])<4):
        raise ValueError('compiled roots/features/exclusions not preserved')
    compiled=data['compiled_source_evidence']
    if (compiled['source_inputs']!=len(native) or compiled['units']!=len(published['units'])
            or compiled['capture_commit']!=published['capture_commit']
            or compiled['capture_receipt_sha256']!=published['evidence']['receipt_sha256']
            or compiled['index_sha256']!=published['evidence']['index_sha256']
            or compiled['config_sha256']!=sha((root/census.CONFIG).read_bytes())):
        raise ValueError('published compiler evidence differs')
    if set(data.get('denied_inferences',{}))!=DENIED or any(v is not False for v in data['denied_inferences'].values()):
        raise ValueError('source/normal-return evidence promoted beyond its scope')
    families=data['families']
    if [r['id'] for r in families]!=IDS:raise ValueError('missing/extra/duplicate/reordered interface family')
    cache={};locations=0
    for row in families:
        if (row.get('public_status') not in STATUSES or row.get('new_trust_requested') is not False
                or row.get('use_binding')!='source-review-not-whole-function-proof'):
            raise ValueError('unreviewed status or trust scope')
        for key in ['title','original_object_and_config','normal_result','failure_and_partial_effects',
                    'cancel_panic_crash','progress_conditions','caller_obligations','locations','contracts','business_owners']:
            if not row.get(key):raise ValueError('missing guarantee/caller/source field: '+row['id']+'/'+key)
        if any(not re.fullmatch(r'A(?:[1-9]|1[0-2])|B[1-4]',v) for v in row['contracts']):
            raise ValueError('unknown A/B condition')
        if any(not re.fullmatch(r'V[0-9]',v) for v in row['business_owners']):raise ValueError('missing business owner')
        for loc in row['locations']:
            path=safe(root,loc['path'])
            if loc['path'] not in cache:cache[loc['path']]=path.read_bytes()
            raw=cache[loc['path']]
            text=raw.decode();lines=text.splitlines(keepends=True);start,end=loc['line'],loc['end_line']
            positions=[i for i,line in enumerate(text.splitlines(),1) if loc['symbol'] in line]
            if (positions!=[start] or not 1<=start<=end<=len(lines) or sha(raw)!=loc['file_sha256']
                    or sha(''.join(lines[start-1:end]).encode())!=loc['fragment_sha256']):
                raise ValueError('stale/ambiguous source location: '+row['id'])
            locations+=1
        for item in row['existing_evidence']:
            if item['kind'] not in EVIDENCE or not item.get('limit'):raise ValueError('unscoped proof label')
            if 'path' in item:safe(root,item['path'])
            elif (item.get('url')!='https://github.com/VeriInfra-Lab/sandproof/pull/617'
                  or item.get('commit')!='e50f6b5b56392ec5e6cee5abc4587693e47642af'
                  or row['public_status']!='pending_617'):
                raise ValueError('pending evidence promoted to integrated proof')
    missing=[r['id'] for r in families if r['public_status']=='missing_public_binding']
    if missing!=['tls-original-provenance']:raise ValueError('public missing binding classification changed')
    pending=[r['id'] for r in families if r['public_status']=='pending_617']
    if pending!=['helper-child']:raise ValueError('pending 617 scope changed')
    completion=data['public_completion'];proposal=completion['one_proposal']
    if (proposal['status']!='proposal-only-not-implemented' or proposal['new_trust'] is not False
            or proposal['contracts']!=['A6','A9','A12','B4'] or len(completion['T8_public_blockers'])!=2):
        raise ValueError('unapproved interface expansion or hidden blocker')
    return {'admitted':True,'families':len(families),'source_locations':locations,
            'compiled_variants':compiled['units'],'kind':data['claim_kind'],'new_program_theorems':0}

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=Path,default=ROOT)
    args=p.parse_args();print(json.dumps(validate(args.root,read(args.root/CATALOGUE)),ensure_ascii=False))
