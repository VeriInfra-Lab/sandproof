"""Independent transparent proposal prefix; full setter admission stays strict."""
import argparse
from hashlib import sha256
import importlib.util,json
from pathlib import Path
HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('source_set_prefix_inputs',HERE/'inputs.py')
inputs=importlib.util.module_from_spec(spec);spec.loader.exec_module(inputs)
parent,retire=inputs.parent,inputs.retire
ROOT='sandproof_controller_service::controller::lifecycle_inbox::source_work::full::end::propose_end'
NAMESPACE='SandproofSourceSetEndPrefix'
OPTIONS=dict(inputs.OPTIONS,roots=[ROOT],retained_roots=[parent.ROOT])
STAGED,END,VARIANTS=inputs.STAGED,inputs.END,inputs.VARIANTS
reconstruct,mutate=inputs.reconstruct,inputs.mutate
LIBRARIES=set(parent.LIBRARIES)
BUSINESS=tuple(n for n in parent.BUSINESS if n not in (
 'controller.lifecycle_inbox.source_work.full.end.material_writer_closed',
 'controller.lifecycle_inbox.source_work.full.drain.writer_closed'))+(
 'controller.lifecycle_inbox.source_work.full.end.propose_end',)
def digest(path):return sha256(path.read_bytes()).hexdigest()
def admit(ir,generated,target):
 tree=retire.checked_translation(ir,OPTIONS,target,ROOT,parent.ROOT)
 mapping=json.loads((generated/'translation.json').read_bytes())['functions'];report=dict(business={},libraries=[])
 for name in (*BUSINESS,*parent.TRANSPARENT_STANDARD):
  rows=[r for r in mapping if r['lean_name']==NAMESPACE+'.'+name]
  if len(rows)!=1 or rows[0]['is_opaque']:raise ValueError('whole transparent prefix/parent/standard function required: '+name)
  bodies=[f for f in tree['fun_decls']if f and f['def_id']==rows[0]['def_id']]
  if len(bodies)!=1 or'Structured'not in(bodies[0]['body']or{}):raise ValueError('actual complete structured body required: '+name)
  report['business'][name]=rows[0]['def_id']
 for row in mapping:
  if row['is_opaque']:
   if row['source']['file'].startswith(('controller-service/','controller-core/','sandbox-protocol/'))and row['lean_name']not in LIBRARIES:raise ValueError('no extra business opacity: '+row['lean_name'])
   report['libraries'].append(row['lean_name'])
 report['libraries'].sort()
 if len(report['libraries'])!=29 or set(report['libraries'])!=LIBRARIES:raise ValueError('only exact29 already inherited interfaces; no Box ne or new proposal encoder: '+str(report['libraries']))
 return report

def capture(output,tools,lock,variant):
 retire.ROOT,retire.NAMESPACE,retire.OPTIONS,retire.STAGED=ROOT,NAMESPACE,OPTIONS,STAGED
 retire.reconstruct,retire.admit=reconstruct,admit
 try:retire.capture(output,tools,lock,variant)
 finally:
  path=output/'result.json'
  if path.is_file():
   r=json.loads(path.read_bytes());r.update(base_driver=r['driver'],driver=digest(Path(__file__)),staging_driver=digest(HERE/'inputs.py'),parent_driver=digest(Path(parent.__file__)),capture_root=ROOT,transparent_original_prefix=True,whole_set_end_proved=False,full_setter_admission_unchanged=True,mutation=None if variant=='original'else'delete only early completed/NoRequest rejection; later setter still separately open')
   path.write_text(json.dumps(r,indent=2)+'\n')
if __name__=='__main__':
 cli=argparse.ArgumentParser(description=__doc__);cli.add_argument('--variant',choices=VARIANTS,required=True)
 for name in('output','tools','lock'):cli.add_argument('--'+name,type=Path,required=True)
 a=cli.parse_args();capture(a.output.resolve(),a.tools.resolve(strict=True),a.lock.resolve(strict=True),a.variant)
