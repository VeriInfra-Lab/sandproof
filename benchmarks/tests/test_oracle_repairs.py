"""Reference repair integrity and residue parsing, without running task code."""
import asyncio
import contextlib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import AsyncMock, Mock, patch

from harbor.agents.oracle import OracleAgent
from harbor.models.agent.context import AgentContext
from harbor.models.trial.paths import TrialPaths
from harbor.models.task.task import Task

from benchmarks.oracle_repairs import prepare_repaired_task, repair_solution
from benchmarks.protein_sequences import expand_sequence, get_single_fasta, get_component
from tools.benchmark_tasks import InvalidInput


class OracleRepairTests(unittest.TestCase):
    def test_dependency_repairs_execute_in_order_and_preserve_failure(self):
        # Only synthetic shell fixtures run here. Commands are recording stubs;
        # no package manager, R compiler or benchmark solution runs on the host.
        cases = {
            'build-cython-ext': (
                'pip install -e .\n', ['pip install planarity==0.6 -e .']),
            'mcmc-sampling-stan': (
                'set -e\nsudo apt-get install -y \\\n    gfortran=4:13.2.0-7ubuntu1\nR model.R\n',
                ['apt-get install -y cmake gfortran=4:13.2.0-7ubuntu1', 'R model.R']),
            'make-doom-for-mips': (
                'apt install -y clang gcc-mips-linux-gnu g++-mips-linux-gnu llvm llvm-dev lld\nmake doom\n',
                ['apt-get update', 'apt install -y clang gcc-mips-linux-gnu g++-mips-linux-gnu llvm llvm-dev lld', 'make doom']),
            'build-pmars': (
                'set -e\napt-get update\napt-get install -y build-essential=12.12 libncurses-dev=6.5+20250216-2 ca-certificates=20250419 dpkg-dev=1.22.21 devscripts=2.25.15+deb13u1\napt-get source pmars\nmake\n',
                ['apt-get update', 'apt-get install -y build-essential libncurses-dev ca-certificates dpkg-dev devscripts', 'apt-get source pmars', 'make']),
        }
        for name, (script, expected) in cases.items():
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                source = root / 'source'
                source.mkdir()
                (source / 'solve.sh').write_text(script)
                with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                        name: hashlib.sha256(script.encode()).hexdigest()}):
                    _, solve = repair_solution(name, source, root / 'repaired')
                bin_dir = root / 'bin'
                bin_dir.mkdir()
                stub = ('#!/bin/bash\nname=${0##*/}\n'
                        'printf "%s %s\\n" "$name" "$*" >> "$REPAIR_TEST_LOG"\n'
                        'if [[ "$name $*" == "$REPAIR_TEST_FAIL" ]]; then exit 42; fi\n')
                for command in ('apt-get', 'apt', 'R', 'make', 'sudo', 'pip'):
                    path = bin_dir / command
                    path.write_text('#!/bin/bash\nexec "$@"\n' if command == 'sudo' else stub)
                    path.chmod(0o700)
                log = root / 'commands'
                env = {**os.environ, 'PATH': str(bin_dir), 'REPAIR_TEST_LOG': str(log),
                       'REPAIR_TEST_FAIL': ''}
                subprocess.run(['/bin/bash', str(solve)], env=env, check=True)
                self.assertEqual([line.rstrip() for line in log.read_text().splitlines()], expected)
                log.unlink()
                env['REPAIR_TEST_FAIL'] = expected[0]
                failed = subprocess.run(['/bin/bash', str(solve)], env=env)
                self.assertEqual(failed.returncode, 42)
                self.assertEqual(log.read_text().splitlines(), [expected[0]])
                self.assertEqual((source / 'solve.sh').read_text(), script)
                evidence = json.loads((root / 'oracle-repair.json').read_text())
                self.assertEqual(evidence['original_files']['solve.sh'], hashlib.sha256(script.encode()).hexdigest())
                self.assertEqual(evidence['repaired_files']['solve.sh'], hashlib.sha256(solve.read_bytes()).hexdigest())

    def test_historical_clone_skips_current_tree_but_keeps_full_history(self):
        # Execute only this synthetic fixture, never a benchmark solution.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            repo = root / 'upstream'
            def git(*args, cwd=repo):
                return subprocess.check_output(['git', *args], cwd=cwd, text=True).strip()
            git('init', '-q', str(repo), cwd=root)
            git('config', 'user.name', 'Synthetic Test')
            git('config', 'user.email', 'test@example.invalid')
            (repo / 'historical.txt').write_text('historical result\n')
            git('add', '.')
            git('commit', '-qm', 'historical fixture')
            historical = git('rev-parse', 'HEAD')
            (repo / 'current-only.txt').write_text('current tree\n' * 1024)
            git('add', '.')
            git('commit', '-qm', 'current fixture')
            current = git('rev-parse', 'HEAD')
            script = ('set -eu\n'
                      'git clone https://github.com/embeddings-benchmark/results\n'
                      'test ! -e results/current-only.txt\n'
                      'test ! -e results/historical.txt\n'
                      'cd results\n'
                      f'git checkout {historical}\n'
                      "cat > leaderboard.py <<'PY'\n"
                      'import os\nfrom pathlib import Path\n'
                      'assert Path(os.environ["MTEB_CACHE"]) / Path.cwd().name == Path.cwd()\n'
                      'PY\npython leaderboard.py\n')
            source = root / 'source'
            source.mkdir()
            (source / 'solve.sh').write_text(script)
            with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                    'mteb-leaderboard': hashlib.sha256(script.encode()).hexdigest()}):
                _, solve = repair_solution('mteb-leaderboard', source, root / 'repaired')
            env = {**os.environ, 'GIT_CONFIG_COUNT': '1',
                   'GIT_CONFIG_KEY_0': f'url.{repo.as_uri()}.insteadOf',
                   'GIT_CONFIG_VALUE_0': 'https://github.com/embeddings-benchmark/results'}
            subprocess.run(['bash', str(solve)], cwd=root, env=env, check=True,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            clone = root / 'results'
            self.assertEqual(git('rev-parse', 'HEAD', cwd=clone), historical)
            self.assertEqual(git('cat-file', '-t', current, cwd=clone), 'commit')
            self.assertEqual(git('rev-parse', '--is-shallow-repository', cwd=clone), 'false')
            self.assertFalse((clone / 'current-only.txt').exists())
            self.assertEqual((clone / 'historical.txt').read_text(), 'historical result\n')

    def test_real_harbor_agent_accepts_repaired_task_without_bypassing_containment(self):
        # Exercise pinned Harbor's actual input validation and upload path.
        # The environment records commands; no fixture script runs on the host.
        script = ('def get_single_fasta(pdb_id):\n    return "old"\n\n'
                  'def get_pdb_title(pdb_id):\n    pass\npython pdb_ids.py\n')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            (source / 'solution').mkdir(parents=True)
            (source / 'tests').mkdir()
            (source / 'solution/solve.sh').write_text(script)
            (source / 'tests/test.sh').write_text('# synthetic verifier\n')
            (source / 'task.toml').write_text('version = "1.0"\n')
            (source / 'instruction.md').write_text('Synthetic task.\n')
            with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                    'protein-assembly': hashlib.sha256(script.encode()).hexdigest()}):
                repaired = prepare_repaired_task('protein-assembly', source, root / 'repaired-task')
            original_task = Task(source)
            paths = TrialPaths(root / 'trial')
            paths.agent_dir.mkdir(parents=True)
            agent = OracleAgent(logs_dir=paths.agent_dir, task_dir=repaired, trial_paths=paths)
            env = SimpleNamespace(os=original_task.config.environment.os,
                                  capabilities=SimpleNamespace(mounted=False),
                                  upload_dir=AsyncMock(), download_file=AsyncMock(),
                                  exec=AsyncMock(return_value=SimpleNamespace(return_code=0)),
                                  scoped_exec_env=lambda _: contextlib.nullcontext())
            asyncio.run(agent.run(original_task.instruction, env, AgentContext()))
            env.upload_dir.assert_awaited_once_with(source_dir=(repaired / 'solution').resolve(), target_dir='/solution')
            self.assertTrue(env.exec.await_count > 0)
            self.assertFalse((paths.agent_dir / 'exit-code.txt').exists())
            self.assertEqual((source / 'solution/solve.sh').read_text(), script)
            self.assertEqual((repaired / 'tests/test.sh').read_bytes(), (source / 'tests/test.sh').read_bytes())
            with self.assertRaises(ValueError):
                original_task.paths.validate_input_tree(repaired / 'solution')

    def test_parent_residues_expand_without_losing_adjacent_sequence(self):
        component = Mock(side_effect=lambda code: {'AAA': 'ACD', 'BBB': 'M'}[code])
        self.assertEqual(expand_sequence('MA\n(AAA)GG(BBB)Y', component), 'MAACDGGMY')
        self.assertEqual(component.call_count, 2)

    def test_ambiguous_or_malformed_sequence_is_rejected(self):
        for sequence in ('MX', 'M(BAD)', 'M(AAA', 'MA*', 'M(AAA)'):
            with self.subTest(sequence=sequence), self.assertRaises(ValueError):
                expand_sequence(sequence, lambda code: 'X')

    def test_parent_list_handles_missing_compound_one_letter_code(self):
        records = {'CMP': {'mon_nstd_parent_comp_id': ['GLY', 'ALA']},
                   'GLY': {'one_letter_code': 'G'}, 'ALA': {'one_letter_code': 'A'}}
        with patch('benchmarks.protein_sequences.component_record', side_effect=records.__getitem__):
            self.assertEqual(get_component('CMP'), 'GA')

    def test_multiple_polymer_entities_are_not_silently_combined(self):
        response = Mock()
        response.json.return_value = {'rcsb_entry_container_identifiers': {'polymer_entity_ids': ['1', '2']}}
        with patch('benchmarks.protein_sequences.requests.get', return_value=response) as request:
            self.assertIsNone(get_single_fasta('TEST'))
        request.assert_called_once()

    def test_changed_source_is_rejected_before_copying(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            source.mkdir()
            (source / 'solve.sh').write_text('echo changed\n')
            with self.assertRaises(InvalidInput):
                repair_solution('protein-assembly', source, root / 'repaired')
            self.assertFalse((root / 'repaired').exists())

    def test_solution_copy_and_evidence_preserve_original(self):
        script = ('def get_single_fasta(pdb_id):\n    return "old"\n\n'
                  'def get_pdb_title(pdb_id):\n    pass\npython pdb_ids.py\n')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source'
            source.mkdir()
            (source / 'solve.sh').write_text(script)
            (source / 'extra.txt').write_text('unchanged')
            with patch.dict('benchmarks.oracle_repairs.REPAIRS', {
                    'protein-assembly': hashlib.sha256(script.encode()).hexdigest()}):
                destination, solve = repair_solution('protein-assembly', source, root / 'repaired')
            self.assertEqual((source / 'solve.sh').read_text(), script)
            self.assertEqual((destination / 'extra.txt').read_text(), 'unchanged')
            self.assertIn('from protein_sequences import get_single_fasta', solve.read_text())
            evidence = json.loads((root / 'oracle-repair.json').read_text())
            self.assertEqual(evidence['oracle_mode'], 'repaired')
            self.assertEqual(evidence['repaired_files']['solve.sh'], hashlib.sha256(solve.read_bytes()).hexdigest())
            self.assertNotEqual(evidence['original_files']['solve.sh'], evidence['repaired_files']['solve.sh'])


if __name__ == '__main__':
    unittest.main()
