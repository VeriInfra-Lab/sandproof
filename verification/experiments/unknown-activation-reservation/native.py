"""Original checkpoint refusal of a second reservation after unknown activation.

A private wrong retry policy reopens only the same restore plan whose exact
activation has OutcomeUnknown. The original checkpoint test must reject an
erroneous new startup-resource decision at its unchanged final assertion.
This boundary does not execute a second Load or a physical VM.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

HERE = Path('verification/experiments/unknown-activation-reservation')
ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location('unknown_activation_native_runner',
    ROOT / 'verification/experiments/unknown-host-release/native.py')
runner = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = runner
SPEC.loader.exec_module(runner)

SOURCE = Path('controller-core/src/scheduler.rs')
FIXTURE = Path('controller-core/src/checkpoint/tests/resume.rs')
PACKAGE = 'sandbox-controller-core'
PACKAGE_DIR = Path('controller-core')
CRATE = 'sandbox_controller_core'
MUTANT = 'unknown-activation-reopens-reservation'
KIND = 'unknown-activation-reservation-native'
FIXTURE_SCOPE = 'complete original checkpoint Resume file and scheduler test body; all other committed inputs also checked'
BEFORE = '''    if plan.sealed {
        return Err(SchedulerReject::PlanAlreadySealed);
    }
    if plan.vm.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.slot.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
'''
AFTER = '''    let retry_unknown_restore = plan.sealed
        && plan.slot.is_some()
        && plan.restore_from.is_some()
        && world.effect_ids().iter().any(|effect| {
            world.effect_status(effect) == Some(crate::EffectStatus::OutcomeUnknown)
                && world.logical_effect(&effect.logical).is_some_and(|logical| {
                    logical.origin == crate::EffectOrigin::Operation(key)
                })
                && matches!(effect.command(), Command::ActivateVm { sandbox, epoch, generation, vm }
                    if *sandbox == key.sandbox && *epoch == plan.epoch
                        && *generation == plan.generation && plan.vm == Some(*vm))
        });
    if plan.sealed && !retry_unknown_restore {
        return Err(SchedulerReject::PlanAlreadySealed);
    }
    if plan.vm.is_some() && !retry_unknown_restore {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.slot.is_some() && !retry_unknown_restore {
        return Err(SchedulerReject::PlanNotOpen);
    }
'''
FIRST = '\npub fn handle_reserve_startup('
LAST = '\n/// Reserve a pause snapshot'
TEST_BODY = '\n#[cfg(test)]\nmod tests {'
PREFIX = 'checkpoint::tests::resume::'
PROPERTY = PREFIX + 'unknown_activation_survives_restart_and_never_requests_another_load'
TESTS = (PROPERTY,
    PREFIX + 'original_resume_driver_advances_after_its_fence',
    'scheduler::tests::startup_rejections_follow_the_lean_guard_order',
    PREFIX + 'two_restore_cycles_roundtrip_every_microstep_without_reusing_old_vm',
    'terminal::tests::unknown_activation_is_covered_by_the_current_kill_epoch_without_reactivation')
ASSERTION = b'SchedulerInput::ReserveStartup'
MARKER = '''    assert!(
        handle_controller(
            &world,
            ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
                sandbox: SID,
                operation: OperationId(3)
            }))
        )
        .is_err()
    );'''


def scheduler_tests(project):
    text = (project / SOURCE).read_text()
    if text.count(TEST_BODY) != 1:
        raise ValueError('Require the unique complete original scheduler test body')
    return text[text.index(TEST_BODY):]


def fixture_digest(project):
    inputs = dict(resume=runner.sha(project / FIXTURE),
        scheduler_tests=hashlib.sha256(scheduler_tests(project).encode()).hexdigest())
    return hashlib.sha256(json.dumps(inputs,sort_keys=True).encode()).hexdigest()


def mutate(project):
    path = project / SOURCE
    text = path.read_text()
    first = text.index(FIRST)
    last = text.index(LAST, first)
    fixture = fixture_digest(project)
    if text.count(BEFORE) != 1 or text[first:last].count(BEFORE) != 1:
        raise ValueError('Require the unique original sealed/VM/slot reservation gate in its handler')
    changed = text.replace(BEFORE, AFTER, 1)
    if changed[changed.index(TEST_BODY):] != scheduler_tests(project):
        raise ValueError('Mutation would change the complete original scheduler tests')
    path.write_text(changed)
    if fixture_digest(project) != fixture:
        raise ValueError('Mutation changed the complete original Rust fixtures')


def rejection_line(project):
    text = (project / FIXTURE).read_text()
    first = text.index('\nfn ' + PROPERTY.rsplit('::',1)[1] + '() {')
    last = text.index('\n#[test]', first)
    if text[first:last].count(MARKER) != 1:
        raise ValueError('Require the exact original post-recovery reservation-refusal assertion')
    return text[:text.index(MARKER, first)].count('\n') + 1


def checked_execution(stdout, stderr, code, name, rejected, line):
    if rejected and name != PROPERTY:
        raise ValueError('Only the original unknown-activation reservation assertion is the counterexample')
    message = runner.checked_execution(stdout,stderr,code,name,rejected,line,
        fixture=FIXTURE,assertion=ASSERTION)
    if rejected and b'.is_err()' not in message:
        raise ValueError('Require the complete original reservation rejection expression')


if __name__ == '__main__':
    if sys.platform != 'linux':
        raise SystemExit('This actual checkpoint reservation entry requires the fixed Linux toolchain')
    runner.main(case=sys.modules[__name__])
