"""Exercise the repaired exit hook with synthetic C, never benchmark code."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.oracle_repairs import prepare_repaired_task, repair_solution
from tools.benchmark_tasks import InvalidInput


SCRIPT = '''set -e
echo "void unix_init_POVRAY();" >> config.h
"$POVRAY_TEST_CC" -std=c89 -Werror=return-type render.c -o render
./render "$@"
'''
# The source archive's platform contract invokes FINISH_POVRAY only after
# normal rendering/cleanup. This fixture checks that contract independently.
RENDER = '''#include <stdio.h>
#include "config.h"
#ifndef FINISH_POVRAY
#define FINISH_POVRAY
#endif
static int render(int fail) {
    if (fail) return 23;
    puts("render and cleanup completed");
    FINISH_POVRAY
}
int main(int argc, char **argv) { return render(argc > 1); }
'''


class PovrayExitTests(unittest.TestCase):
    def test_normal_completion_returns_zero_and_real_failure_is_preserved(self):
        cc = shutil.which('cc')
        self.assertIsNotNone(cc, 'C compiler required for the C89 exit regression')
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            original = root / 'original'
            (original / 'solution').mkdir(parents=True)
            (original / 'tests').mkdir()
            (original / 'solution/solve.sh').write_text(SCRIPT)
            (original / 'tests/test.sh').write_text('original scoring\n')
            (original / 'task.toml').write_text('[agent]\ntimeout_sec = 3600.0\n')
            with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                    'build-pov-ray': hashlib.sha256(SCRIPT.encode()).hexdigest()}):
                repaired = prepare_repaired_task('build-pov-ray', original, root / 'repaired')
            baseline = root / 'baseline'
            baseline.mkdir()
            (baseline / 'config.h').write_text('void unix_init_POVRAY();\n')
            (baseline / 'render.c').write_text(RENDER)
            check = subprocess.run([cc, '-std=c89', '-Werror=return-type', 'render.c', '-o', 'render'],
                                   cwd=baseline, capture_output=True, text=True)
            self.assertNotEqual(check.returncode, 0, 'empty normal-exit hook must lack a return')
            for args, code, output in (([], 0, 'render and cleanup completed\n'), (['fail'], 23, '')):
                with self.subTest(args=args):
                    work = root / ('failure' if args else 'success')
                    work.mkdir()
                    (work / 'render.c').write_text(RENDER)
                    result = subprocess.run(['/bin/bash', str(repaired / 'solution/solve.sh'), *args],
                        cwd=work, env={**os.environ, 'POVRAY_TEST_CC': cc},
                        capture_output=True, text=True)
                    self.assertEqual(result.returncode, code, result.stderr)
                    self.assertEqual(result.stdout, output)
            self.assertEqual((original / 'solution/solve.sh').read_text(), SCRIPT)
            for relative in ('tests/test.sh', 'task.toml'):
                self.assertEqual((repaired / relative).read_bytes(), (original / relative).read_bytes())
            evidence = json.loads((root / 'oracle-repair.json').read_text())
            self.assertEqual(evidence['repaired_files']['solve.sh'],
                             hashlib.sha256((repaired / 'solution/solve.sh').read_bytes()).hexdigest())
            self.assertIn('#define FINISH_POVRAY return 0;', evidence['diff'])

    def test_missing_or_ambiguous_platform_setup_is_rejected(self):
        marker = 'echo "void unix_init_POVRAY();" >> config.h'
        for script in ('set -e\n', marker + '\n' + marker + '\n'):
            with self.subTest(script=script), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / 'source'
                source.mkdir()
                (source / 'solve.sh').write_text(script)
                with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                        'build-pov-ray': hashlib.sha256(script.encode()).hexdigest()}):
                    with self.assertRaises(InvalidInput):
                        repair_solution('build-pov-ray', source, root / 'repaired')
                self.assertFalse((root / 'repaired').exists())


if __name__ == '__main__':
    unittest.main()
