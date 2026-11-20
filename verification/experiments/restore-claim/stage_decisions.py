"""Move selected-record decisions into named private extraction roots."""
import argparse
from hashlib import sha256
from pathlib import Path
from textwrap import dedent


RESTORE = Path('controller-service/src/controller/lifecycle_inbox/source_work/restore.rs')
RESTORE_SHA256 = '3bc21d0f6e354de0718a254ec0cf9dd73389d3afc7e906aeb1fb880d32f046be'
ACTIVATION = '''    if let Some(original) = &claim.activation {
        return Ok(original == action);
    }
'''
ACTIVATION_BEFORE_COMPARISON = '''    if !claim.material_use {
        return Ok(false);
    }
    if !matches!(action.command.command(), core::Command::ActivateVm { .. }) {
        return Ok(true);
    }
'''
ACTIVATION_WRITE = '    claim.activation = Some(action.clone());\n'
CLAIM = '''        if let Some(original) = &sources.records[index].restore {
            return if original.intent == *intent {
                Ok(inbox.state(s, e))
            } else {
                Err(JournalError::Conflict)
            };
        }
'''
RESUME_PREFIX = '''            claim.local.sandbox == request.sandbox
                && claim.local.operation == request.operation
                && request.actor == core::Actor::Tenant(claim.local.owner)
                && row.owner == claim.local.owner
                && !row.terminal
                && row.generation.checked_add(1) == Some(claim.generation)
                && row.phase == core::Phase::Paused
                && claim.material_use
                && claim.activation.is_none()'''


def replace_once(source, old, new):
    if source.count(old) != 1:
        raise ValueError('production statement differs from the selected source')
    return source.replace(old, new)


def stage(project, wrong):
    path = project / RESTORE
    original = path.read_bytes()
    if sha256(original).hexdigest() != RESTORE_SHA256:
        raise ValueError('production restore.rs differs from the fixed baseline')
    source = original.decode()
    activation = ACTIVATION.replace('return Ok(', 'return Some(')
    if wrong:
        activation = replace_once(activation, 'return Some(original == action);',
                                  'let _ = (original, action);\n        return Some(true);')
    source = replace_once(source, ACTIVATION_BEFORE_COMPARISON + ACTIVATION + ACTIVATION_WRITE, '''    if let Some(decision) = proof_claim_activation_row(claim, action) {
        return Ok(decision);
    }
''')
    source = replace_once(source, CLAIM, '''        if let Some(same_intent) = proof_existing_restore(&sources.records[index].restore, intent) {
            return if same_intent {
                Ok(inbox.state(s, e))
            } else {
                Err(JournalError::Conflict)
            };
        }
''')
    source = replace_once(source, RESUME_PREFIX,
                          '            proof_resume_prefix(claim, row, request)')
    source += '''
fn proof_existing_activation(
    claim: &RestoreClaim,
    action: &WorkAction<core::EffectId>,
) -> Option<bool> {
''' + activation + '''    None
}

// Some forwards the original early return. None reaches codec::save.
fn proof_claim_activation_row(
    claim: &mut RestoreClaim,
    action: &WorkAction<core::EffectId>,
) -> Option<bool> {
''' + ACTIVATION_BEFORE_COMPARISON.replace('return Ok(', 'return Some(') + '''    if let Some(decision) = proof_existing_activation(claim, action) {
        return Some(decision);
    }
''' + ACTIVATION_WRITE + '''    None
}

fn proof_existing_restore(
    restore: &Option<RestoreClaim>,
    intent: &AcceptedLifecycleIntent,
) -> Option<bool> {
    if let Some(original) = restore {
        return Some(original.intent == *intent);
    }
    None
}
'''
    source += '''
fn proof_resume_prefix(
    claim: &RestoreClaim,
    row: &core::SandboxRecord,
    request: &core::ResumeRequest,
) -> bool {
''' + '\n'.join('    ' + line for line in dedent(RESUME_PREFIX).splitlines()) + '\n}\n'
    path.write_text(source)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('--wrong', action='store_true')
    args = parser.parse_args()
    stage(args.project, args.wrong)
