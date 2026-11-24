import argparse
from collections import Counter
from dataclasses import dataclass, field
from hashlib import sha256
import json
from pathlib import Path

if not __debug__:
    raise RuntimeError('run the LLBC checks with Python assertions enabled')

parser = argparse.ArgumentParser(description='Check the retained CloseWindow transaction flow.')
parser.add_argument('capture', type=Path)
parser.add_argument('--source-commit', default='207ab640325e6deff97c4687116bee7123dbedcf')
arguments = parser.parse_args()
capture = arguments.capture
raw = json.loads(capture.read_bytes())
assert raw['has_errors'] is False
translated = raw['translated']
options = translated['options']
assert not any(options[name] for name in ('skip_borrowck', 'no_typecheck', 'monomorphize'))
source_inputs = {
    'controller-service/src/controller/lifecycle_inbox/window.rs':
        '68c4b7b1865cc6b3b71f0c6fee687d3b45965122e0c89b85c2c88fb125ac145e',
    'controller-service/src/controller/branching.rs':
        '74df2ce73bc26fe6484788fecc3b309d69e45df64d550a56ca2b40cf8a681ac3',
}
for file, digest in source_inputs.items():
    embedded, = (record for record in translated['files']
                 if record['name'] == {'Local': file})
    assert sha256(embedded['contents'].encode()).hexdigest() == digest, file

functions = {entry['def_id']: entry for entry in translated['fun_decls'] if entry}

def full_name(entry):
    return tuple(item['Ident'][0] for item in entry['item_meta']['name'] if 'Ident' in item)

def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)

constants = {}
for node in nodes(translated):
    value = node.get('Value')
    if (isinstance(value, list) and len(value) == 2 and isinstance(value[1], list)
            and len(value[1]) == 2 and isinstance(value[1][0], dict)
            and 'Discriminant' in value[1][0]):
        constants[value[0]] = value[1]

controller = ('sandproof_controller_service', 'controller')
inbox = (*controller, 'lifecycle_inbox')
begin = (*controller, 'transaction', 'begin')
commit = (*controller, 'transaction', 'commit')
retire = (*controller, 'branching', 'retire_closed_parent')
save = (*inbox, 'codec', 'save')
cleaned = (*inbox, 'access', 'destroy_cleaned')
world_deref = (*controller, 'transaction', 'deref')
sqlite_deref = ('rusqlite', 'transaction', 'deref')
try_branch = ('core', 'result', 'branch')

root, = (entry for entry in functions.values()
         if full_name(entry) == (*inbox, 'window', 'close_lifecycle_window')
         and entry['item_meta']['name'][-1] == {'Ident': ['close_lifecycle_window', 0]})
assert root['item_meta']['opacity'] == 'Transparent'
block = root['body']['Structured']['body']
outcomes = []

