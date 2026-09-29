"""Reference repair integrity and residue parsing, without running task code."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

from benchmarks.oracle_repairs import repair_solution
from benchmarks.protein_sequences import expand_sequence, get_single_fasta, get_component
from tools.benchmark_tasks import InvalidInput


class OracleRepairTests(unittest.TestCase):
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
