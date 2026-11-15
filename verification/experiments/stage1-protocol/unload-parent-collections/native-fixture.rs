#[cfg(test)]
mod collection_link_contract {
    use super::{original_action_identities, original_receipt_digests};
    use sandbox_controller_core as core;
    use sandproof_protocol::{
        ActionIdentity, LogicalActionId, OperationId, ResourceId, ResourceKind, ResourceRef,
        SandboxId, TenantId, WorkAction,
    };
    use sha2::{Digest, Sha256};

    fn action(suffix: &str, attempt: u64) -> WorkAction<core::EffectId> {
        WorkAction {
            identity: ActionIdentity {
                protocol_version: sandproof_protocol::PROTOCOL_VERSION,
                owner: TenantId::try_from(format!("owner-{suffix}")).unwrap(),
                sandbox: SandboxId::try_from(format!("sandbox-{suffix}")).unwrap(),
                operation: OperationId::try_from(format!("operation-{suffix}")).unwrap(),
                epoch: 7 + attempt,
                generation: 13 + attempt,
                logical_action: LogicalActionId::try_from(format!("action-{suffix}")).unwrap(),
                attempt,
                resource: ResourceRef {
                    kind: ResourceKind::NetworkSlot,
                    id: ResourceId::try_from(format!("slot-{suffix}")).unwrap(),
                },
            },
            command: core::EffectId {
                logical: core::LogicalEffectId {
                    command: core::Command::CleanupSlot {
                        sandbox: core::SandboxId::new(41),
                        generation: 29,
                        slot: core::NetSlotId::new(43),
                    },
                },
                attempt,
            },
        }
    }

    #[test]
    fn full_identity_contract() {
        let first = action("a", 5);
        let second = action("b", 9);
        first.identity.validate_version().unwrap();
        second.identity.validate_version().unwrap();
        let expected = vec![
            first.identity.clone(),
            second.identity.clone(),
            first.identity.clone(),
        ];
        let values = vec![first.clone(), second, first];
        assert_eq!(original_action_identities(&values), expected);
        assert_eq!(values[0].identity.attempt, 5);
        assert!(original_action_identities(&[]).is_empty());
    }

    #[test]
    fn full_receipt_contract() {
        let receipts = vec![vec![0, 1, 255], vec![], vec![4, 3, 2, 1], vec![0, 1, 255]];
        let expected: Vec<[u8; 32]> = receipts
            .iter()
            .map(|raw| Sha256::digest(raw).into())
            .collect();
        let actual: Vec<[u8; 32]> = original_receipt_digests(&receipts)
            .into_iter()
            .map(|value| value.into_bytes())
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(receipts[0], vec![0, 1, 255]);
        assert!(original_receipt_digests(&[]).is_empty());
    }
}
