"""Exact outer source-record serde schema and required-option use sites."""
import json,re,sys
from pathlib import Path

EXPECTED = {
 'SourceAnchor': [
  ('request','AcceptedLifecycleIntent',False),('execution_origin','AcceptedLifecycleIntent',False),
  ('local','InternalLocalBinding',False),('native','NativeCreateSnapshot',False),
  ('continued_from','Option<sandbox_controller_core::capture::OriginalContinueIdentity>',True),
  ('node','NodeIdentity',False),('host_bootstrap','HostEpochRegistration',False),
  ('source','SourceProcessIdentity',False),('options_sha256','ContentDigest',False),
  ('credential','(u64,[u8;32])',False),('original_actions','Vec<ActionIdentity>',False),
  ('original_receipts','Vec<ContentDigest>',False)],
 'SourceWork': [('anchor','SourceAnchor',False),('requirement','SourceObservationRequirement',False)],
 'SourceWorkRecord': [
  ('work','SourceWork',False),('observation','Option<RawSourceObservation>',False),
  ('capture','Option<full::CaptureWork>',True),('restore','Option<restore::RestoreClaim>',True),
  ('retirement','Option<retirement::MaterialRetirementState>',True),
  ('material_release','retirement::MaterialReleaseFacts',False)],
}


def split_fields(body):
    fields=[]; start=0; stack=[]; pairs={'>':'<',')':'(',']':'['}
    for i,c in enumerate(body):
        if c in '<([': stack.append(c)
        elif c in pairs:
            if not stack or stack.pop()!=pairs[c]: raise ValueError('field type is not balanced')
        elif c==',' and not stack:
            fields.append(body[start:i].strip()); start=i+1
    if stack or body[start:].strip(): raise ValueError('unreviewed field syntax')
    return fields


def schema(source, generated):
    text=re.sub(r'//[^\n]*','',source.read_text())
    actual={}
    for name,expected in EXPECTED.items():
        matches=re.findall(r'#\[derive\(([^]]+)\)\]\s*#\[serde\(deny_unknown_fields\)\]\s*(?:pub\s+)?struct '+name+r'\s*\{([^}]+)\}',text)
        if len(matches)!=1 or not {'Serialize','Deserialize'}.issubset(set(x.strip() for x in matches[0][0].split(','))):
            raise ValueError('actual standard derive or deny_unknown_fields changed: '+name)
        fields=[]
        for field in split_fields(matches[0][1]):
            required=field.startswith('#[serde(deserialize_with = "required_option")]')
            if required: field=field.removeprefix('#[serde(deserialize_with = "required_option")]').strip()
            m=re.fullmatch(r'(?:pub\s+)?(\w+)\s*:\s*(.+)',field,re.S)
            if not m: raise ValueError('unreviewed field or custom attribute: '+name)
            fields.append((m[1],re.sub(r'\s+','',m[2]),required))
        if fields!=expected: raise ValueError('actual complete fields or required_option attributes changed: '+name)
        decl=re.search(r'^structure controller\.lifecycle_inbox\.source_work\.'+name+r' where\n(.*?)(?=\n\n/--|\Z)',generated.read_text(),re.M|re.S)
        if not decl: raise ValueError('complete actual generated carrier missing: '+name)
        names=re.findall(r'^  «?(\w+)»?\s*:',decl[1],re.M)
        if names!=[f[0] for f in fields]: raise ValueError('generated outer field inventory/order differs: '+name)
        actual[name]=dict(deny_unknown=True,fields=[dict(name=f[0],rust_type=f[1],required_option=f[2]) for f in fields])
    if text.count('Option::<T>::deserialize(deserializer)')!=1: raise ValueError('actual required_option forwarding differs')
    return actual

if __name__=='__main__':
    out=schema(Path(sys.argv[1]),Path(sys.argv[2]));print('outer exact fields', {n:len(v['fields']) for n,v in out.items()},'required_option sites',sum(f['required_option'] for v in out.values() for f in v['fields']))
