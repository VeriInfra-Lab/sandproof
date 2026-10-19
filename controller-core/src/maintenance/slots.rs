//! Pure physical-position protocol. A position is reusable; its VM identity is not.
//! Host adapters must persist/check the same binding before every side effect.

use crate::{Generation, NetSlotId, SandboxId, TenantId, VmId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HostSlotId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HostBinding {
    pub owner: TenantId,
    pub sandbox: SandboxId,
    pub generation: Generation,
    pub vm: VmId,
    pub network: NetSlotId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HostSlotPhase {
    Occupied,
    Releasing,
    Free,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct HostSlotRecord {
    pub binding: HostBinding,
    pub phase: HostSlotPhase,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SlotInput {
    Bind {
        position: HostSlotId,
        expected_previous: Option<VmId>,
        binding: HostBinding,
    },
    BeginRelease {
        position: HostSlotId,
        binding: HostBinding,
    },
    /// Only the trusted adapter may supply this acknowledgement, after exact
    /// physical resource absence AND old workers/deliveries, data connections,
    /// relays and release-blocking pins have drained. A `CleanupVm` receipt alone
    /// is insufficient. The adapter must keep this quiescence true through the
    /// durable release and future rebind under its serial execution boundary.
    /// A lost acknowledgement leaves Releasing, never Free. This pure input
    /// cannot itself inspect any of those physical facts.
    ConfirmRelease {
        position: HostSlotId,
        binding: HostBinding,
    },
    CheckOccupied {
        position: HostSlotId,
        binding: HostBinding,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotReject {
    OutOfRange,
    BindingMismatch,
    NotFree,
    NotOccupied,
    NotReleasing,
    StaleIdentity,
    IdentityAlreadyBound,
}

/// The pure compare-and-transition used by both the core and durable host
/// binding adapter. The caller must serialize the check with its actual I/O.
///
/// # Errors
/// Rejects stale identities, conflicting positions and invalid release order.
pub fn reduce_slot(
    previous: Option<HostSlotRecord>,
    input: SlotInput,
) -> Result<HostSlotRecord, SlotReject> {
    match input {
        SlotInput::Bind {
            expected_previous,
            binding,
            ..
        } => {
            // Preserve the first rejection without generic Option comparison.
            let expected_matches = match (previous, expected_previous) {
                (None, None) => true,
                (Some(row), Some(expected)) => row.binding.vm == expected,
                _ => false,
            };
            if !expected_matches {
                return Err(SlotReject::BindingMismatch);
            }
            if let Some(prior) = previous {
                if prior.phase != HostSlotPhase::Free {
                    return Err(SlotReject::NotFree);
                }
                if binding.vm.0 <= prior.binding.vm.0 {
                    return Err(SlotReject::StaleIdentity);
                }
            }
            Ok(HostSlotRecord {
                binding,
                phase: HostSlotPhase::Occupied,
            })
        }
        SlotInput::BeginRelease { binding, .. } => {
            let row = previous.ok_or(SlotReject::BindingMismatch)?;
            if row.binding != binding {
                return Err(SlotReject::BindingMismatch);
            }
            if row.phase == HostSlotPhase::Free {
                return Err(SlotReject::NotOccupied);
            }
            Ok(HostSlotRecord {
                binding,
                phase: HostSlotPhase::Releasing,
            })
        }
        SlotInput::ConfirmRelease { binding, .. } => {
            let row = previous.ok_or(SlotReject::BindingMismatch)?;
            if row.binding != binding {
                return Err(SlotReject::BindingMismatch);
            }
            if row.phase == HostSlotPhase::Occupied {
                return Err(SlotReject::NotReleasing);
            }
            Ok(HostSlotRecord {
                binding,
                phase: HostSlotPhase::Free,
            })
        }
        SlotInput::CheckOccupied { binding, .. } => {
            let row = previous.ok_or(SlotReject::BindingMismatch)?;
            if row.binding != binding {
                return Err(SlotReject::BindingMismatch);
            }
            if row.phase != HostSlotPhase::Occupied {
                return Err(SlotReject::NotOccupied);
            }
            Ok(row)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(vm: u64) -> HostBinding {
        HostBinding {
            owner: TenantId(1),
            sandbox: SandboxId(vm),
            generation: 1,
            vm: VmId(vm),
            network: NetSlotId(vm),
        }
    }

    #[test]
    fn explicit_bind_checks_match_original_order_and_integer_boundaries() {
        let identities = [0, 1, u64::MAX - 1, u64::MAX];
        let expected = [None, Some(VmId(0)), Some(VmId(1)), Some(VmId(u64::MAX))];
        let phases = [
            HostSlotPhase::Occupied,
            HostSlotPhase::Releasing,
            HostSlotPhase::Free,
        ];
        let mut previous_values = vec![None];
        for vm in identities {
            for phase in phases {
                previous_values.push(Some(HostSlotRecord {
                    binding: binding(vm),
                    phase,
                }));
            }
        }
        for previous in previous_values {
            for expected_previous in expected {
                for vm in identities {
                    for position in [HostSlotId(0), HostSlotId(u64::MAX)] {
                        let binding = binding(vm);
                        // Original expression and rejection order, kept only as a regression oracle.
                        let original = if previous.map(|row| row.binding.vm) != expected_previous {
                            Err(SlotReject::BindingMismatch)
                        } else if previous.is_some_and(|row| row.phase != HostSlotPhase::Free) {
                            Err(SlotReject::NotFree)
                        } else if previous.is_some_and(|row| binding.vm <= row.binding.vm) {
                            Err(SlotReject::StaleIdentity)
                        } else {
                            Ok(HostSlotRecord {
                                binding,
                                phase: HostSlotPhase::Occupied,
                            })
                        };
                        assert_eq!(
                            reduce_slot(
                                previous,
                                SlotInput::Bind {
                                    position,
                                    expected_previous,
                                    binding,
                                }
                            ),
                            original
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn lost_release_and_old_messages_cannot_rebind_or_touch_successor() {
        let position = HostSlotId(0);
        let old = binding(0);
        let occupied = reduce_slot(
            None,
            SlotInput::Bind {
                position,
                expected_previous: None,
                binding: old,
            },
        )
        .unwrap();
        assert_eq!(
            reduce_slot(
                Some(occupied),
                SlotInput::ConfirmRelease {
                    position,
                    binding: old
                }
            ),
            Err(SlotReject::NotReleasing)
        );
        let releasing = reduce_slot(
            Some(occupied),
            SlotInput::BeginRelease {
                position,
                binding: old,
            },
        )
        .unwrap();
        assert_eq!(
            reduce_slot(
                Some(releasing),
                SlotInput::Bind {
                    position,
                    expected_previous: Some(old.vm),
                    binding: binding(1),
                }
            ),
            Err(SlotReject::NotFree)
        );
        let free = reduce_slot(
            Some(releasing),
            SlotInput::ConfirmRelease {
                position,
                binding: old,
            },
        )
        .unwrap();
        assert_eq!(free.binding, old);
        assert_eq!(
            reduce_slot(
                Some(free),
                SlotInput::CheckOccupied {
                    position,
                    binding: old
                }
            ),
            Err(SlotReject::NotOccupied)
        );
        assert_eq!(
            reduce_slot(
                Some(free),
                SlotInput::Bind {
                    position,
                    expected_previous: Some(old.vm),
                    binding: old,
                }
            ),
            Err(SlotReject::StaleIdentity)
        );
        let new = binding(1);
        let successor = reduce_slot(
            Some(free),
            SlotInput::Bind {
                position,
                expected_previous: Some(old.vm),
                binding: new,
            },
        )
        .unwrap();
        for delayed in [
            SlotInput::BeginRelease {
                position,
                binding: old,
            },
            SlotInput::ConfirmRelease {
                position,
                binding: old,
            },
            SlotInput::CheckOccupied {
                position,
                binding: old,
            },
        ] {
            assert_eq!(
                reduce_slot(Some(successor), delayed),
                Err(SlotReject::BindingMismatch)
            );
        }
    }

    #[test]
    fn one_position_keeps_constant_metadata_across_many_incarnations() {
        let position = HostSlotId(0);
        let mut row = None;
        for vm in 0..1000 {
            let binding = binding(vm);
            row = Some(
                reduce_slot(
                    row,
                    SlotInput::Bind {
                        position,
                        expected_previous: row.map(|r| r.binding.vm),
                        binding,
                    },
                )
                .unwrap(),
            );
            row = Some(reduce_slot(row, SlotInput::BeginRelease { position, binding }).unwrap());
            row = Some(reduce_slot(row, SlotInput::ConfirmRelease { position, binding }).unwrap());
        }
        assert_eq!(row.unwrap().binding.vm, VmId(999));
    }
}
