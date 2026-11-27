"""Join two complete Aeneas pages to the recording provider's existing request types."""
import json
import os
from pathlib import Path
import re

from codec_receipt import AREA, PACKAGE, digest, sha
from journal_hook import HOOK, LEAN_METHOD, LIB as HOOK_SOURCE, TRAIT, check_journal_hook
from wiring import DECIDE, MODEL, MODULE, SERVICE, declaration, inspect_parent, require
from guard_binding import check_guard
from journal_entry import check_entry
from sender_binding import inspect_senders

LIB = 'SandproofJournalDecision'
FOLDER = Path('verification/aeneas/journal-decision')
REQUEST = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request/Types.lean')
PROTOCOL = Path('SandboxModel/Lifecycle/Protocol')
ACTION = MODULE + '::DataRevocation'
METHOD = SERVICE + '::{impl ' + TRAIT + ' for ' + HOOK + '}::hit'
SHARED = (SERVICE+'::JournalError', MODEL+'::LifecycleWorkState',
          SERVICE+'::controller::internal_inbox::model::InternalLocalBinding',
          'sandproof_protocol::lifecycle::request::LifecycleOperation')
NAMESPACE = 'SandproofStage1RequestRust.'+SERVICE
GENERATED = {'translation.json', *(LIB+'/'+n+'.lean' for n in ('Types', 'Funs', 'TypesExternal_Template'))}
# Both fixed-tool runs use the local controller-service crate. Source annotations are
# relative, and no substitutions or slices of a generated function are installed.
PAGES = {
    'action': {'Types': 'ab5d051922cbc160d0494300f015225f465e4fb39b9d4f1292538a74790c9c2d',
               'Funs': '76f331598497a2f2999635844c8520c339a7852885cdc98c7ded4a2585253fc2'},
    'decision': {'Types': '340615286f09cb2c55c1c705c89eeeaa0bdcc90c8649c7a0110c883225b6becf',
                 'Funs': '091047c98227a817f9ed5a103abe4ae45d47746e8eeaa45364fc4a601e8d1bd1'},
}

PHYSICAL = {str(FOLDER/LIB/(stem+'.lean')): PAGES[mode][stem]
            for mode, stem in (('action', 'Types'), ('decision', 'Funs'))}


def profile(mode):
    opaque = [SERVICE, 'sandbox_controller_core', 'sandproof_protocol', 'tokio', 'serde_json', 'rusqlite',
              'std::io::error::Error']
    if mode == 'action':
        return {'start_from': [ACTION, HOOK], 'include': [ACTION, HOOK], 'opaque': opaque}
    require(mode == 'decision', 'unknown journal decision profile')
    include = [ACTION, *SHARED, DECIDE, HOOK, METHOD]
    return {'start_from': [DECIDE, METHOD.removeprefix(SERVICE+'::')], 'include': include, 'opaque': opaque}


def capture(source, captures, tools, fixed, run):
    for mode in PAGES:
        folder = captures/mode
        folder.mkdir()
        # Serial invocations share only Cargo dependencies inside this component.
        os.environ['CARGO_TARGET_DIR'] = str(captures/'decision-target')
        opts = profile(mode)
        flags = [arg for key, values in opts.items() for value in values
                 for arg in ('--'+key.replace('_', '-'), value)]
        ir = folder/'actual.llbc'
        run(source, folder, 'charon', [tools/'aeneas/charon/bin/charon', 'cargo', '--preset=aeneas',
            '--error-on-warnings', '--sysroot', 'default', *flags, '--dest-file', ir, '--', '--lib', '--manifest-path',
            'controller-service/Cargo.toml', '--locked', '--offline', '--target', fixed['target']])
        run(source, folder, 'aeneas', [tools/'aeneas/bin/aeneas', '-backend', 'lean', '-abort-on-error',
            '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', LIB,
            '-dest', folder/'generated', ir])


def declarations(page):
    """Extract only complete declaration bodies for comparison. Never use these as Lean input."""
    result = {}
    for block in page.split(b'\n/-- [')[1:]:
        symbol = block.split(b']', 1)[0].decode()
        match = re.search(rb'^(inductive|structure|def) .*?(?=\n\n|\Z)', block, re.M|re.S)
        if match:
            require(symbol not in result, 'duplicate generated type: '+symbol)
            result[symbol] = match[0]
    return result


