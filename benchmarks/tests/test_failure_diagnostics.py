"""Execute the allocation probe with only the pinned image's command set."""
import hashlib
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import tempfile
import unittest

from benchmarks.failure_diagnostics import GPT2_COMMAND, GPT2_SOURCE_SHA256


class Gpt2AllocationProbeTests(unittest.TestCase):
    def run_probe(self, *, large_arena=False, limit=False, wrong_source=False,
                  compiler_failure=False):
        # Synthetic program: matching allocation boundaries followed by an
        # observable side effect that the diagnostic must never reach.
        source = ('#include<stdio.h>\n#include<stdlib.h>\n'
                  'int main(void){\n'
                  f'int V={768 if large_arena else 1},C=12,zz=512;\n'
                  'void *D; char *bpe;\n'
                  'D=malloc(2LL*V*V*C*zz);\n'
                  'bpe=malloc(1e9);\n'
                  'FILE *fp=fopen("inference-reached", "w"); fclose(fp);\n'
                  'return 99;\n}\n')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commands = root / 'bin'
            commands.mkdir()
            for name in ('sha256sum', 'mktemp', 'rm', 'cat', 'sed', 'gcc', 'as', 'ld'):
                target = shutil.which(name)
                self.assertIsNotNone(target, name)
                (commands / name).symlink_to(target)
            (commands / 'mktemp').unlink()
            (commands / 'mktemp').write_text(
                '#!/bin/sh\nset -eu\n'
                f'work=$({shlex.quote(shutil.which("mktemp"))} "$@")\n'
                f'printf "%s\\n" "$work" > {shlex.quote(str(root / "temporary-path"))}\n'
                'printf "%s\\n" "$work"\n')
            (commands / 'mktemp').chmod(0o755)
            if compiler_failure:
                (commands / 'gcc').unlink()
                (commands / 'gcc').write_text('#!/bin/sh\nexit 42\n')
                (commands / 'gcc').chmod(0o755)
            (root / 'gpt2.c').write_text(source)
            expected = hashlib.sha256(source.encode()).hexdigest()
            command = GPT2_COMMAND.replace(GPT2_SOURCE_SHA256,
                                           '0' * 64 if wrong_source else expected)
            # Limit virtual address space, not physical host memory. This
            # deterministically rejects the large allocation without touching it.
            if limit:
                command = 'ulimit -v 131072\n' + command
            result = subprocess.run(['/bin/bash', '-c', command], cwd=root,
                                    env={**os.environ, 'PATH': str(commands)},
                                    text=True, capture_output=True, timeout=30)
            self.assertEqual((root / 'gpt2.c').read_text(), source)
            self.assertFalse((root / 'inference-reached').exists())
            if wrong_source:
                self.assertFalse((root / 'temporary-path').exists())
            else:
                private_path = Path((root / 'temporary-path').read_text().strip())
                self.assertFalse(private_path.exists())
            return result

    def test_no_python_success_stops_before_inference(self):
        result = self.run_probe()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('allocation_D bytes=12288 success=1 errno=0', result.stderr)
        self.assertIn('allocation_bpe bytes=1000000000 success=1 errno=0', result.stderr)
        self.assertIn('original_source_unchanged_after_probe=true', result.stdout)

    def test_arena_failure_returns_before_second_allocation(self):
        result = self.run_probe(large_arena=True, limit=True)
        self.assertEqual(result.returncode, 77, result.stderr)
        self.assertIn('allocation_D bytes=7247757312 success=0 errno=12', result.stderr)
        self.assertNotIn('allocation_bpe', result.stderr)
        self.assertIn('original_source_unchanged_after_probe=true', result.stdout)

    def test_second_allocation_failure_is_distinct(self):
        result = self.run_probe(limit=True)
        self.assertEqual(result.returncode, 78, result.stderr)
        self.assertIn('allocation_D bytes=12288 success=1 errno=0', result.stderr)
        self.assertIn('allocation_bpe bytes=1000000000 success=0 errno=12', result.stderr)

    def test_unreviewed_source_is_rejected_before_compiling(self):
        result = self.run_probe(wrong_source=True, compiler_failure=True)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertNotIn('scope=', result.stdout)

    def test_compile_failure_is_preserved(self):
        result = self.run_probe(compiler_failure=True)
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertIn('original_source_unchanged_after_probe=true', result.stdout)
