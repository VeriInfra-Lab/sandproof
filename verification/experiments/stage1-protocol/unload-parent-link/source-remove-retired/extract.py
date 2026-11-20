"""Capture the actual per-record retirement clear, with explicit library calls.

The original parent loop is retained in Rust but is not a proved root: the
fixed backend's nested-borrow failure remains a separate obligation.
"""
import argparse
import fcntl
from hashlib import sha256
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[4]
sys.path.insert(0, str(HERE.parent))
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration
from source_save_row_list_capture import ARCHIVE, SOURCE, TREE
import ports

ROOT = ports.WORK + '::codec::clear_retired_source_record'
NAMESPACE = 'SandproofSourceRemoveRetired'
LIB = Path('controller-service/src/lib.rs')
STAGED = (ports.CODEC, ports.RETIREMENT, ports.CLOSURE, ports.CAPTURE, LIB)
VARIANTS = ('original', 'allow-zero-update')
OPTIONS = dict(roots=[ROOT], retained_roots=[], include=[
    'sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple',
], opaque=[ports.WORK+'::codec::load', 'serde_json', 'sha2', 'rusqlite', *ports.PORTS])
EXPECTED = ')?!=1 {return Err(JournalError::CorruptStore)}'
WRONG = ')?>1 {return Err(JournalError::CorruptStore)}'


def digest(path):
    return sha256(path.read_bytes()).hexdigest()


def mutate(text, variant):
    start = text.index('pub(in crate::controller) fn remove_retired(')
    stop = text.index('\npub(super) fn initialize(', start)
    actual = text[start:stop]
    if actual.count(EXPECTED) != 1:
        raise ValueError('one original exact-row-count retirement check is required')
    if variant == 'allow-zero-update':
        return text[:start] + actual.replace(EXPECTED, WRONG, 1) + text[stop:]
    if variant != 'original':
        raise ValueError('unknown retirement view')
    return text


def reconstruct(archive, source, variant):
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    codec = source / ports.CODEC
    original = mutate(codec.read_text(), variant)
    start = original.index('pub(in crate::controller) fn remove_retired(')
    stop = original.index('\npub(super) fn initialize(', start)
    actual = original[start:stop]
    loop_start = actual.index('    for record in expected {\n')
    loop_end = actual.rindex('    }\n    Ok(())\n}')
    body = actual[loop_start+len('    for record in expected {\n'):loop_end]
    last = EXPECTED if variant == 'original' else WRONG
    if (not body.startswith('        let key = record.work.requirement.work_binding;\n')
            or not body.endswith('        '+last+'\n')):
        raise ValueError('the complete original per-record body is required')
    helper = ('fn clear_retired_source_record(connection: &Connection, current: &Sources, '
              'keys: &mut BTreeSet<Vec<u8>>, record: &SourceWorkRecord) -> Result<(), JournalError> {\n'
              + body + '    Ok(())\n}\n\n')
    loop = '''    let mut records = expected.iter();
    loop {
        match records.next() {
            None => break Ok(()),
            Some(record) => match clear_retired_source_record(connection, &current, &mut keys, record) {
                Ok(()) => (),
                Err(error) => break Err(error),
            },
        }
    }
}
'''
    staged = helper + actual[:loop_start] + loop
    if staged.count(body) != 1:
        raise ValueError('the original row body must be moved verbatim exactly once')
    codec.write_text(original[:start] + staged + original[stop:])
    if tuple(ports.stage(source)) != STAGED[:-1]:
        raise ValueError('the complete library-call staging inventory is required')
    library = source / LIB
    text = library.read_text()
    conversion = 'impl From<serde_json::Error> for JournalError {\n    fn from(error: serde_json::Error) -> Self {'
    renamed = ('#[charon::rename("JournalErrorFromJson")]\nimpl From<serde_json::Error> for JournalError {\n'
               '    #[charon::rename("from_json")]\n    fn from(error: serde_json::Error) -> Self {')
    library.write_text('#![feature(register_tool)]\n#![register_tool(charon)]\n'
                       + ports.once(text, conversion, renamed))


