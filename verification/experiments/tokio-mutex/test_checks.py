"""Prevent narrowing or silently changing the component admission."""
from pathlib import Path
import hashlib
import json
import shutil
import tempfile
import unittest
from check import check_inputs, ROOT, OWN, MODEL

class Admission(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)
        env=json.loads((ROOT/OWN/'environment.json').read_text())
        frozen=json.loads((ROOT/OWN/'frozen.json').read_text())
        for n in {*env['base_inputs'],*env['production_sources'],*frozen['files'],str(OWN/'frozen.json')}:
            p=self.root/n;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/n,p)
    def rehash(self,name):
        p=self.root/OWN/'frozen.json';d=json.loads(p.read_text())
        d['files'][str(name)]=hashlib.sha256((self.root/name).read_bytes()).hexdigest();p.write_text(json.dumps(d))
    def change_environment(self,change):
        n=OWN/'environment.json';p=self.root/n;d=json.loads(p.read_text());change(d);p.write_text(json.dumps(d));self.rehash(n)
    def test_valid_frozen_component(self):
        self.assertEqual(len(check_inputs(self.root)['files']),len(json.loads((ROOT/OWN/'frozen.json').read_text())['files']))
    def test_cannot_drop_actual_input_binding(self):
        self.change_environment(lambda d:d['use_points'].pop())
        with self.assertRaisesRegex(ValueError,'Seven original'):check_inputs(self.root)
    def test_cannot_turn_it_into_async_refinement(self):
        self.change_environment(lambda d:d.update(async_rust_refinement=True))
        with self.assertRaisesRegex(ValueError,'Unsupported scope'):check_inputs(self.root)
    def test_cannot_remove_audit_even_with_refreshed_digest(self):
        n=MODEL/'Audit.lean';p=self.root/n;s=p.read_text();p.write_text(s[:s.rfind('#audit_controller_proof')]);self.rehash(n)
        with self.assertRaisesRegex(ValueError,'Exact 21'):check_inputs(self.root)
    def test_changed_real_source_fails_closed(self):
        p=self.root/'sandbox-guest-service/src/session.rs';p.write_text(p.read_text()+'\n// changed real source\n')
        with self.assertRaisesRegex(ValueError,'Frozen input differs'):check_inputs(self.root)
    def test_extra_unregistered_module_fails_closed(self):
        (self.root/MODEL/'Extra.lean').write_text('import Std\n')
        with self.assertRaisesRegex(ValueError,'unregistered'):check_inputs(self.root)

if __name__=='__main__':unittest.main()
