"""Combine only the existing reviewed record and validation/save candidates in a private copy."""
from pathlib import Path
import hashlib,subprocess,shutil
FILES=['controller-service/src/controller/lifecycle_inbox/model.rs','controller-service/src/controller/lifecycle_inbox/renewal_confirmation.rs']
def apply(root,project,out):
 review=out/'record-candidate';review.mkdir(mode=0o700)
 patch=root/'verification/experiments/stage1-protocol/inbox-record/production-candidate.patch'
 for n in FILES:
  p=review/n;p.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(root/n,p)
 r=subprocess.run(['patch','--batch','--fuzz=0','-p1','-i',str(patch)],cwd=review,capture_output=True)
 (out/'record-patch.log').write_bytes(r.stdout+r.stderr);assert r.returncode==0
 merge={}
 for n in FILES:
  r=subprocess.run(['git','merge-file','--stdout',str(project/n),str(root/n),str(review/n)],capture_output=True)
  (out/(Path(n).name+'.raw-merge')).write_bytes(r.stdout)
  assert r.returncode in (0,1)
  text=r.stdout.decode();resolved=False
  if r.returncode:
   assert n==FILES[0] and text.count('<<<<<<< ')==1 and text.count('=======\n')==1 and text.count('>>>>>>> ')==1
   prefix,rest=text.split('<<<<<<< ',1);_,rest=rest.split('\n',1)
   ours,rest=rest.split('=======\n',1);theirs,rest=rest.split('>>>>>>> ',1);_,suffix=rest.split('\n',1)
   # Both candidates inserted different Session helpers before the same unchanged method.
   assert ours.startswith('    fn count_entry_key_from(') and ours.endswith('        false\n')
   assert ours.count('fn contains_entry_key(')==1 and ours.count('fn ')==2
   assert theirs.startswith('    fn find_entry(') and theirs.endswith('        Ok(None)\n') and theirs.count('fn ')==1
   assert suffix.startswith('    }\n\n    pub(super) fn current_execution_origin(')
   text=prefix+ours+'    }\n\n'+theirs+suffix;resolved=True
  assert not any(marker in text for marker in ['<<<<<<< ','=======','>>>>>>> '])
  (project/n).write_text(text);merge[n]=dict(exit_code=r.returncode,additive_helper_insertion_resolved=resolved)
 subprocess.run(['rustfmt','--edition','2024',*[str(project/n)for n in FILES]],check=True)
 return dict(record_patch_sha256=hashlib.sha256(patch.read_bytes()).hexdigest(),merge=merge,new_business_algorithm=False)
