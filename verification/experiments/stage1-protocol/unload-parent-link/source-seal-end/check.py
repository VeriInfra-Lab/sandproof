"""Run both complete views through the unchanged full parent checker.

Only the materializer, fixed variant inventory and owned registration path
are supplied. All role/lock/build/actual-file/audit/reproduction/dependency
checks remain. This never signs ordinary, native or public acceptance.
"""
import argparse
import importlib.util
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


prepare = module('source_seal_end_complete_prepare', HERE / 'prepare.py')
checker = module('source_seal_end_parent_checker', HERE.parent / 'source-remove-retired/check.py')
checker.HERE = HERE
checker.prepare = prepare
checker.extract = SimpleNamespace(VARIANTS=prepare.capture.VARIANTS,
    REPO=prepare.validation.REPO, SOURCE=prepare.capture.retire.SOURCE, TREE=prepare.capture.retire.TREE)


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('inputs', 'output', 'lock', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=name != 'output'))
    raise SystemExit(checker.check(args))
