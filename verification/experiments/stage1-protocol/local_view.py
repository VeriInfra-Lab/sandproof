"""Install reviewed proof-only inputs in a private copy; never patch production.

All business functions remain the copied current Rust. The extra Rust module
contains only transparent extraction roots. The active Lake view is independent
of the public V0 view, with the same tool/dependency revision. Its generated
Foundations library retains the narrow original Rust-name style setting.
"""
from pathlib import Path
import hashlib
import shutil

HERE = Path('verification/experiments/stage1-protocol')
LEVELS = ('schema-leaves', 'envelope-schema', 'envelope-bytes', 'envelope-budget',
          'envelope-writer')
ENCODING = Path('SandboxModel/Boundaries/JsonRequest/Encoding.lean')
EXTRACT_ROOT = Path('verification/extract-foundations/src/lib.rs')
ROLE_ROOT = EXTRACT_ROOT.parent / 'protocol_leaves.rs'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def install(project, level):
    if level not in LEVELS:
        raise ValueError('unknown independent proof view')
    changed = {}
    # This is the private copy's active configuration. The caller's source
    # lakefile and original public generated providers are never modified.
    template = project / HERE / 'local-view.toml.in'
    shutil.copyfile(template, project / 'lakefile.toml')
    changed['lakefile.toml'] = sha(project / 'lakefile.toml')
    root = project / EXTRACT_ROOT
    text = root.read_text()
    if 'pub mod protocol_leaves;' in text:
        raise ValueError('private role root already installed')
    root.write_text(text + '\npub mod protocol_leaves;\n')
    role = project / ROLE_ROOT
    if role.exists():
        raise ValueError('private role root must be fresh')
    shutil.copyfile(project / HERE / 'schema-leaves/protocol_leaves.rs.in', role)
    for path in (EXTRACT_ROOT, ROLE_ROOT):
        changed[str(path)] = sha(project / path)
    # Output syntax only: the original constructors retain their meanings.
    # Signed time and booleans are required by the existing full-field model.
    if level != 'schema-leaves':
        encoding = project / ENCODING
        before = encoding.read_text()
        needle = '  | unsigned (value : Nat)\n'
        if before.count(needle) != 1 or '  | signed ' in before or '  | boolean ' in before:
            raise ValueError('unreviewed shared output syntax')
        encoding.write_text(before.replace(needle, needle +
            '  | signed (value : Int)\n  | boolean (value : Bool)\n', 1))
        changed[str(ENCODING)] = sha(encoding)
    stems = [('schema-leaves', 'RoleLeaves'), ('envelope-schema', 'EnvelopeSchema'),
             ('envelope-bytes', 'EnvelopeBytes'), ('envelope-budget', 'EnvelopeBudget')]
    for part, stem in stems:
        if LEVELS.index(part) > LEVELS.index(level):
            continue
        target = project / 'SandboxModel/Lifecycle/Protocol' / (stem + '.lean')
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(project / HERE / part / (stem + '.lean.in'), target)
        changed[str(target.relative_to(project))] = sha(target)
    return changed


def role_provider(original):
    """The original provider plus the separately extracted std Result::map."""
    text = original.read_text()
    if 'SandproofProtocolResult' in text or 'abbrev core.result.Result.map' in text:
        raise ValueError('original V0 provider unexpectedly includes the private map view')
    return 'import SandproofProtocolResult.Funs\n' + text + (
        '\nnamespace Aeneas.Std\n'
        'abbrev core.result.Result.map := @SandproofProtocolResultRust.core.result.Result.map\n'
        'end Aeneas.Std\n')
