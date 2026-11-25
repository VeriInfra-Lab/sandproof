"""Exact A6 use-site source admission; not a proof of the entire Serde/JSON engine."""
import hashlib
import json
from pathlib import Path
import subprocess

EXPECTED = {
    'serde': ('1.0.228', ['alloc', 'default', 'derive', 'serde_derive', 'std']),
    'serde_core': ('1.0.228', ['alloc', 'default', 'result', 'std']),
    'serde_derive': ('1.0.228', ['default']),
    'serde_json': ('1.0.142', ['default', 'raw_value', 'std']),
}
EXTRA_DE = 'serde_derive-1.0.228/src/de.rs'
EXTRA_SHA = '08fc2db0db3e7263ccfb8999ecc6249e5a8c3d9d3a3b85ea3b8995e865f08c83'


def unique(text, pattern, label):
    if text.count(pattern) != 1:
        raise ValueError('the exact original library source call differs: ' + label)


def compact(text):
    return ''.join(text.split())


def capture(source, root):
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
        '--manifest-path', str(source / 'controller-service/Cargo.toml')], cwd=source))
    features = {item['id']: sorted(item['features']) for item in metadata['resolve']['nodes']}
    packages = {item['name']: item for item in metadata['packages'] if item['name'] in EXPECTED}
    if len(packages) != len(EXPECTED) or any((item['version'], features[item['id']]) != EXPECTED[name]
            for name,item in packages.items()):
        raise ValueError('actual fixed Serde version/features differ')
    pinned = json.loads((root / 'verification/json-request-binding.json').read_bytes())['library_sources']
    selected = {name: pinned[name] for name in ('serde_core-1.0.228/src/ser/impls.rs',
        'serde_core-1.0.228/src/de/impls.rs','serde_json-1.0.142/src/ser.rs',
        'serde_json-1.0.142/src/de.rs','serde-1.0.228/src/private/de.rs')}
    selected[EXTRA_DE] = EXTRA_SHA
    texts={}
    for relative,digest in selected.items():
        crate, path = relative.split('/', 1)
        name, version = crate.rsplit('-', 1)
        actual = Path(packages[name]['manifest_path']).parent / path
        if actual.is_symlink() or hashlib.sha256(actual.read_bytes()).hexdigest()!=digest:
            raise ValueError('the fixed original library source differs: ' + relative)
        texts[relative] = actual.read_text()
    option_ser = texts['serde_core-1.0.228/src/ser/impls.rs'].split('impl<T> Serialize for Option<T>',1)[1].split('////////////////////////////////',1)[0]
    unique(compact(option_ser), compact('match *self { Some(ref value) => serializer.serialize_some(value), None => serializer.serialize_none(), }'), 'Option Serialize dispatch')
    json_ser = texts['serde_json-1.0.142/src/ser.rs']
    primary = json_ser.split("impl<'a, W, F> ser::Serializer for &'a mut Serializer<W, F>",1)[1].split("impl<'a, W, F> ser::Serializer for MapKeySerializer",1)[0]
    unique(compact(primary), compact('fn serialize_some<T>(self, value: &T) -> Result<()> where T: ?Sized + Serialize, { value.serialize(self) }'), 'original Compact Serializer Some forwarding')
    unique(compact(json_ser), compact('pub fn new(writer: W) -> Self { Serializer::with_formatter(writer, CompactFormatter) }'), 'same CompactFormatter')
    unique(compact(json_ser), compact('pub fn to_writer<W, T>(writer: W, value: &T) -> Result<()> where W: io::Write, T: ?Sized + Serialize, { let mut ser = Serializer::new(writer); value.serialize(&mut ser) }'), 'same original writer operation')
    unique(compact(json_ser), compact('pub fn to_vec<T>(value: &T) -> Result<Vec<u8>> where T: ?Sized + Serialize, { let mut writer = Vec::with_capacity(128); tri!(to_writer(&mut writer, value)); Ok(writer) }'), 'same fresh Vec and original first failure')
    json_de = texts['serde_json-1.0.142/src/de.rs']
    unique(compact(json_de), compact("""match tri!(self.parse_whitespace()) { Some(b'n') => { self.eat_char(); tri!(self.parse_ident(b"ull")); visitor.visit_none() } _ => visitor.visit_some(self), }"""), 'original option token dispatch')
    unique(compact(json_de), compact('let mut de = Deserializer::new(read); let value = tri!(de::Deserialize::deserialize(&mut de));'), 'original full document start')
    unique(compact(json_de), compact('tri!(de.end()); Ok(value)'), 'original end before value return')
    unique(compact(json_de), compact('from_trait(read::SliceRead::new(v))'), 'original from_slice input')
    core_de = texts['serde_core-1.0.228/src/de/impls.rs']
    option_de = core_de.split('struct OptionVisitor<T>',1)[1].split('////////////////////////////////',1)[0]
    unique(compact(option_de), compact('T::deserialize(deserializer).map(Some)'), 'original Some visitor')
    unique(compact(option_de), compact('deserializer.deserialize_option(OptionVisitor { marker: PhantomData, })'), 'original Option Deserialize dictionary')
    derive = texts[EXTRA_DE].split('fn expr_is_missing(field:',1)[1].split('fn expr_is_missing_seq(',1)[0]
    unique(compact(derive), compact('return _serde::#private::Err(<__A::Error as _serde::de::Error>::missing_field(#name))'), 'present custom deserialize_with required field')
    return {'features': {name: {'version': item['version'], 'features': features[item['id']]} for name,item in packages.items()},
            'library_sources': selected, 'complete': True, 'library_refinement': False}
