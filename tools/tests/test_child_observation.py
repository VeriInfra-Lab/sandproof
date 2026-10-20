"""Fail-closed admission checks; no Rust/Lean compilation in these tests."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from tools.check_child_observation import (ASSETS, ENVIRONMENT, VARIANTS,
    check_ast, check_sources, check_formal, expected_audits, sha)
from tools.check_sqlite_statements import check_record
from tools.check_sqlite_extraction import proof_sources_digest

ROOT=Path(__file__).resolve().parents[2]

class ChildObservationAdmission(unittest.TestCase):
    def test_registered_inputs_and_audits(self):
        check_sources(ROOT)
        self.assertTrue(expected_audits(ROOT,'correct'))
        self.assertEqual(len(expected_audits(ROOT,'kill-success-release')),4)

    def test_ast_does_not_ignore_body_drop_options_or_source(self):
        env=json.loads((ROOT/ENVIRONMENT).read_text())
        for variant,name in VARIANTS.items():
            data=json.loads((ROOT/'verification/aeneas/llbc'/name).read_text())
            check_ast(data,env['variants'][variant])
            for flag in ['skip_borrowck','no_typecheck','no_normalize','no_reorder_decls']:
                wrong=copy.deepcopy(data);wrong['translated']['options'][flag]=True
                with self.subTest(variant=variant,flag=flag),self.assertRaises(ValueError):
                    check_ast(wrong,env['variants'][variant])
            for mutate in [lambda d:d.update(has_errors=True),
                lambda d:d['translated']['fun_decls'].clear(),
                lambda d:d['translated']['type_decls'].clear(),
                lambda d:d['translated']['files'].clear()]:
                wrong=copy.deepcopy(data);mutate(wrong)
                with self.assertRaises(ValueError):check_ast(wrong,env['variants'][variant])

    def test_failed_formal_batch_is_never_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            output=Path(tmp)
            (output/'result.json').write_text('{"success":false,"views":{}}')
            with self.assertRaises(ValueError):check_formal(ROOT,output)

    def test_whole_physical_record_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            view=Path(tmp);name=Path('SandboxModel/Child.lean');(view/name).parent.mkdir()
            (view/name).write_text('theorem child : True := True.intro\n')
            for n in ['lakefile.toml','lake-manifest.json','lean-toolchain']:(view/n).write_text('{}\n')
            versions={'server_version':'0.30.0','client_version':'0.13.2','mcp_version':'2.0.0'}
            record={'request':{'tool':'lean_diagnostic_messages','arguments':{'file_path':str(name)},'expected_sha256':sha(view/name)},'stale':False,
                'revision':{'file_sha256':sha(view/name),'sources_sha256':proof_sources_digest(view),'toolchain':'{}','lakefile_sha256':sha(view/'lakefile.toml'),'manifest_sha256':sha(view/'lake-manifest.json'),**versions},
                'result':{'result':{'success':True,'partial':False,'timed_out':False,'items':[{'message':'Checked Example.child: 0 standard logical dependencies'}]}}}
            self.assertEqual(check_record(view,record,{'Example.child'},name,versions),1)
            for key in ['partial','timed_out']:
                wrong=copy.deepcopy(record);wrong['result']['result'][key]=True
                with self.assertRaises(ValueError):check_record(view,wrong,{'Example.child'},name,versions)
            for wrong in [dict(record,stale=True),dict(record,request={'tool':'lean_diagnostic_messages','arguments':{'file_path':str(name),'severity':'error'}})]:
                with self.assertRaises(ValueError):check_record(view,wrong,{'Example.child'},name,versions)
            for items in [[],[{'severity':'warning','message':'warning'}],[{'message':'Checked Example.other: 0 standard logical dependencies'}]]:
                wrong=copy.deepcopy(record);wrong['result']['result']['items']=items
                with self.assertRaises(ValueError):check_record(view,wrong,{'Example.child'},name,versions)

if __name__=='__main__':unittest.main()
