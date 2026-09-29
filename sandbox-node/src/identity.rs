//! Pure host identity checks, shared by startup validation and VM preparation.

pub(crate) fn validate_vm_id_range(uid_base: u32, max_vm_id: u64) -> Result<(), &'static str> {
    if uid_base == 0
        || max_vm_id == 0
        || u64::from(uid_base)
            .checked_add(max_vm_id)
            .is_none_or(|last_uid| last_uid >= u64::from(u32::MAX))
    {
        return Err("invalid node resource limits");
    }
    Ok(())
}

pub(crate) fn vm_uid(uid_base: u32, max_vm_id: u64, vm: u64) -> Result<u32, &'static str> {
    let id = u32::try_from(vm).map_err(|_| "VM ID exceeds u32 range")?;
    if vm > max_vm_id {
        return Err("node VM ID exceeds allocated UID range");
    }
    let uid = uid_base.checked_add(id).ok_or("VM uid overflow")?;
    if uid == u32::MAX {
        return Err("VM uid is reserved");
    }
    Ok(uid)
}

#[cfg(test)]
mod tests {
    use super::{validate_vm_id_range, vm_uid};

    #[test]
    fn deployment_range_excludes_root_reserved_uid_and_overflow() {
        for (base, maximum) in [
            (0, 1),
            (1, 0),
            (u32::MAX - 1, 1),
            (1, u64::from(u32::MAX)),
            (1000, u64::MAX),
        ] {
            assert!(validate_vm_id_range(base, maximum).is_err());
        }
        for (base, maximum) in [(1000, 1), (1, u64::from(u32::MAX - 2)), (u32::MAX - 2, 1)] {
            assert_eq!(validate_vm_id_range(base, maximum), Ok(()));
            assert_eq!(vm_uid(base, maximum, 0), Ok(base));
            assert_eq!(
                vm_uid(base, maximum, maximum),
                Ok(base + u32::try_from(maximum).unwrap())
            );
            assert!(vm_uid(base, maximum, maximum + 1).is_err());
        }
    }

    #[test]
    fn preparation_rejects_unrepresentable_or_reserved_identity() {
        assert!(vm_uid(1000, u64::MAX, u64::from(u32::MAX) + 1).is_err());
        assert!(vm_uid(1000, 1, 2).is_err());
        assert!(vm_uid(u32::MAX - 1, 2, 2).is_err());
        assert!(vm_uid(u32::MAX - 1, 1, 1).is_err());
        assert_eq!(vm_uid(u32::MAX - 2, 1, 1), Ok(u32::MAX - 1));
    }
}
