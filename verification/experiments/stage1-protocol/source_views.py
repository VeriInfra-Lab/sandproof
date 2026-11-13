"""Thin extraction roots and exact mutations of the adopted production source.

Correct views add calls only. Business implementations always come from the
current production tree, never from a historical candidate or generated body.
"""
from hashlib import sha256
import json
from pathlib import Path
import re
import subprocess


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def inputs(project, package):
    inventory = json.loads((project / package / 'inventory.json').read_text())
    expected = dict(inventory['production_inputs'])
    expected.update(inventory.get('original_inputs', {}))
    for name, digest in expected.items():
        if sha(project / name) != digest:
            raise ValueError('adopted source input differs: ' + name)
    names = {*expected, 'verification/experiments/stage1-protocol/inbox-record/lib.rs'}
    return {name: sha(project / name) for name in names}


def append(project, name, fragment):
    source = project / name
    text = source.read_text()
    functions = re.findall(r'\bfn\s+(\w+)\s*(?:<|\()', fragment)
    if not functions or any(re.search(r'\bfn\s+' + function + r'\s*(?:<|\()', text)
                            for function in functions):
        raise ValueError('missing or already installed forwarding root: ' + name)
    source.write_text(text.rstrip() + '\n\n' + fragment.strip() + '\n')


def edit(project, name, edits, scope=None):
    """Mutate a unique actual expression, optionally within one named function."""
    path = project / name
    before = path.read_text()
    start, end = 0, len(before)
    if scope is not None:
        if before.count(scope) != 1:
            raise ValueError('nonunique mutation function: ' + scope)
        start = before.index(scope)
        opening = before.index('{', start)
        depth = 1
        end = opening + 1
        # These registered mutation targets contain no braces in literals.
        while depth:
            depth += (before[end] == '{') - (before[end] == '}')
            end += 1
    body = before[start:end]
    for old, new in edits:
        if body.count(old) != 1 or old == new:
            raise ValueError('nonunique actual mutation expression: ' + old)
        body = body.replace(old, new, 1)
    after = before[:start] + body + before[end:]
    path.write_text(after)
    return dict(path=name, edits=[list(pair) for pair in edits], scope=scope,
                before=sha256(before.encode()).hexdigest(),
                after=sha256(after.encode()).hexdigest())


def finish(project, before, mutation=None):
    changed = [name for name, digest in before.items() if sha(project / name) != digest]
    rust = [name for name in changed if name.endswith('.rs')]
    if rust:
        subprocess.run(['rustfmt', '--edition', '2024', *rust], cwd=project, check=True)
    return ({name: sha(project / name) for name in changed},
            dict(original_inputs=before, mutation=mutation, production_adopted=True,
                 production_modified=False, private_refactor=False,
                 isolated_production_mutation=mutation is not None,
                 original_equivalence_proved=False,
                 standard_provider_connection_proved=False))
