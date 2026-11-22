"""Capture the unchanged original full setter; retain the strict30 rejection."""
import argparse
import importlib.util
from pathlib import Path
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('source_set_end_finish_original', HERE.parent/'source-set-end/capture.py')
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
ROOT, NAMESPACE, OPTIONS, STAGED, END, VARIANTS = base.ROOT, base.NAMESPACE, base.OPTIONS, base.STAGED, base.END, base.VARIANTS
LIBRARIES, retire, parent, inputs = base.LIBRARIES, base.retire, base.parent, base.inputs
reconstruct, admit = base.reconstruct, base.admit
if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', choices=VARIANTS, required=True)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--'+name, type=Path, required=True)
    args = cli.parse_args()
    base.capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
