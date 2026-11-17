"""Same original Rust namespace-reference assertions in three actual libtests.

The original Rust fixtures use regular-file inodes and controlled proc-like
paths. This is not a real procfs/VM/KVM or a Lean/extraction acceptance run.
"""
import hashlib
import importlib.util
from pathlib import Path
import sys

HERE = Path('verification/experiments/namespace-reference')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('namespace_reference_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = FIXTURE = Path('sandbox-runtime/src/retirement.rs')
PACKAGE = 'sandbox-runtime'
PACKAGE_DIR = Path('sandbox-runtime')
CRATE = 'sandbox_runtime'
MUTANT = 'skip-descriptor-check'
KIND = 'namespace-reference-native'
FIXTURE_SCOPE = 'complete original cfg(test) module; production and tests share this file'
BEFORE = '            require_not_namespace_holder(&descriptor.path(), namespace)?;\n'
AFTER = '            let _ = descriptor.path();\n'
PROPERTY = ('linux::retirement::tests::'
            'namespace_scan_rejects_live_thread_and_open_fd_after_named_handle_disappears')
TESTS = (PROPERTY,
    'linux::retirement::tests::fd_to_namespace_transition_at_observation_boundary_is_rejected',
    'linux::retirement::tests::namespace_scan_does_not_confuse_inode_on_another_device_and_fails_unknown')


def test_module(project):
    text = (project / FIXTURE).read_bytes()
    marker = b'\n#[cfg(test)]\nmod tests {'
    if text.count(marker) != 1:
        raise ValueError('Require the complete unchanged original Rust test module')
    return marker + text.split(marker, 1)[1]


def fixture_digest(project):
    return hashlib.sha256(test_module(project)).hexdigest()


def mutate(project):
    path = project / SOURCE
    text = path.read_text()
    start = text.index('\nfn require_task_namespace_unheld(')
    end = text.index('\nfn numeric_proc_name(', start)
    before_tests = fixture_digest(project)
    if text.count(BEFORE) != 1 or text[start:end].count(BEFORE) != 1:
        raise ValueError('Require the unique actual descriptor identity check in its original function')
    path.write_text(text.replace(BEFORE, AFTER, 1))
    if fixture_digest(project) != before_tests:
        raise ValueError('Mutation changed the original Rust tests')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    marker = ('        // The task need not be inside the namespace to keep it alive.\n'
              '        assert!(require_namespace_unheld(temp.path(), &namespace).is_err());')
    if text.count(marker) != 1 or PROPERTY.rsplit('::', 1)[1] not in text:
        raise ValueError('Require the exact original FD-only rejection assertion')
    return text[:text.index(marker)].count('\n') + 2


def checked_execution(stdout, stderr, code, name, rejected, line):
    runner.checked_execution(stdout, stderr, code, name, rejected, line, fixture=FIXTURE,
        assertion=b'require_namespace_unheld(temp.path(), &namespace).is_err()')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual Runtime native entry requires Linux')
    runner.main(case=sys.modules[__name__])
