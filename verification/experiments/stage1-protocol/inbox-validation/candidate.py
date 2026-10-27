"""Compose existing private Rust candidates for the complete decoded Inbox validator.

Production adoption and original/candidate equivalence remain separate obligations.
"""
from pathlib import Path
import importlib.util

MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
PARENT = Path('verification/experiments/stage1-protocol')


def module(project, package):
    spec = importlib.util.spec_from_file_location('outer_'+package.replace('-', '_'),
                                                project/PARENT/package/'prepare.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('nonunique outer composition: '+old[:100])
    return text.replace(old, new, 1)


HELPERS = '''
impl Inbox {
    fn lanes_invalid<T: TerminalEncoding>(&self, context: &AuthenticatedCenterContext) -> bool {
        let mut index = 0;
        while index < self.lanes.len() {
            let lane = &self.lanes[index];
            if usize::from(lane.slot) != index + 1 || !lane.valid_using::<T>(context) {
                return true;
            }
            index += 1;
        }
        false
    }

    fn prior_owner_conflict(&self, index: usize, owner: &Owner) -> bool {
        let previous = &self.owners[..index];
        let mut position = 0;
        while position < previous.len() {
            let old = &previous[position];
            if old.external == owner.external || old.local == owner.local {
                return true;
            }
            position += 1;
        }
        false
    }

    fn has_local_owner(&self, session: &Session, local_owner: sandbox_controller_core::TenantId) -> bool {
        let mut position = 0;
        while position < self.owners.len() {
            let owner = &self.owners[position];
            if owner.external == session.owner && owner.local == local_owner {
                return true;
            }
            position += 1;
        }
        false
    }
}

fn has_destroy_at_maximum(session: &Session, maximum: u64) -> bool {
    let mut position = 0;
    while position < session.entries.len() {
        let entry = &session.entries[position];
        if matches!(entry.intent.request.operation, LifecycleOperation::Destroy {})
            && entry.intent.accepted_version == maximum
        {
            return true;
        }
        position += 1;
    }
    false
}
'''


def candidate(project):
    # Reuse the existing implementations of every lower-level candidate. They
    # still call actual Rust methods; only the original codec calls are generic.
    old_spec = importlib.util.spec_from_file_location('outer_historical_split', Path(__file__).with_name('split_owners.py'))
    old_split = importlib.util.module_from_spec(old_spec)
    old_spec.loader.exec_module(old_split)
    split_body, _, split_record = old_split.prepare((project/MODEL).read_text())
    (project/MODEL).write_text(split_body)
    shape = module(project, 'session-shape')
    lane = module(project, 'retention-lane')
    config = module(project, 'inbox-config-validation')
    retired = module(project, 'inbox-retired-identities')
    entry = module(project, 'inbox-entry-validation')
    renewal = module(project, 'inbox-renewal-session')
    body, saved, shape_changes = shape.candidate(project)
    # One shared TerminalEncoding definition for lane and session checks.
    body = replace_once(body, lane.INTERFACE, '')
    (project/MODEL).write_text(body)
    (project/shape.SAVED).write_text(saved)
    _, body, lane_changes = lane.candidate(project)
    body = replace_once(body, config.ORIGINAL, config.CANDIDATE)
    (project/MODEL).write_text(body)
    _, body = retired.candidate(project)
    (project/MODEL).write_text(body)
    sources, entry_changes = entry.candidate(project)
    for name, (_, text) in sources.items():
        (project/name).write_text(text)
    _, renewal_body, renewal_changes = renewal.candidate(project)
    renewal_body = replace_once(renewal_body, 'struct StandardRenewalEncoding;',
                                'pub(super) struct StandardRenewalEncoding;')
    unused_renewal = '''pub(super) fn validate_session(session: &Session) -> Result<(), JournalError> {
    validate_session_using::<StandardRenewalEncoding>(session)
}

'''
    renewal_body = replace_once(renewal_body, unused_renewal, '')
    (project/renewal.SOURCE).write_text(renewal_body)

    body = (project/MODEL).read_text()
    begin = body.index('    pub(super) fn validate(&self) -> Result<(), JournalError> {')
    end = body.index('    fn prior_retired_identity_conflict(', begin)
    outer = body[begin:end]
    signature = '    pub(super) fn validate(&self) -> Result<(), JournalError> {'
    generic = '''    fn validate_using<
        R: super::native_retirement::RetirementEncoding,
        T: TerminalEncoding,
        N: super::renewal_confirmation::RenewalEncoding,
        U: super::unload_resolution::UnloadEncoding,
    >(&self) -> Result<(), JournalError> {'''
    wrapper = signature+'''
        self.validate_using::<
            super::native_retirement::StandardRetirementEncoding,
            StandardTerminalEncoding,
            super::renewal_confirmation::StandardRenewalEncoding,
            super::unload_resolution::StandardUnloadEncoding,
        >()
    }

'''
    edits = [
        (signature, wrapper+generic),
        ('self.validate_one_session(i, session, &context)?;', 'self.validate_one_session::<R, T, N, U>(i, session, &context)?;'),
        ('    fn validate_one_session(', '''    fn validate_one_session<
        R: super::native_retirement::RetirementEncoding,
        T: TerminalEncoding,
        N: super::renewal_confirmation::RenewalEncoding,
        U: super::unload_resolution::UnloadEncoding,
    >('''),
        ('''self
                .lanes
                .iter()
                .enumerate()
                .any(|(index, lane)| usize::from(lane.slot) != index + 1 || !lane.valid(&context))''',
         'self.lanes_invalid::<T>(&context)'),
        ('''self.owners[..i]
                    .iter()
                    .any(|old| old.external == owner.external || old.local == owner.local)''',
         'self.prior_owner_conflict(i, owner)'),
        ('self.validate_session_shape(i, session)?;',
         'self.validate_session_shape_using::<R, T>(i, session)?;'),
        ('super::renewal_confirmation::validate_session(session)?;',
         'super::renewal_confirmation::validate_session_using::<N>(session)?;'),
        ('self.entry_kind(session, entry, context)?;',
         'self.entry_kind_using::<U>(session, entry, context)?;'),
        ('''session.entries.iter().any(|e| {
                    matches!(e.intent.request.operation, LifecycleOperation::Destroy {})
                        && e.intent.accepted_version == maximum
                })''', 'has_destroy_at_maximum(session, maximum)'),
        ('''self
                    .owners
                    .iter()
                    .any(|o| o.external == session.owner && o.local == local.owner)''',
         'self.has_local_owner(session, local.owner)'),
    ]
    for old, new in edits:
        outer = replace_once(outer, old, new)
    body = body[:begin]+outer+body[end:]+HELPERS
    unused_wrappers = {'renewal_confirmation.validate_session': unused_renewal}
    for signature, following, call in [
        ('    fn valid(&self, context: &AuthenticatedCenterContext) -> bool {',
         '    fn valid_using<', 'self.valid_using::<StandardTerminalEncoding>(context)'),
        ('    fn validate_session_shape(&self,', '    fn validate_session_shape_using<',
         'self.validate_session_shape_using::<'),
        ('    fn entry_kind(', '    fn entry_kind_using<', 'self.entry_kind_using::<'),
    ]:
        assert body.count(signature) == 1
        begin = body.index(signature)
        end = body.index(following, begin)
        removed = body[begin:end]
        assert removed.count(call) == 1 and 'if ' not in removed
        unused_wrappers[signature.strip()] = removed
        body = body[:begin]+body[end:]
    start = body.index('        let mut maximum = 0;', body.index('    fn validate_one_session<'))
    end = body.index('        session.validate_execution_lineage()?;', start)
    entry_loop = body[start:end]
    assert entry_loop.count('for entry in &session.entries') == 1
    helper = '''
impl Inbox {
    fn validate_entry_order_using<U: super::unload_resolution::UnloadEncoding>(
        &self,
        session: &Session,
        context: &AuthenticatedCenterContext,
    ) -> Result<(u64, [u16; 5]), JournalError> {
''' + entry_loop + '''        Ok((maximum, kinds))
    }
}
'''
    body = body[:start] + '        let (maximum, kinds) = self.validate_entry_order_using::<U>(session, context)?;\n' + body[end:] + helper
    # Exact scalar Ord::max expression from the pinned core library; the fixed
    # backend emits an incompatible dictionary argument for the default method.
    maximum_original = 'maximum = maximum.max(intent.accepted_version);'
    maximum_candidate = 'maximum = if maximum > intent.accepted_version { maximum } else { intent.accepted_version };'
    body = replace_once(body, maximum_original, maximum_candidate)
    # Keep the existing configuration predicate; parameterize only its original
    # std Bytes.all expression. The standard implementation is extracted again
    # in a separate primitive view, avoiding the fixed Iterator trait collision.
    config_using = config.CANDIDATE.replace('pub fn valid(&self)',
        'fn valid_using<B: crate::capture::GraphicBytes>(&self)').replace(
        'Some(registration) => registration.valid(),',
        'Some(registration) => registration.valid_using::<B>(),')
    config_wrapper = '''    #[must_use]
    pub fn valid(&self) -> bool {
        self.valid_using::<crate::capture::StandardGraphicBytes>()
    }
'''
    body = replace_once(body, config.CANDIDATE, config_wrapper+config_using)
    body = replace_once(body, '            super::unload_resolution::StandardUnloadEncoding,\n        >()',
        '            super::unload_resolution::StandardUnloadEncoding,\n            crate::capture::StandardGraphicBytes,\n        >()')
    signature = '''    fn validate_using<
        R: super::native_retirement::RetirementEncoding,
        T: TerminalEncoding,
        N: super::renewal_confirmation::RenewalEncoding,
        U: super::unload_resolution::UnloadEncoding,
    >'''
    body = replace_once(body, signature, signature.replace('    >',
        '        B: crate::capture::GraphicBytes,\n    >'))
    body = replace_once(body, '!self.config.valid()', '!self.config.valid_using::<B>()')
    capture_path = project/'controller-service/src/capture.rs'
    capture = capture_path.read_text()
    start = capture.index('impl HostEpochRegistration {')
    end = capture.index('\n}\n', start)+3
    host_original = capture[start:end]
    host_using = replace_once(host_original, 'pub fn valid(&self)',
                             'pub(crate) fn valid_using<B: GraphicBytes>(&self)')
    host_using = replace_once(host_using,
        'self.boot_id.bytes().all(|b| b.is_ascii_graphic())',
        'B::all_graphic(&self.boot_id)')
    host_using = replace_once(host_using, 'impl HostEpochRegistration {', '''impl HostEpochRegistration {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.valid_using::<StandardGraphicBytes>()
    }
''')
    primitive = '''pub(crate) trait GraphicBytes {
    fn all_graphic(value: &str) -> bool;
}
pub(crate) struct StandardGraphicBytes;
impl GraphicBytes for StandardGraphicBytes {
    fn all_graphic(value: &str) -> bool {
        graphic_bytes(value)
    }
}
pub(crate) fn graphic_bytes(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_graphic())
}

'''
    capture_path.write_text(capture[:start]+primitive+host_using+capture[end:])
    (project/MODEL).write_text(body)
    return dict(maximum_original=maximum_original, maximum_candidate=maximum_candidate, original_host_body=host_original, graphic_bytes_interface=primitive,
                primitive_connection_proved=False, moved_entry_order_body=entry_loop, removed_unused_standard_wrappers=unused_wrappers, historical_split=split_record, shape=shape_changes, lane=lane_changes, entry=entry_changes,
                renewal=renewal_changes, outer_edits=edits, original_equivalence_proved=False,
                production_modified=False, semantic_proof=False)
