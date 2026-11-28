"""One shared full digest contract, exact retained providers and independent Save views."""
from pathlib import Path
import importlib.util,json,re,shutil
ROOT=Path(__file__).resolve().parents[4]
HERE=Path('verification/experiments/stage1-protocol/standard-save-digest')
PROOFS=Path('SandboxModel/Lifecycle/Protocol')
GEN=Path('verification/aeneas/stage1-protocol/standard-save-digest/SandproofStandardSaveDigest')
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);return m
parent=module('save_digest_exact_parent',ROOT/HERE.parent/'standard-digest/support.py')
blob=parent.blob
preparation=module('save_digest_actual_source_preparation',ROOT/HERE/'preparation.py')
base,checker,resolver=parent.base,parent.checker,parent.resolver
runtime,reader,registry_module=parent.runtime,parent.reader,parent.registry_module
sha,mode,ir_value_digest=parent.sha,parent.mode,parent.ir_value_digest
COMMON=['StandardSaveDigestModel','StandardSaveDigestRepresentation','StandardSaveDigestValues']
VIEWS={'actual':['StandardSaveDigestCore','StandardSaveDigestWitnesses','InboxSaveDigestDomain','StandardSaveDigestBridge'],
 'save-input-empty':['StandardSaveInputCounter'],'save-tail-zero':['StandardSaveTailCounter'],'delegate-input-empty':['StandardSaveDelegateCounter']}
def retained_roots(variant):
 return blob.COMMON+parent.COMMON+COMMON+VIEWS[variant]
def inventory():
 # This slice checks its exact reused providers below; it does not reaccept
 # the parent package or retag its original execution under a newer tree.
 d=json.loads((ROOT/HERE/'inventory.json').read_text())
 if d['source_adopted'] is not True or d['library_provider_adopted'] is not True or d['full_v3']:raise ValueError('adopted finite source/library conditions required')
 for group,prefix in [('source_inputs',ROOT),('providers',ROOT),('package_inputs',ROOT/HERE),('generated',ROOT)]:
  for n,h in d[group].items():
   p=prefix/n
   if p.is_symlink() or not p.is_file() or sha(p)!=h:raise ValueError('reviewed '+group+' changed: '+n)
 owned={p.stem.removesuffix('.lean') for p in (ROOT/HERE).glob('*.lean.in')}
 if owned!=set(COMMON).union(*map(set,VIEWS.values())):raise ValueError('retained owned template missing or unregistered')
 audits={stem:re.findall(r'^#audit_controller_proof (\S+)$',(ROOT/HERE/(stem+'.lean.in')).read_text(),re.M)for stem in sorted(owned)}
 if audits!=d['owned_audits']:raise ValueError('owned audit inventory differs')
 return d