def admit(ir, generated, target):
    tree = checked_translation(ir, OPTIONS, target, ROOT, None)
    mapping = json.loads((generated/'translation.json').read_bytes())['functions']
    business = [ROOT, ports.WORK+'::codec::checked_retired_record', *(
        ports.WORK+'::retirement::'+name for name in (
            'is_settled', 'material_charge', 'bounded', 'request', 'closure::restore_binding',
            'closure::validate_bindings'))]
    report = dict(business={}, library={})
    for name in business:
        function = declaration(tree['fun_decls'], name)
        entries = [item for item in mapping if item['def_id']==function['def_id']]
        if ('Structured' not in (function['body'] or {}) or not entries
                or any(item['is_opaque'] for item in entries)):
            raise ValueError('the entire actual business declaration must be transparent: '+name)
        report['business'][name] = function['def_id']
    for name in ports.PORTS:
        function = declaration(tree['fun_decls'], name)
        entries = [item for item in mapping if item['def_id']==function['def_id']]
        if len(entries)!=1 or entries[0]['is_opaque'] is not True:
            raise ValueError('one explicit typed library call is required: '+name)
        report['library'][name] = function['def_id']
    opaque_ids = {declaration(tree['fun_decls'], name)['def_id'] for name in ports.PORTS}
    unexpected = [item for item in mapping if item['is_opaque']
                  and item['source']['file'].startswith('controller-service/')
                  and item['def_id'] not in opaque_ids]
    if unexpected:
        raise ValueError('no additional owned business opacity is allowed')
    return report


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE')!='formal'
            or min(shutil.disk_usage(path).free for path in (REPO, output.parent))<80*GIB):
        raise ValueError('the existing formal role and capture budget are required')
    os.umask(0o077)
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    fixed = json.loads((REPO/'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools/'charon/bin/charon', tools/'bin/aeneas'
    if any(digest(binary)!=fixed[name]['binary_sha256']
           for name,binary in (('charon',charon),('aeneas',aeneas))):
        raise ValueError('the original fixed tool binaries are required')
    if subprocess.check_output(['git','rev-parse',SOURCE+'^{tree}'],cwd=REPO,text=True).strip()!=TREE:
        raise ValueError('the exact original production tree is required')
    output.mkdir(mode=0o700)
    report = dict(source=SOURCE,tree=TREE,variant=variant,options=OPTIONS,
                  driver=digest(Path(__file__)),ports_driver=digest(Path(ports.__file__)),
                  complete=False,public_acceptance=False,commands=[])
    try:
        archive = output/'source.tar'
        with archive.open('xb') as file:
            subprocess.run(['git','archive',SOURCE],cwd=REPO,stdout=file,check=True)
        if digest(archive)!=ARCHIVE:
            raise ValueError('the complete fixed production archive is required')
        source = output/'source'
        source.mkdir(mode=0o700)
        reconstruct(archive,source,variant)
        report['staged'] = {str(path):digest(source/path) for path in STAGED}
        ir, generated = output/'actual.llbc', output/'generated'
        selectors = [part for flag,group in (('--start-from','roots'),('--start-from-if-exists','retained_roots'),
                     ('--include','include'),('--opaque','opaque')) for pattern in OPTIONS[group]
                     for part in (flag,pattern)]
        env = dict(os.environ,CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4',CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output/'target'))
        for name in ('RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name,None)
        commands = [
            ('charon',[charon,'cargo','--preset=aeneas','--error-on-warnings','--sysroot','default',*selectors,
                       '--dest-file',ir,'--','--manifest-path','controller-service/Cargo.toml','--lib',
                       '--locked','--offline','--target',fixed['target']],240),
            ('aeneas',[aeneas,'-backend','lean','-abort-on-error','-warnings-as-errors','-split-files','-emit-json',
                       '-namespace',NAMESPACE,'-subdir',NAMESPACE,'-dest',generated,ir],120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard,fcntl.LOCK_EX|fcntl.LOCK_NB)
            for name,argv,seconds in commands:
                result = run(name,argv,seconds,source,output,env)
                report['commands'].append(result)
                if result['exit_code'] or result['bounded_stop']:
                    raise ValueError('the complete fixed '+name+' result is required')
        report['ir'] = digest(ir)
        report['generated'] = {str(path.relative_to(generated)):digest(path)
                               for path in sorted(generated.rglob('*')) if path.is_file()}
        report['admission'] = admit(ir,generated,fixed['target'])
        report['complete'] = True
    finally:
        (output/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(variant,'complete actual per-record retirement-clear capture')


if __name__=='__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant',choices=VARIANTS,required=True)
    for name in ('output','tools','lock'):
        parser.add_argument('--'+name,type=Path,required=True)
    args = parser.parse_args()
    capture(args.output.resolve(),args.tools.resolve(strict=True),args.lock.resolve(strict=True),args.variant)