def project_inputs(root, captures, request_types):
    def raw(mode, stem):
        page = (captures/mode/'generated'/LIB/(stem+'.lean')).read_bytes()
        require(digest(page) == PAGES[mode][stem], 'journal decision generation differs: '+mode+'/'+stem)
        return page

    action = raw('action', 'Types')
    small, complete, provider = map(declarations, (action, raw('decision', 'Types'), request_types))
    require(set(small) == {ACTION, HOOK} and set(complete) == set(small) | set(SHARED)
            and all(complete[n] == body for n, body in small.items()), 'action or hook type changed')
    # The service is the local crate in the decision view and an external crate in the
    # request view. Compare every constructor, field, payload and its order.
    for name in SHARED:
        require(name in provider and complete[name].split() ==
                provider[name].replace((SERVICE+'.').encode(), b'').split(),
                'journal decision and transaction disagree on '+name)
    return {
        str(FOLDER/LIB/'Types.lean'): action,
        str(FOLDER/LIB/'Funs.lean'): raw('decision', 'Funs'),
        str(FOLDER/LIB/'TypesExternal.lean'): b'import SandproofStage1Request.Types\n',
        str(PROTOCOL/'CoordinatorRevocation.lean'): ('import '+LIB+'.Funs\n').encode() +
            (root/PACKAGE/'CoordinatorRevocation.lean.in').read_bytes(),
        str(PROTOCOL/'InboxStateValues.lean'): (root/AREA/'inbox-state/InboxStateValues.lean.in').read_bytes(),
    }


def source_span(root, view, body, relative):
    span = body['item_meta']['span']['Untagged']['data']
    file = view.data['files'][span['file_id']]
    require(file['name'] == {'Local': str(relative)} and file['contents'] is not None
            and file['contents'].encode() == (root/relative).read_bytes(), 'detached decision source: '+str(relative))
    return span['beg'], span['end']


def readback(root, captures, original, target):
    """Bind the parent's actual journal, operation, result, guard and selected hook to both translations."""
    folder = captures/'coordinator'
    parent = original(json.loads((folder/'actual.llbc').read_bytes()))
    guard = check_guard(parent)
    branch = inspect_parent(parent.raw, refactored=True, guard=guard)
    selected = check_journal_hook(parent, root)
    caller = Path('controller-service/src/application/coordinator/lifecycle_intent.rs')
    result = {'coordinator': {'ir': sha(folder/'actual.llbc'), 'caller': sha(root/caller),
        'guard': guard, 'senders': inspect_senders(parent, branch), 'hook': selected, 'journal': check_entry(parent)}}
    for mode in PAGES:
        folder = captures/mode
        view = original(json.loads((folder/'actual.llbc').read_bytes()))
        opts = view.data['options']
        require([v['key'] for v in view.data['target_information']] == [target] and
                all(opts[k] == v for k, v in profile(mode).items()) and not any(opts[k] for k in
                ('start_from_if_exists', 'exclude', 'skip_borrowck', 'no_typecheck', 'monomorphize')),
                'unexpected '+mode+' extraction options')
        gen = folder/'generated'
        require({str(p.relative_to(gen)) for p in gen.rglob('*') if p.is_file()} == GENERATED,
                'unexpected '+mode+' output files')
        mapping = json.loads((gen/'translation.json').read_bytes())
        methods = {row['rust_name']:row for row in mapping['functions']}
        expected = {DECIDE, METHOD} if mode == 'decision' else set()
        require(set(methods) == expected and len(methods) == len(mapping['functions']) and
                not any(mapping[k] for k in ('globals', 'trait_decls', 'trait_impls')), 'unexpected decision symbols')
        for symbol, row in methods.items():
            body = view.functions[row['def_id']]
            relative = caller if symbol == DECIDE else HOOK_SOURCE
            start, end = source_span(root, view, body, relative)
            if symbol == DECIDE:
                require((start, end) == source_span(root, parent, declaration(parent.functions.values(), DECIDE), caller),
                        'parent selected another decision implementation')
            expected_name = 'SandproofStage1RequestRust.'+DECIDE.replace('::', '.') if symbol == DECIDE else LEAN_METHOD
            require(row['is_local'] and not row['is_opaque'] and row['lean_file'] == LIB+'/Funs.lean'
                    and row['lean_name'] == expected_name and 'Structured' in body['body']
                    and row['source'] == dict(file=str(relative), begin_line=start['line'], end_line=end['line']),
                    'detached generated function: '+symbol)
            if symbol == METHOD:
                require(row['source']['begin_line'] == selected['start']['line'] and
                        selected['source_sha256'] == sha(root/HOOK_SOURCE), 'parent selected another hook')
        hashes = {n: sha(gen/n) for n in sorted(GENERATED)}
        require(all(hashes[LIB+'/'+n+'.lean'] == h for n, h in PAGES[mode].items()),
                'unreviewed '+mode+' generated page')
        result[mode] = {'ir': sha(folder/'actual.llbc'), 'generated': hashes}
    return result
