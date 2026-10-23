"""Fail-closed admission checks; no Rust/Lean compilation in these tests."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

from tools.check_child_observation import (ASSETS, ENVIRONMENT, VARIANTS,
    check_ast, check_sources, check_formal, expected_audits, fresh_native_artifact, sha)
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


class ActualChildHarnessAdmission(unittest.TestCase):
    def test_actual_same_crate_harness_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);source=root/'source';target=root/'target';target.mkdir()
            package=source/'sandbox-runtime';binary=target/'runtime-tests';log=root/'cargo.log'
            row={'reason':'compiler-artifact','fresh':False,'package_id':package.as_uri().replace('file:','path+file:')+'#0.1.0',
                 'manifest_path':str(package/'Cargo.toml'),'target':{'name':'sandbox_runtime','kind':['lib'],'src_path':str(package/'src/lib.rs')},
                 'profile':{'test':True},'executable':str(binary),'filenames':[str(binary)]}
            def admit(rows):
                log.write_text('\n'.join(json.dumps(x) for x in rows))
                return fresh_native_artifact(log,target,source)
            self.assertEqual(admit([row]),row)
            for mutate in [lambda x:x.update(fresh=True),lambda x:x.update(package_id='registry+https://example.invalid#sandbox-runtime@0.1.0'),
                           lambda x:x.update(manifest_path=str(root/'old/Cargo.toml')),
                           lambda x:x['target'].update(src_path=str(root/'old/src/lib.rs')),
                           lambda x:x.update(executable=None),lambda x:x.update(filenames=[str(root/'other-binary')])]:
                wrong=copy.deepcopy(row);mutate(wrong)
                with self.assertRaises(ValueError):admit([wrong])
            for rows in [[],[row,row]]:
                with self.assertRaises(ValueError):admit(rows)
