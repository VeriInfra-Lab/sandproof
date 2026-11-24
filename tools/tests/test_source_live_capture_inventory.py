"""Check the committed SourceLive capture inputs without running extraction."""
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest


class SourceLiveCaptureInventoryTests(unittest.TestCase):
    def test_capture_preflight_accepts_committed_native_inputs(self):
        root = Path(__file__).resolve().parents[2]
        capture = root / 'verification/experiments/stage1-protocol/source-live/capture.py'
        with TemporaryDirectory() as directory:
            output = Path(directory) / 'capture-admission'
            run = subprocess.run(
                [sys.executable, str(capture), '--repo', str(root),
                 '--check-only', '--output', str(output)],
                cwd=root, capture_output=True, text=True,
            )
            self.assertEqual(run.returncode, 0, run.stderr)
            result = json.loads((output / 'result.json').read_text())

        self.assertEqual(tuple(result['views']),
                         ('correct', 'skip-version', 'allow-expired', 'ignore-startup'))
        self.assertIs(result['input_admission_complete'], True)
        for field in ('complete', 'capture_complete', 'formal_acceptance',
                      'formal_counterexamples_reached', 'ning_acceptance', 'full_v3'):
            self.assertIs(result[field], False, field)