@dataclass
class State:
    # T is the value returned by this path's exact WorldTransaction::begin.
    # The call receiver changes type when shared deref selects its transaction
    # and connection, but the underlying origin tag remains the same.
    values: dict[int, str] = field(default_factory=dict)
    events: tuple[str, ...] = ()

    def fork(self):
        return State(dict(self.values), self.events)

    def emit(self, name):
        self.events += (name,)

    def origin(self, place):
        kind = place['kind']
        if 'Projection' in kind:
            return self.origin(kind['Projection'][0])
        return self.values.get(kind.get('Local'))

    def place(self, destination, identity):
        kind = destination['kind']
        if 'Local' in kind:
            local = kind['Local']
            if identity is None:
                self.values.pop(local, None)
            else:
                self.values[local] = identity
        elif self.origin(destination) == 'T':
            raise ValueError('unexpected field assignment to transaction')

    def operand(self, incoming):
        for kind in ('Move', 'Copy'):
            if kind in incoming:
                return self.origin(incoming[kind])
        return None

    def assign(self, change):
        destination, value = change
        if 'Use' in value:
            identity = self.operand(value['Use'][0])
        elif 'Ref' in value:
            identity = self.origin(value['Ref']['place'])
        else:
            identity = None
        self.place(destination, identity)

    def call(self, operation):
        call = operation['call']
        fun = call['func'].get('Regular', {}).get('kind', {}).get('Fun')
        name = full_name(functions[fun]) if fun in functions else ()
        argument = self.operand(call['args'][0]) if call['args'] else None
        produced = None
        if name == begin:
            assert 'begin' not in self.events
            self.emit('begin')
            produced = 'T'
        elif name in (world_deref, sqlite_deref, try_branch):
            produced = argument
        elif name in (cleaned, retire, save, commit):
            assert argument == 'T' and 'begin' in self.events, name
            self.emit(name[-1])
            if name == retire:
                produced = 'retire-result'
            elif name == save:
                produced = 'save-result'
            elif name == commit:
                produced = 'commit-result'
        self.place(call['dest'], produced)

    def branch(self, selector, branch_index):
        # CloseWindow's Result::branch switches discriminate on the exact
        # ControlFlow type. Resolve the actual variant targeted by each arm.
        # Track only the three relevant Results so unrelated numeric and
        # bool switches stay opaque.
        info = selector['data']
        discriminant = info.get('scrutinee', {}).get('Discriminant')
        identity = self.origin(discriminant) if discriminant is not None else None
        if identity not in ('retire-result', 'save-result', 'commit-result'):
            return
        arms = {}
        for explicit, index in info['branches']:
            terms = (
                constants[explicit['Deduplicated']] if 'Deduplicated' in explicit
                else explicit['Value'][1]
            )
            control_type, number = terms[0]['Discriminant']
            decl = type_decls[control_type['id']]
            assert full_name(decl) == ('core', 'ops', 'control_flow', 'ControlFlow')
            arms[index] = decl['kind']['Enum'][number]['name']
        missing = {'Continue', 'Break'}.difference(arms.values())
        if info['fallback'] is not None:
            assert len(missing) == 1
            arms[info['fallback']], = missing
        else:
            assert not missing
        self.emit(identity.replace('-result', '-ok' if arms[branch_index] == 'Continue' else '-err'))

type_decls = {entry['def_id']: entry for entry in translated['type_decls'] if entry}

def execute(scope, state):
    live = [state]
    for statement in scope['statements']:
        flow = []
        for current in live:
            effect = statement['kind']
            if effect == 'Return':
                outcomes.append(current.events)
                continue
            if isinstance(effect, str):
                raise ValueError('unhandled terminal instruction: ' + effect)
            ((kind, data),) = effect.items()
            if kind == 'Switch':
                for index, arm in enumerate(data['branches']):
                    split = current.fork()
                    split.branch(data, index)
                    flow.extend(execute(arm, split))
                continue
            if kind == 'Assign':
                current.assign(data)
            elif kind == 'Call':
                current.call(data)
            elif kind == 'StorageDead':
                current.values.pop(data, None)
            elif kind not in ('StorageLive', 'Drop', 'Borrowck', 'PlaceMention'):
                raise ValueError('unhandled instruction: ' + kind)
            flow.append(current)
        live = flow
    return live

assert not execute(block, State()), 'method falls through without returning'
scopes = Counter(outcomes)
assert any('retire-ok' in path and 'commit-ok' in path for path in scopes)
assert any('retire-err' in path for path in scopes)
assert any('save-err' in path for path in scopes)
for path in scopes:
    if 'retire_closed_parent' in path:
        assert path.index('destroy_cleaned') < path.index('retire_closed_parent'), path
    if 'commit' in path:
        assert path.index('save-ok') < path.index('commit'), path
        assert path.count('begin') == path.count('commit') == 1, path
    if 'retire_closed_parent' in path and 'save' in path:
        assert path.index('retire-ok') < path.index('save'), path
    if 'retire-err' in path:
        assert 'save' not in path and 'commit' not in path, path

print(json.dumps(dict(
    source=arguments.source_commit,
    source_inputs=source_inputs,
    captured_root='::'.join(full_name(root)),
    artifact_sha256=sha256(capture.read_bytes()).hexdigest(),
    terminal_paths=sum(scopes.values()), distinct_event_traces=len(scopes),
    traces=[dict(events=path, count=count) for path, count in sorted(scopes.items())],
), indent=2))
