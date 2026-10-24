//! Original core transitions distinguish never-issued and unknown host work.
use super::{core, effect_needs_recovery};

fn apply(world: &core::World, input: core::ControllerInput) -> core::World {
    core::handle_controller(world, input).unwrap().next
}
fn pending_startup() -> (core::World, core::EffectId) {
    let world = apply(
        &core::initial_world(),
        core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            owner: core::TenantId(1),
            sandbox: core::SandboxId(1),
            workspace: core::FsId(1),
            file_policy: core::FilePolicyRef {
                digest: core::PolicyDigest(1),
            },
            policy: core::NetworkPolicy::DenyAll,
            operation: core::OperationId(1),
        })),
    );
    let startup = world
        .effect_ids()
        .iter()
        .find(|id| {
            matches!(
                id.command(),
                core::Command::InstallFence {
                    terminal: false,
                    ..
                }
            )
        })
        .unwrap()
        .clone();
    (world, startup)
}
fn terminate(world: &core::World) -> core::World {
    apply(
        world,
        core::ControllerInput::Request(core::Request::Kill(core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(1),
            operation: core::OperationId(2),
        })),
    )
}

#[test]
fn terminal_intent_excludes_only_never_dispatched_startup_from_recovery() {
    let (world, startup) = pending_startup();
    assert!(effect_needs_recovery(&world, &startup));
    let terminal = terminate(&world);
    assert_eq!(
        terminal.effect_status(&startup),
        Some(core::EffectStatus::Committed)
    );
    assert!(core::effect::startup_dispatch_revoked(
        &terminal,
        startup.command()
    ));
    assert_eq!(
        core::effect::handle_effect_dispatch(&terminal, startup.clone()).unwrap_err(),
        core::EffectReject::NotDispatchable
    );
    assert!(!effect_needs_recovery(&terminal, &startup));
    let fence = terminal
        .effect_ids()
        .iter()
        .find(|id| {
            matches!(
                id.command(),
                core::Command::InstallFence { terminal: true, .. }
            )
        })
        .unwrap();
    assert_eq!(
        terminal.effect_status(fence),
        Some(core::EffectStatus::Committed)
    );
    assert!(effect_needs_recovery(&terminal, fence));
}

#[test]
fn terminal_intent_keeps_dispatched_and_unknown_startup_responsibility() {
    for unknown in [false, true] {
        let (world, startup) = pending_startup();
        let dispatched = apply(
            &world,
            core::ControllerInput::Effect(core::EffectInput::Dispatch(startup.clone())),
        );
        let (world, expected) = if unknown {
            (
                apply(
                    &dispatched,
                    core::ControllerInput::Effect(core::EffectInput::Timeout(startup.clone())),
                ),
                core::EffectStatus::OutcomeUnknown,
            )
        } else {
            (dispatched, core::EffectStatus::Dispatched)
        };
        let terminal = terminate(&world);
        assert!(core::effect::startup_dispatch_revoked(
            &terminal,
            startup.command()
        ));
        assert_eq!(terminal.effect_status(&startup), Some(expected));
        assert!(effect_needs_recovery(&terminal, &startup));
    }
}
