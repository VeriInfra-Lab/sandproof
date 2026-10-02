use crate::SandboxId;
use crate::auxiliary::{AccessInput, AuxiliaryDecision, AuxiliaryEvent, AuxiliaryReject};
use crate::effect::{EffectDecision, EffectInput, EffectReject};
use crate::model::{EffectId, Event, World};
use crate::publication::{
    PublicationDecision, PublicationEvent, PublicationInput, PublicationReject,
};
use crate::recovery_monitor::{RuntimeDecision, RuntimeEvent, RuntimeInput, RuntimeReject};
use crate::request::{Decision as RequestDecision, Reject as RequestReject, Request};
use crate::scheduler::{SchedulerInput, SchedulerReject};
use crate::worker::{WorkerDecision, WorkerEvent, WorkerReject};

/// Every state-changing or state-observing input owned by the pure
/// controller boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ControllerInput {
    Maintenance(crate::maintenance::MaintenanceInput),
    Request(Request),
    Effect(EffectInput),
    Scheduler(SchedulerInput),
    Worker(EffectId),
    Publication(PublicationInput),
    RpcTimeout(SandboxId),
    Access(AccessInput),
    Runtime(RuntimeInput),
}

/// Events from all controller sub-protocols, kept tagged so the public entry
/// point cannot confuse two domains with similarly shaped payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControllerEvent {
    Maintenance(crate::maintenance::MaintenanceEvent),
    Protocol(Event),
    Worker(WorkerEvent),
    Publication(PublicationEvent),
    Auxiliary(AuxiliaryEvent),
    Runtime(RuntimeEvent),
}

/// Rejections from all controller sub-protocols.  Machine-boundary failures
/// such as exhausted counters remain visible in their originating domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControllerReject {
    Maintenance(crate::maintenance::MaintenanceReject),
    Request(RequestReject),
    Effect(EffectReject),
    Scheduler(SchedulerReject),
    Worker(WorkerReject),
    Publication(PublicationReject),
    Auxiliary(AuxiliaryReject),
    Runtime(RuntimeReject),
}

/// A successful pure reduction.  The caller must atomically persist `next`
/// before dispatching any newly committed effect from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControllerDecision {
    pub event: ControllerEvent,
    pub next: World,
}

fn protocol_decision(decision: RequestDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Protocol(decision.event),
        next: decision.next,
    }
}

fn effect_decision(decision: EffectDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Protocol(decision.event),
        next: decision.next,
    }
}

fn worker_decision(decision: WorkerDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Worker(decision.event),
        next: decision.next,
    }
}

fn publication_decision(decision: PublicationDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Publication(decision.event),
        next: decision.next,
    }
}

fn auxiliary_decision(decision: AuxiliaryDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Auxiliary(decision.event),
        next: decision.next,
    }
}

fn runtime_decision(decision: RuntimeDecision) -> ControllerDecision {
    ControllerDecision {
        event: ControllerEvent::Runtime(decision.event),
        next: decision.next,
    }
}

/// The single public pure-reducer entry point for the sandbox controller.
///
/// # Errors
///
/// Returns the first ordered rejection of the selected sub-handler.
/// Maintenance additions are not covered by the historical refinement proofs.
/// A rejection never returns a successor state.
pub fn handle_controller(
    world: &World,
    input: ControllerInput,
) -> Result<ControllerDecision, ControllerReject> {
    match input {
        ControllerInput::Maintenance(input) => crate::maintenance::handle_maintenance(world, input)
            .map_err(ControllerReject::Maintenance),
        ControllerInput::Request(request_input) => {
            match crate::request::handle(world, request_input) {
                Ok(decision) => Ok(protocol_decision(decision)),
                Err(reject) => Err(ControllerReject::Request(reject)),
            }
        }
        ControllerInput::Effect(effect_input) => {
            match crate::effect::handle_effect(world, effect_input) {
                Ok(decision) => Ok(effect_decision(decision)),
                Err(reject) => Err(ControllerReject::Effect(reject)),
            }
        }
        ControllerInput::Scheduler(input) => {
            match crate::scheduler::handle_scheduler(world, input) {
                Ok(decision) => Ok(protocol_decision(decision)),
                Err(reject) => Err(ControllerReject::Scheduler(reject)),
            }
        }
        ControllerInput::Worker(effect) => match crate::worker::handle_worker(world, &effect) {
            Ok(decision) => Ok(worker_decision(decision)),
            Err(reject) => Err(ControllerReject::Worker(reject)),
        },
        ControllerInput::Publication(input) => {
            match crate::publication::handle_publication(world, input) {
                Ok(decision) => Ok(publication_decision(decision)),
                Err(reject) => Err(ControllerReject::Publication(reject)),
            }
        }
        ControllerInput::RpcTimeout(sandbox) => {
            match crate::auxiliary::handle_rpc_timeout(world, sandbox) {
                Ok(decision) => Ok(auxiliary_decision(decision)),
                Err(reject) => Err(ControllerReject::Auxiliary(reject)),
            }
        }
        ControllerInput::Access(input) => match crate::auxiliary::handle_access(world, input) {
            Ok(decision) => Ok(auxiliary_decision(decision)),
            Err(reject) => Err(ControllerReject::Auxiliary(reject)),
        },
        ControllerInput::Runtime(input) => {
            match crate::recovery_monitor::handle_runtime(world, input) {
                Ok(decision) => Ok(runtime_decision(decision)),
                Err(reject) => Err(ControllerReject::Runtime(reject)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerEvent, ControllerInput, ControllerReject, handle_controller};
    use crate::model::initial_world;
    use crate::recovery_monitor::{RuntimeEvent, RuntimeInput, RuntimeReject};

    #[test]
    fn unified_entry_routes_runtime_success_and_rejection() {
        let initial = initial_world();
        let crashed = handle_controller(&initial, ControllerInput::Runtime(RuntimeInput::Crash))
            .expect("an available controller can crash");
        assert_eq!(
            crashed.event,
            ControllerEvent::Runtime(RuntimeEvent::ControllerCrash)
        );
        assert_eq!(
            handle_controller(&crashed.next, ControllerInput::Runtime(RuntimeInput::Crash)),
            Err(ControllerReject::Runtime(
                RuntimeReject::ControllerAlreadyDown
            ))
        );
    }
}
