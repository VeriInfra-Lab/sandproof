"""Check complete current PathBuf nesting and exact actual environment-map type."""
def admit(raw,carrier):
 table,_=carrier.interned(raw);types={t['def_id']:t for t in raw['translated']['type_decls'] if t}
 def named(path):
  name=[{'Ident':[n,0]} for n in path.split('::')]
  found=[t for t in types.values() if t['item_meta']['name']==name];assert len(found)==1;return found[0]
 def field_type(declaration,name):
  fields=declaration['kind']['Struct'];assert len(fields)==1 and fields[0]['name']==name
  return carrier.resolve(fields[0]['ty'],table)
 paths=['std::path::PathBuf','std::ffi::os_str::OsString','std::sys::os_str::bytes::Buf'];declarations=[named(p) for p in paths]
 for left,right in zip(declarations,declarations[1:]):
  ty=field_type(left,'inner');assert ty['Adt']['id']==right['def_id'] and not ty['Adt']['generics']['types'];assert left['item_meta']['opacity']=='Transparent'
 vector=field_type(declarations[-1],'inner')['Adt'];vec=named('alloc::vec::Vec');assert vector['id']==vec['def_id'];params=vector['generics']['types'];assert len(params)==1 and len(vec['generics']['types'])==1
 assert carrier.resolve(params[0],table)=={'Scalar':{'Integer':{'Unsigned':'U8'}}}
 startup=named('sandproof_controller_service::templates::TemplateStartup');field=next(f for f in startup['kind']['Struct'] if f['name']=='env_vars');mapping=carrier.resolve(field['ty'],table)['Adt'];assert mapping['id']==named('alloc::collections::btree::map::BTreeMap')['def_id'];params=mapping['generics']['types'];assert len(params)==3
 assert all(carrier.resolve(p,table)['Adt']['id']==named('alloc::string::String')['def_id'] for p in params[:2]);assert carrier.resolve(params[2],table)['Adt']['id']==named('alloc::alloc::Global')['def_id']
 return dict(complete_raw_path_type_chain=[dict(path=p,def_id=d['def_id'],fields=d['kind']['Struct']) for p,d in zip(paths,declarations)],full_byte_vec_preserved=True,no_utf8_or_normalization_assumption=True,actual_map_field='CreateAttachment.registered_template.startup.env_vars',actual_map_is_full_string_string_global=True,existing_map_representation_provider='verification/aeneas/storage/SandproofStorage/TypesExternal.lean',machine_path_connection_proved=False,parent_semantics_proved=False)