def materialize(project,variant,generated,generic=None):
 changed={}
 def copy(source,destination):
  if source.is_symlink() or not source.is_file():raise ValueError('provider must be an original regular file')
  target=project/destination;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(source,target);changed[str(destination)]=sha(target)
 # Same canonical Read/connection and pure SHA library/math in every view.
 for stem in blob.COMMON:copy(ROOT/blob.HERE/(stem+'.lean.in'),PROOFS/(stem+'.lean'))
 for stem in ('TypesExternal','FunsExternal','ReadValueData','ReadTypeBindings'):copy(ROOT/blob.HERE/'external'/(stem+'.lean.in'),blob.READ_GEN/(stem+'.lean'))
 read_index=json.loads((ROOT/blob.HERE/'generated-index.json').read_text())['captures']['standard-actual']['published']
 for n in ('Types.lean','Funs.lean'):
  row=read_index[blob.READ_GEN.name+'/'+n];assert sha(ROOT/row['path'])==row['sha256'];copy(ROOT/row['path'],blob.READ_GEN/n)
 for stem in parent.MATH:copy(ROOT/parent.HERE/'reuse'/(stem+'.lean.in'),PROOFS/(stem+'.lean'))
 copy(ROOT/parent.HERE/'reuse'/parent.ARRAY,parent.ARRAY)
 digest_index=json.loads((ROOT/parent.HERE/'generated-index.json').read_text())['captures']['actual']['published']
 for n in ('Types.lean','Funs.lean'):
  row=digest_index[parent.GEN.name+'/'+n];assert sha(ROOT/row['path'])==row['sha256'];copy(ROOT/row['path'],parent.GEN/n)
 for stem in ('TypesExternal','FunsExternal'):copy(ROOT/parent.HERE/'external'/(stem+'.lean.in'),parent.GEN/(stem+'.lean'))
 for stem in parent.COMMON:copy(ROOT/parent.HERE/(stem+'.lean.in'),PROOFS/(stem+'.lean'))
 for n in ('Types.lean','Funs.lean'):copy(generated/GEN.name/n,GEN/n)
 for stem in ('TypesExternal','FunsExternal'):copy(ROOT/HERE/'external'/(stem+'.lean.in'),GEN/(stem+'.lean'))
 if variant=='actual':
  if generic is None:raise ValueError('actual generic Save captures are required')
  # Reuse current existing protocol/validation mathematics. Only explicit
  # previously reviewed namespace and Store24 adaptations are admitted.
  providers=json.loads((ROOT/blob.HERE/'materialization.json').read_text())['providers']
  for n,e in providers.items():
   source=ROOT/e['provider'];assert sha(source)==e['provider_sha256'];value=source.read_bytes()
   if e['transformation']=='only-existing-library-namespace':value=value.replace(b'SandproofStage1OwnerMapping',b'SandproofStage1Request')
   elif e['transformation']=='reviewed-store23-to24-literals':
    value,count=re.subn(rb'\b23\b',b'24',value);assert count==e['literal_occurrences']
   elif e['transformation']!='identity':raise ValueError('unreviewed transformation')
   target=project/n;target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(value);assert sha(target)==e['sha256'];changed[n]=sha(target)
  copy(ROOT/blob.HERE/'external/LoadFunsExternal.lean.in',blob.LOAD_GEN/'FunsExternal.lean')
  for stem in ('InboxSaveModel','InboxSaveRun','InboxSaveProofs'):copy(ROOT/HERE.parent/'inbox-save'/(stem+'.lean.in'),PROOFS/(stem+'.lean'))
  for key,namespace,destination in [('canonical','SandproofStage1Request',blob.LOAD_GEN),('primitive','SandproofStage1Graphic',blob.GRAPHIC_GEN)]:
   for n in ('Types.lean','Funs.lean'):copy(Path(generic[key]['generated_directory'])/namespace/n,destination/n)
 for stem in COMMON+VIEWS[variant]:copy(ROOT/HERE/(stem+'.lean.in'),PROOFS/(stem+'.lean'))
 if variant!='actual':
  forbidden=('StandardDigestCore','StandardDigestBridge','StandardDigestWitnesses','StandardSaveDigestCore','StandardSaveDigestWitnesses','StandardSaveDigestBridge','InboxSaveDigestDomain')
  if any((project/PROOFS/(stem+'.lean')).exists() for stem in forbidden):raise ValueError('wrong environment contains correct correspondence')
 lake=project/'lakefile.toml';text=lake.read_text()
 libs=[('SandproofLoadBlob',blob.READ_GEN.parent),('SandproofStorage',Path('verification/aeneas/storage')),('SandproofStandardDigest',parent.GEN.parent),('SandproofDigestArray32',parent.ARRAY.parent.parent),('SandproofStandardSaveDigest',GEN.parent)]
 if variant=='actual':libs.append(('SandproofStage1Graphic',blob.GRAPHIC_GEN.parent))
 for name,directory in libs:
  if 'name = "'+name+'"' not in text:
   text+='\n[[lean_lib]]\nname = "'+name+'"\nsrcDir = "'+str(directory)+'"\n'
   if name=='SandproofLoadBlob':text+='leanOptions = { weak.linter.style.nameCheck = false }\n'
 lake.write_text(text);changed['lakefile.toml']=sha(lake)
 return changed
