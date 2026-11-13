"""Reuse the exact existing value models and kernel-safe literal adapter."""
from hashlib import sha256
from pathlib import Path
import importlib.util

HERE = Path('verification/experiments/stage1-protocol/inbox-save')
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
GRAPHIC = Path('verification/aeneas/stage1-protocol/graphic/SandproofStage1Graphic')
SOURCE = 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'


def parent(root):
    spec = importlib.util.spec_from_file_location('save_existing_bindings',
        root/HERE.parent/'inbox-validation/library_binding.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def sha(path):
    return sha256(path.read_bytes()).hexdigest()


def fragment(text, name, attributes=False):
    return parent(Path(__file__).resolve().parents[4]).fragment(text,name,attributes)


def check_literal(root):
    source = (root/SOURCE).read_text()
    literal = (root/HERE/'LiteralBinding.lean.in').read_text()
    if literal.strip() != fragment(source,'toStr').strip():
        raise ValueError('changed existing literal adapter')
    return literal


def check(root):
    inherited = parent(root).check(root)
    check_literal(root)
    return dict(previous=inherited, literal_provider=SOURCE, literal_provider_sha256=sha(root/SOURCE),
        literal_fragment_sha256=sha(root/HERE/'LiteralBinding.lean.in'),
        new_business_models=False, original_equivalence_proved=False)


def install(root, project):
    check(root)
    installed = parent(root).install(root,project)
    target = project/GENERATED/'FunsExternal.lean'
    source = target.read_text()
    marker = 'namespace SandproofStage1RequestRust\n'
    if source.count(marker) != 1 or 'def toStr ' in source:
        raise ValueError('ambiguous literal binding insertion')
    literal = (root/HERE/'LiteralBinding.lean.in').read_text().rstrip()
    target.write_text(source.replace(marker,marker+
        '\n-- Reuse the fixed Foundations literal adapter without altering generated bodies.\n'+literal+'\n',1))
    installed[str(target.relative_to(project))] = sha(target)
    return installed


def result_tags(raw):
    return parent(Path(__file__).resolve().parents[4]).result_tags(raw)
