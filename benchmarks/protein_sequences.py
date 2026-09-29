"""Recover parent residues from RCSB's public chemical component dictionary.

Used only by the explicitly selected repaired reference solution inside a VM.
No target sequence or task answer is embedded here.
"""
import re
from functools import lru_cache

import requests

AMINO_ACIDS = frozenset('ACDEFGHIKLMNPQRSTVWY')


def expand_sequence(encoded, component):
    encoded = ''.join(encoded.split())
    tokens = re.findall(r'\([A-Z0-9]+\)|[A-Z]', encoded)
    if ''.join(tokens) != encoded:
        raise ValueError('unrecognized polymer sequence syntax')
    result = []
    for token in tokens:
        letters = component(token[1:-1]) if token.startswith('(') else token
        if not letters or not set(letters) <= AMINO_ACIDS:
            raise ValueError('unresolved or ambiguous parent residue')
        result.append(letters)
    return ''.join(result)


@lru_cache(maxsize=128)
def component_record(code):
    if not re.fullmatch('[A-Z0-9]+', code):
        raise ValueError('invalid chemical component identifier')
    response = requests.get(f'https://data.rcsb.org/rest/v1/core/chemcomp/{code}', timeout=20)
    response.raise_for_status()
    return response.json()['chem_comp']


def get_component(code):
    record = component_record(code)
    if record.get('one_letter_code'):
        return record['one_letter_code']
    # Some compound chromophores have only an ordered parent-residue list.
    parents = record.get('mon_nstd_parent_comp_id') or []
    letters = [component_record(parent).get('one_letter_code') for parent in parents]
    if not letters or any(not letter or not set(letter) <= AMINO_ACIDS for letter in letters):
        raise ValueError('component has no unambiguous parent-residue sequence')
    return ''.join(letters)


def get_single_fasta(pdb_id):
    response = requests.get(f'https://data.rcsb.org/rest/v1/core/entry/{pdb_id}', timeout=20)
    response.raise_for_status()
    entities = response.json()['rcsb_entry_container_identifiers']['polymer_entity_ids']
    # Keep the reference solution's single-polymer selection policy.
    if len(entities) != 1:
        return None
    response = requests.get(
        f'https://data.rcsb.org/rest/v1/core/polymer_entity/{pdb_id}/{entities[0]}', timeout=20)
    response.raise_for_status()
    polymer = response.json()['entity_poly']
    if polymer['type'] != 'polypeptide(L)':
        return None
    sequence = expand_sequence(polymer['pdbx_seq_one_letter_code'], get_component)
    print(f'RCSB parent-residue sequence {pdb_id}: {len(sequence)} residues', flush=True)
    return sequence
