//! Current local eligibility to accept readonly source facts. This is not a
//! physical source observation, snapshot compatibility or a capture permission.
//! The accepted version is positive under V3 validation; this first-Create
//! profile starts epoch/generation at one in `transition::begin_create`.
//! Native owner/VM identifiers keep their original types, including zero.
use crate::{Phase, TenantId, VmId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceObservationGuard {
    pub work_version: u64,
    pub current_version: u64,
    pub work_owner: TenantId,
    pub current_owner: TenantId,
    pub work_vm: VmId,
    pub current_vm: Option<VmId>,
    pub work_generation: u64,
    pub current_generation: u64,
    pub work_epoch: u64,
    pub current_epoch: u64,
    pub phase: Phase,
    pub terminal: bool,
    pub now_ms: i64,
    pub deadline_ms: i64,
}

impl SourceObservationGuard {
    #[must_use]
    pub fn allows(self) -> bool {
        let Some(vm) = self.current_vm else {
            return false;
        };
        self.work_version > 0
            && self.work_version == self.current_version
            && self.work_owner == self.current_owner
            && self.work_vm == vm
            && self.work_generation > 0
            && self.work_generation == self.current_generation
            && self.work_epoch > 0
            && self.work_epoch == self.current_epoch
            && self.phase == Phase::Running
            && !self.terminal
            && self.now_ms >= 0
            && self.now_ms < self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn live() -> SourceObservationGuard {
        SourceObservationGuard {
            work_version: 2,
            current_version: 2,
            work_owner: TenantId(3),
            current_owner: TenantId(3),
            work_vm: VmId(4),
            current_vm: Some(VmId(4)),
            work_generation: 1,
            current_generation: 1,
            work_epoch: 5,
            current_epoch: 5,
            phase: Phase::Running,
            terminal: false,
            now_ms: 10,
            deadline_ms: 20,
        }
    }
    #[test]
    fn only_the_same_live_current_source_can_accept_read_facts() {
        assert!(live().allows());
        // Native VM allocation starts at zero; typed IDs need equality, not a
        // positive-integer rule borrowed from the external protocol.
        assert!(
            SourceObservationGuard {
                work_vm: VmId(0),
                current_vm: Some(VmId(0)),
                ..live()
            }
            .allows()
        );
        assert!(
            SourceObservationGuard {
                work_owner: TenantId(0),
                current_owner: TenantId(0),
                ..live()
            }
            .allows()
        );
        let cases = [
            SourceObservationGuard {
                current_version: 3,
                ..live()
            },
            SourceObservationGuard {
                work_version: 0,
                ..live()
            },
            SourceObservationGuard {
                current_owner: TenantId(4),
                ..live()
            },
            SourceObservationGuard {
                work_owner: TenantId(0),
                ..live()
            },
            SourceObservationGuard {
                current_vm: None,
                ..live()
            },
            SourceObservationGuard {
                current_vm: Some(VmId(5)),
                ..live()
            },
            SourceObservationGuard {
                work_vm: VmId(0),
                ..live()
            },
            SourceObservationGuard {
                current_generation: 2,
                ..live()
            },
            SourceObservationGuard {
                work_generation: 0,
                ..live()
            },
            SourceObservationGuard {
                current_epoch: 6,
                ..live()
            },
            SourceObservationGuard {
                work_epoch: 0,
                ..live()
            },
            SourceObservationGuard {
                phase: Phase::Paused,
                ..live()
            },
            SourceObservationGuard {
                terminal: true,
                ..live()
            },
            SourceObservationGuard {
                now_ms: -1,
                ..live()
            },
            SourceObservationGuard {
                now_ms: 20,
                ..live()
            },
            SourceObservationGuard {
                now_ms: 21,
                ..live()
            },
        ];
        for stale in cases {
            assert!(!stale.allows());
        }
    }
}
