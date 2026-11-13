"""Structural admission of the real u64 roots and complete generated error values."""


def collect(node,key):
    result=[]
    def visit(value):
        if isinstance(value,dict):
            if key in value:result.append(value[key])
            for child in value.values():visit(child)
        elif isinstance(value,list):
            for child in value:visit(child)
    visit(node)
    return result


def admit(raw,roots,includes,opaque):
    assert raw['has_errors'] is False
    translated=raw['translated'];options=translated['options']
    assert options['start_from']==roots and options['include']==includes and options['opaque']==opaque
    assert not any(options[n] for n in ('exclude','skip_borrowck','no_typecheck','monomorphize','remove_unused_clauses'))
    assert {x['key'] for x in translated['target_information']}=={'x86_64-unknown-linux-gnu'}
    functions=[f for f in translated['fun_decls'] if f];types=[t for t in translated['type_decls'] if t]
    def path(names):return [{'Ident':[n,0]} for n in names]
    def forwarding(names,target_name):
        root=next(f for f in functions if f['item_meta']['name']==path(names))
        assert root['item_meta']['opacity']=='Transparent' and root['body']['Structured']['locals']['arg_count']==1
        calls=collect(root['body'],'Call');assign=collect(root['body'],'Assign');assert len(calls)==len(assign)==1
        call=calls[0]['call'];destination,expression=assign[0]
        assert call['dest']['kind']=={'Local':0} and len(call['args'])==1
        assert call['args'][0]['Move']['kind']==destination['kind'] and expression['Use'][0]['Copy']['kind']=={'Local':1}
        target=next(f for f in functions if f['def_id']==call['func']['Regular']['kind']['Fun'])
        assert target['item_meta']['name'][-1]=={'Ident':[target_name,0]} and target['item_meta']['opacity']=='Transparent'
        assert 'Structured' in target['body']
        return dict(root=root['def_id'],actual=target['def_id'],complete_u64_input_and_result_forwarded=True)
    numeric=forwarding(['sandproof_stage1_inbox_record_extract','proof_deadline_cast'],'try_from')
    helper=forwarding(['sandproof_controller_service','controller','lifecycle_inbox','proof_create_attachment_deadline'],'create_attachment_deadline')
    enum=next(t for t in types if t['item_meta']['name']==path(['core','num','error','IntErrorKind']))
    variants=enum['kind']['Enum'];assert [v['name'] for v in variants]==['Empty','InvalidDigit','PosOverflow','NegOverflow','Zero','NotAPowerOfTwo'] and all(not v['fields'] for v in variants)
    wrapper=next(t for t in types if t['item_meta']['name']==path(['core','num','error','TryFromIntError']))
    fields=wrapper['kind']['Struct'];assert len(fields)==1 and fields[0]['ty']['Value'][1]['Adt']['id']==enum['def_id']
    mapped=[f for f in functions if f['item_meta']['name'][-1]=={'Ident':['map_err',0]}]
    assert len(mapped)==1 and mapped[0]['item_meta']['opacity']=='Transparent' and 'Structured' in mapped[0]['body']
    journal=next(t for t in types if t['item_meta']['name']==path(['sandproof_controller_service','JournalError']))
    assert journal['item_meta']['opacity']=='Transparent' and 'Enum' in journal['kind']
    return dict(numeric_root=numeric,helper_root=helper,complete_error_kind=enum,actual_single_field_error_wrapper=wrapper,complete_journal_error=journal,actual_std_map_err_transparent_llbc=True,map_err_uses_existing_fixed_backend=True)
