"""Capture the actual ordered observation-save prefix and the original caller."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import observation_slot_capture as slot


HERE = Path(__file__).with_name('observation-prefix')
NAMESPACE = 'SandproofObservationPrefix'
ROOT = slot.WORK + '::source_observation_prefix'
TRAIT = slot.WORK + '::SourceObservationPrefix'
ERRORS = slot.WORK + '::ObservationPrefixErrors'
MODES = ('core', 'begin-first', 'parent')
WORK = Path(slot.finish.prepare.SOURCE)
LIB = Path('controller-service/src/lib.rs')
GUARD = slot.ROOT / 'verification/experiments/stage1-protocol/source-observation'
PROBE = 'crate::proof_source_observation_matches'
INPUTS = {**slot.finish.prepare.SOURCE_INPUTS,
          str(LIB): 'bc15a0989bca7001b2927e6c912601832bf01601cf83e3cfac34351e4fbd6f73',
          'controller-service/src/capture/source.rs': '862c749de515fbe35595b6f00043455935d9581e22ae10554f639579c1833fbc'}
ORIGINAL = '''        if !observation.matches(work.requirement()) {
            return Err(JournalError::InvalidHostEvidence);
        }
        let active = self.active_host_epoch.clone();
        let tx =
            super::WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let inbox = super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, work.request(), context)?;
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let current = snapshot::current::<E>(
            &tx,
            &self.namespace,
            work.request(),
            now_ms,
            snapshot::CurrentSource {
                world: &tx.world(),
                inbox: &inbox,
                limits: &sources.config,
                active: active.as_ref(),
            },
        )?;
        if current != *work {
            return Err(JournalError::Conflict);
        }
'''
CURRENT = '''snapshot::current::<E>(
            &tx,
            &self.namespace,
            work.request(),
            now_ms,
            snapshot::CurrentSource {
                world: &tx.world(),
                inbox: &inbox,
                limits: &sources.config,
                active: active.as_ref(),
            },
        )'''
CALL = '''        let (tx, inbox, mut sources) = source_observation_prefix::<_, _, _, _, _, _, _, _, _, _, JournalObservationPrefix<E>>(
            (&mut self.journal.connection, &self.committed_world),
            &self.namespace,
            &self.active_host_epoch,
            work,
            context,
            &observation,
            now_ms,
            ObservationPrefixErrors {
                evidence: JournalError::InvalidHostEvidence,
                inbox: JournalError::UnsupportedStore,
                sources: JournalError::UnsupportedStore,
                conflict: JournalError::Conflict,
            },
        )?;
'''


def factor(text, mode):
    if mode not in MODES:
        raise ValueError('unknown actual observation prefix mode')
    work = slot.factor(text, 'core')
    opening = '    pub(crate) fn ' + slot.finish.METHOD
    before, parent = work.split(opening)
    marker = 'impl ControllerJournal {'
    if before.count(marker) != 1 or parent.count(ORIGINAL) != 1:
        raise ValueError('the exact original observation prefix is required')
    transfers = {
        'observation.matches(work.requirement())': 'Inputs::matches(observation, work)',
        'JournalError::InvalidHostEvidence': 'rejected.evidence',
        'self.active_host_epoch.clone()': 'active.clone()',
        'let tx =\n            super::WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;':
            'let tx = Inputs::begin(start)?;',
        'super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?':
            'Inputs::inbox(&tx)?.ok_or(rejected.inbox)?',
        'authenticated(&inbox, work.request(), context)': 'Inputs::authenticated(&inbox, work, context)',
        'let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;':
            'let sources = Inputs::sources(&tx)?.ok_or(rejected.sources)?;',
        CURRENT: 'Inputs::current(&tx, namespace, work, now_ms, &inbox, &sources, &active)',
        'JournalError::Conflict': 'rejected.conflict',
    }
    helper = ORIGINAL
    for old, new in transfers.items():
        if helper.count(old) != 1:
            raise ValueError('exact original operation or error transfer differs: ' + old)
        helper = helper.replace(old, new)
    if mode == 'begin-first':
        marker_text = '        let tx = Inputs::begin(start)?;\n'
        if helper.count(marker_text) != 1:
            raise ValueError('one original Begin call required')
        helper = marker_text + helper.replace(marker_text, '', 1)
    template = (HERE / 'prefix.rs.in').read_text()
    instance = template.replace('    OBSERVATION_PREFIX\n', helper)
    return before.replace(marker, instance + '\n' + marker) + opening + parent.replace(ORIGINAL, CALL)


def configuration(mode):
    include = [ROOT, TRAIT, ERRORS]
    if mode == 'parent':
        cfg = slot.configuration(mode)
        cfg['include'].extend([*include, *(slot.WORK + '::*::' + member for member in (
            'matches', 'begin', 'inbox', 'authenticated', 'sources', 'current'))])
        cfg['opaque'].append(slot.WORK + '::snapshot::current')
        return cfg
    return dict(roots=[ROOT], retained_roots=[PROBE], include=[
        'sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service', 'core::tuple'], opaque=[])


def stage(source, mode):
    for name, digest in INPUTS.items():
        if slot.digest(source / name) != digest:
            raise ValueError('the exact original input differs: ' + name)
    path = source / WORK
    path.write_text(factor(path.read_text(), mode))
    library = source / LIB
    with library.open('ab') as file:
        file.write((GUARD / 'probe.rs.in').read_bytes())
    return {str(name): slot.digest(source / name) for name in (WORK, LIB)}


def admit(ir, generated, mode, target):
    raw = json.loads(ir.read_bytes())
    translated = raw['translated']
    options = translated['options']
    cfg = configuration(mode)
    if (raw['has_errors'] is not False or options['start_from'] != cfg['roots']
            or options['start_from_if_exists'] != cfg['retained_roots'] or options['include'] != cfg['include']
            or options['exclude'] or options['opaque'] != cfg['opaque']
            or any(options[key] is not False for key in (
                'skip_borrowck', 'no_typecheck', 'remove_unused_clauses', 'monomorphize'))
            or {row['key'] for row in translated['target_information']} != {target}):
        raise ValueError('the actual observation prefix capture options differ')
    root = slot.declaration(translated['fun_decls'], ROOT)
    forms = json.loads((generated / 'translation.json').read_bytes())['functions']
    form, = [row for row in forms if row['def_id'] == root['def_id']]
    if form['is_opaque'] or form['source']['file'] != str(WORK):
        raise ValueError('the actual observation prefix must be transparently generated')
    report = dict(prefix=root['def_id'], definition=form['lean_name'])
    if mode == 'parent':
        parent = slot.finish.prepare.method(translated['fun_decls'], slot.WORK, slot.finish.METHOD)
        if list(slot.regular_calls(parent['body']['Structured']['body'])).count(root['def_id']) != 1:
            raise ValueError('the original parent must call the observation prefix exactly once')
        report['parent'] = parent['def_id']
    return report


def capture(mode, output, tools, lock):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(slot.ROOT).free, shutil.disk_usage(output.parent).free) < 80 * slot.GIB):
        raise ValueError('formal role and original capture filesystem budget required')
    cpus = sorted(os.sched_getaffinity(0))[:4]
    os.sched_setaffinity(0, cpus)
    os.umask(0o077)
    environment = json.loads((slot.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(slot.digest(path) != environment[name]['binary_sha256'] for name, path in (
            ('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the original fixed tool binaries are required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(mode=mode, source=slot.finish.SOURCE, tree=slot.finish.TREE, cpus=cpus, complete=False,
                  commands=[], driver_sha256=slot.digest(Path(__file__)),
                  template_sha256=slot.digest(HERE / 'prefix.rs.in'), probe_sha256=slot.digest(GUARD / 'probe.rs.in'))
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', slot.finish.SOURCE], cwd=slot.ROOT, stdout=stream, check=True)
        if slot.digest(archive) != slot.finish.ARCHIVE:
            raise ValueError('the original integration source archive differs')
        with tarfile.open(archive) as inputs:
            inputs.extractall(source, filter='data')
        report['staged'] = stage(source, mode)
        cfg = configuration(mode)
        selectors = [arg for flag, key in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                          ('--include', 'include'), ('--opaque', 'opaque'))
                     for item in cfg[key] for arg in (flag, item)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4', CARGO_INCREMENTAL='0',
                   CARGO_TARGET_DIR=str(output / 'target'))
        for key in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(key, None)
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', environment['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for label, command, limit in commands:
                result = slot.run(label, command, limit, source, output, env)
                report['commands'].append(result)
                if result['exit_code'] != 0 or result['bounded_stop']:
                    raise ValueError(label + ' did not complete')
                if label == 'charon':
                    report['ir_sha256'] = slot.digest(ir)
        if slot.digest(ir) != report['ir_sha256']:
            raise ValueError('actual observation prefix IR changed')
        report['admission'] = admit(ir, generated, mode, environment['target'])
        report['generated_sha256'] = {str(path.relative_to(generated)): slot.digest(path)
                                      for path in sorted(generated.rglob('*')) if path.is_file()}
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(mode, report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--mode', required=True, choices=MODES)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.mode, args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True))
