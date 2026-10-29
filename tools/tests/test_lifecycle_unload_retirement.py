"""Deterministic retirement consumers; no manager, VM or deletion is launched."""
import copy
import hashlib
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[2]/'tests/system'))
import lifecycle_unload_retirement as retirement


def charge(record):
    # The production strict plan/accounting reader is tested separately. These
    # cases supply its result to exercise current/original observation selection.
    state = record['retirement']
    return 0 if state is not None and state['state']=='settled' else record['capture']['reserved_material_bytes']


def fixture():
    physical = lambda number: dict(facts=dict(manifest_digest=[number]*32,
        capture=number,source_incarnation=number))
    def row(number):
        return dict(work=dict(anchor=dict(local=dict(sandbox=1,owner=2),
            native=dict(generation=number),request=dict(request=dict(binding=dict(session='original'))))),
            capture=dict(completed=[number]*32,reserved_material_bytes=100,
                full=dict(pause=dict(binding=dict(capture=number,source_incarnation=number)))),
            restore=None,retirement=None)
    old,current = row(1),row(2)
    raw=b'fixed exact original plan'
    old['retirement']=dict(state='settled',plan=dict(bytes=list(raw),
        work_binding=[3]*32,plan_binding=list(hashlib.sha256(raw).digest())))
    old['restore']=dict(material_use=False,activation={'original':'candidate'},
        intent=dict(request=dict(binding=dict(session='original',key={'sequence':3})),accepted_version=3))
    resume=dict(key={'sequence':3},accepted_version=3)
    return [old,current],physical(1),physical(2),resume


class RetirementSnapshotTests(unittest.TestCase):
    def snapshot(self, records,original,successor,resume):
        with patch.object(retirement,'material_source_charge',side_effect=charge):
            return retirement.retirement_snapshot(records,session='original',
                original=original,successor=successor,resume=resume)

    def test_settled_original_preserves_claim_and_exactly_one_held_successor(self):
        records,original,successor,resume=fixture()
        before=copy.deepcopy(records)
        observed=self.snapshot(records,original,successor,resume)
        self.assertEqual(observed['old'],records[0])
        self.assertEqual(observed['successor'],records[1])
        self.assertEqual(observed['held_material_bytes'],100)
        self.assertEqual(observed['source_history_records'],2)
        self.assertEqual(records,before)

    def test_missing_held_and_retiring_original_are_pending_never_success(self):
        for state in (None,'retiring'):
            records,original,successor,resume=fixture()
            records[0]['restore']['material_use']=state is None
            if state is None:
                records[0]['retirement']=None
            else:
                records[0]['retirement']['state']=state
            with self.subTest(state=state):
                self.assertIsNone(self.snapshot(records,original,successor,resume))

    def test_wrong_identity_claim_or_current_source_never_proves_live_retirement(self):
        for fault in ('old_missing','duplicate','manifest','claim','activation','pin',
                      'successor_claimed','successor_retired','another_held'):
            records,original,successor,resume=fixture()
            old,current=records
            if fault=='old_missing':records.remove(old)
            elif fault=='duplicate':records.append(copy.deepcopy(old))
            elif fault=='manifest':old['capture']['completed']=[9]*32
            elif fault=='claim':old['restore']['intent']['accepted_version']=4
            elif fault=='activation':old['restore']['activation']=None
            elif fault=='pin':old['restore']['material_use']=True
            elif fault=='successor_claimed':current['restore']={'material_use':True}
            elif fault=='successor_retired':current['retirement']={'state':'retiring'}
            elif fault=='another_held':
                third=copy.deepcopy(current);third['capture']['completed']=[5]*32
                records.append(third)
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                self.snapshot(records,original,successor,resume)


class RetirementGateBindingTests(unittest.TestCase):
    def observation(self):
        records,_,_,_=fixture()
        records[0]['retirement']['state']='retiring'
        with patch.object(retirement,'material_source_charge',return_value=100):
            material=retirement.gate_material(records[0],'capture-'+'a'*64)
        source=dict(kind='vm',id='original-resource')
        value=dict(schema_version=1,run_identity='b'*64,
            phase='material-deleted-before-settlement',material=material,source_resource=source)
        return records[0],material,source,value

    def test_raw_sql_plan_length_digest_and_identity_are_used(self):
        record,material,source,value=self.observation()
        self.assertEqual(material['plan_bytes'],len(record['retirement']['plan']['bytes']))
        self.assertEqual(material['plan_binding'],bytes(record['retirement']['plan']['plan_binding']).hex())
        self.assertIs(retirement.validate_retirement_gate(value,run_identity='b'*64,
            expected=material,source_resource=source),value)

    def test_partial_old_or_forged_gate_is_rejected(self):
        for fault in ('schema','run','phase','extra','source','material_extra',
                      'sandbox','generation','capture_key','work_binding','plan_binding','plan_bytes','bool'):
            _,material,source,value=self.observation()
            if fault=='schema':value['schema_version']=True
            elif fault=='run':value['run_identity']='c'*64
            elif fault=='phase':value['phase']='deleted'
            elif fault=='extra':value['settled']=True
            elif fault=='source':value['source_resource']={'id':'new'}
            elif fault=='material_extra':value['material']=dict(material,settled=True)
            elif fault=='bool':value['material']=dict(material,sandbox=True)
            else:
                value['material']=copy.deepcopy(material)
                value['material'][fault]=0 if fault in ('sandbox','generation','plan_bytes') else 'c'*64
            with self.subTest(fault=fault),self.assertRaises(ValueError):
                retirement.validate_retirement_gate(value,run_identity='b'*64,
                    expected=material,source_resource=source)

    def test_changed_plan_settled_state_or_early_refund_cannot_be_a_gate(self):
        for fault in ('bytes','state','charge'):
            record,_,_,_=self.observation()
            if fault=='bytes':record['retirement']['plan']['bytes'][0]^=1
            if fault=='state':record['retirement']['state']='settled'
            with self.subTest(fault=fault),patch.object(retirement,'material_source_charge',
                    return_value=0 if fault=='charge' else 100),self.assertRaises(ValueError):
                retirement.gate_material(record,'capture-'+'a'*64)


if __name__=='__main__':
    unittest.main()
