"""Exercise the repaired allocator without loading weights or running inference."""
import hashlib
import json
from pathlib import Path
import resource
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from benchmarks.oracle_repairs import prepare_repaired_task, repair_solution
from tools.benchmark_tasks import InvalidInput


C_SOURCE = '''#include<stdio.h>
#include<stdlib.h>
int main(void){
int V=768,C=12,zz=512;
void*D;
char*bpe;
D=malloc(2LL*V*V*C*zz);
bpe=malloc(1e9);
long long bytes=2LL*V*V*C*zz;
volatile unsigned char*arena=D;
arena[0]=17;arena[bytes-1]=23;
bpe[0]=41;bpe[999*49999+998]=43;
if(arena[0]!=17||arena[bytes-1]!=23||bpe[0]!=41||bpe[999*49999+998]!=43)return 79;
printf("payload-ok\\n");
return 0;
}
'''
SCRIPT = "cat <<'EOF' > gpt2.c\n" + C_SOURCE + 'EOF\n'


class Gpt2ArenaTests(unittest.TestCase):
    def test_sparse_arena_access_and_both_allocation_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / 'source'
            for directory in ('solution', 'tests', 'environment'):
                (source / directory).mkdir(parents=True)
            (source / 'solution/solve.sh').write_text(SCRIPT)
            for filename, content in {
                    'tests/test.sh': 'original verifier\n',
                    'task.toml': '[environment]\nmemory = "4G"\n',
                    'environment/model.fixture': 'unchanged synthetic model input\n'}.items():
                (source / filename).write_text(content)
            with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                    'gpt2-codegolf': hashlib.sha256(SCRIPT.encode()).hexdigest()}):
                repaired = prepare_repaired_task('gpt2-codegolf', source, root / 'repaired')
            subprocess.run(['/bin/bash', str(repaired / 'solution/solve.sh')], cwd=root, check=True)
            program = root / 'program'
            compiled = subprocess.run(['gcc', '-O3', str(root / 'gpt2.c'), '-lm', '-o', str(program)],
                                      capture_output=True, text=True, timeout=30)
            self.assertEqual(compiled.returncode, 0, compiled.stderr)
            # Far-apart addresses must work without materializing the arena.
            # Limits below the arena and below arena+vocabulary must fail before
            # touching the payload, even when MAP_NORESERVE is requested.
            for limit_gib, expected in ((9, 0), (1, 77), (7, 78)):
                with self.subTest(address_space_limit_gib=limit_gib):
                    def limits():
                        size = limit_gib * 1024**3
                        resource.setrlimit(resource.RLIMIT_AS, (size, size))
                        resource.setrlimit(resource.RLIMIT_CPU, (5, 5))
                        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
                    result = subprocess.run([str(program)], preexec_fn=limits,
                                            capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, expected, result.stderr)
                    self.assertEqual(result.stdout, '' if expected else 'payload-ok\n')
            self.assertEqual((source / 'solution/solve.sh').read_text(), SCRIPT)
            for filename in ('tests/test.sh', 'task.toml', 'environment/model.fixture'):
                self.assertEqual((source / filename).read_bytes(), (repaired / filename).read_bytes())
            evidence = json.loads((root / 'oracle-repair.json').read_text())
            self.assertEqual(evidence['original_files']['solve.sh'], hashlib.sha256(SCRIPT.encode()).hexdigest())
            self.assertEqual(evidence['repaired_files']['solve.sh'],
                             hashlib.sha256((repaired / 'solution/solve.sh').read_bytes()).hexdigest())

    def test_changed_source_and_ambiguous_allocator_are_rejected(self):
        for script, expected_hash in (
                (SCRIPT + '# changed\n', hashlib.sha256(SCRIPT.encode()).hexdigest()),
                (SCRIPT + C_SOURCE, hashlib.sha256((SCRIPT + C_SOURCE).encode()).hexdigest())):
            with self.subTest(script_bytes=len(script)), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                source = root / 'source'
                source.mkdir()
                (source / 'solve.sh').write_text(script)
                with patch.dict('benchmarks.oracle_repairs.REPAIRS', {'gpt2-codegolf': expected_hash}):
                    with self.assertRaisesRegex(InvalidInput, 'no longer matches'):
                        repair_solution('gpt2-codegolf', source, root / 'repaired')
                self.assertFalse((root / 'repaired').exists())
                self.assertEqual((source / 'solve.sh').read_text(), script)


if __name__ == '__main__':
    unittest.main()
