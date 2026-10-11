"""Explicit portable-development versus required Linux tool-test selection.

Linux always discovers the entire original suite. The portable selection does
not turn tests into skips or relax a production assertion; it reports the
excluded Linux groups and cannot be presented as complete functional acceptance.
"""
import argparse
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
# These groups exercise original Linux production primitives, rather than a
# substitute Mach-O helper, sendfile API or filesystem accounting algorithm.
LINUX_GROUPS = {
    'test_template_build_windows.WindowBridgeTests': 'ELF and /proc/self/fd execution of the checked inode',
    'test_template_build_service.BuildTests': 'real Linux request-window helper',
    'test_template_build_retention.RetentionTests': 'real Linux request-window helper and retained results',
    'test_template_acceptance.AcceptancePressureTests': 'real helper at full build capacity',
    'test_terminal_build_observation.TerminalBuildObservationTests': 'real helper and expiry observation',
    'test_template_handoff.DownloadHandoffTests': 'Linux private inode handoff and sparse extent copy',
    'test_template_stage.StagingTests': 'Linux sendfile/extent-based publication staging',
    'test_template_sparse.SparseArtifactTests': 'Linux sendfile semantics and allocated-block guarantees',
    'test_template_physical_storage.PhysicalProfileTests': 'Linux shared-filesystem lease and block rounding',
}


def leaves(suite):
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            yield from leaves(test)
        else:
            yield test


def partition(suite, mode):
    selected, excluded = [], {}
    for test in leaves(suite):
        group = '.'.join(test.id().split('.')[:2])
        if mode == 'portable' and group in LINUX_GROUPS:
            excluded[group] = excluded.get(group, 0) + 1
        else:
            selected.append(test)
    return unittest.TestSuite(selected), excluded


def preflight(mode):
    if not (3, 12) <= sys.version_info[:2] < (3, 14):
        raise ValueError('Python >=3.12,<3.14 required')
    if sys.platform not in ('linux', 'darwin'):
        raise ValueError('development checks support Linux or macOS only')
    if mode == 'linux' and sys.platform != 'linux':
        raise ValueError('full functional checks require Linux; use --portable for partial development checks')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=('portable', 'linux'), default='linux')
    parser.add_argument('--preflight-only', action='store_true')
    args = parser.parse_args(argv)
    try:
        preflight(args.mode)
    except ValueError as error:
        parser.error(str(error))
    if args.preflight_only:
        print(f'Python {sys.version.split()[0]}; tool-test mode={args.mode}')
        return 0
    sys.path.insert(0, str(ROOT))
    suite = unittest.defaultTestLoader.discover(str(ROOT / 'tools/tests'))
    chosen, excluded = partition(suite, args.mode)
    for group, count in sorted(excluded.items()):
        print(f'NOT RUN ({count}): {group}: {LINUX_GROUPS[group]}', file=sys.stderr)
    print(f'tool-test mode={args.mode}; selected={chosen.countTestCases()}; '
          f'Linux-only not run={sum(excluded.values())}', file=sys.stderr)
    if not chosen.countTestCases():
        parser.error('empty test selection is not acceptance')
    result = unittest.TextTestRunner(verbosity=2).run(chosen)
    # Environment-dependent skips in the required Linux suite are not green
    # acceptance (for example, missing ldd or a C compiler).
    if args.mode == 'linux' and result.skipped:
        print('Required Linux tests skipped; complete functional checks NOT passed.', file=sys.stderr)
        return 1
    if args.mode == 'portable':
        print('Partial development checks only; Linux functional acceptance NOT run.', file=sys.stderr)
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    raise SystemExit(main())
