//! Pure registered-template selection/CAS rules shared by storage adapters.
//! Artifact validation and durable writes remain their original dependencies.
use super::{Publication, PublicationState, valid_alias, valid_candidate_id};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    Invalid,
}

/// Validate the existing public template-page boundary before storage access.
/// # Errors
/// Rejects invalid page bounds and cursors other than immutable template IDs.
pub fn validate_page(limit: usize, cursor: Option<&str>) -> Result<(), QueryError> {
    if !(1..=100).contains(&limit)
        || cursor.is_some_and(|c| !c.strip_prefix("tpl-").is_some_and(valid_candidate_id))
    {
        return Err(QueryError::Invalid);
    }
    Ok(())
}

pub struct TemplateSelection {
    pub items: Vec<Publication>,
    pub next_cursor: Option<String>,
}

/// Select from checked committed records, preserving immutable-ID ordering.
/// Unready or foreign templates never become query results through an alias.
/// # Errors
/// Rejects the same invalid limit/cursor as [`validate_page`].
pub fn select_templates(
    owner: u64,
    values: Vec<Publication>,
    limit: usize,
    cursor: Option<&str>,
) -> Result<TemplateSelection, QueryError> {
    validate_page(limit, cursor)?;
    let mut visible: Vec<_> = values
        .into_iter()
        .filter(|value| {
            value.binding.owner == Some(owner)
                && value.state == PublicationState::Ready
                && cursor.is_none_or(|c| value.binding.template_id.as_str() > c)
        })
        .collect();
    visible.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
    let more = visible.len() > limit;
    visible.truncate(limit);
    let next_cursor = more
        .then(|| visible.last().map(|v| v.binding.template_id.clone()))
        .flatten();
    Ok(TemplateSelection {
        items: visible,
        next_cursor,
    })
}

/// Validate one alias update against the same-owner committed Ready target.
/// Static reserved names are checked by the caller against its original catalog.
/// # Errors
/// Rejects foreign/unready targets, alias-to-alias assignment, stale expected
/// identity, invalid names or insertion beyond the existing global 256 bound.
pub fn validate_alias_update(
    owner: u64,
    name: &str,
    target: &str,
    value: &Publication,
    previous_candidate: Option<&str>,
    expected: Option<&str>,
    alias_count: i64,
) -> Result<(), QueryError> {
    validate_alias_target(owner, name, target, value)?;
    validate_alias_expected(previous_candidate, expected)?;
    validate_alias_capacity(previous_candidate, alias_count)
}

/// Validate the immutable target before opening a metadata transaction.
/// # Errors
/// Rejects invalid names, foreign/unready records and alias-to-alias updates.
pub fn validate_alias_target(
    owner: u64,
    name: &str,
    target: &str,
    value: &Publication,
) -> Result<(), QueryError> {
    if !valid_alias(name)
        || value.binding.owner != Some(owner)
        || value.state != PublicationState::Ready
        || target != value.binding.template_id
    {
        return Err(QueryError::Invalid);
    }
    Ok(())
}

/// # Errors
/// Rejects a stale expected immutable template ID before a capacity query.
pub fn validate_alias_expected(
    previous_candidate: Option<&str>,
    expected: Option<&str>,
) -> Result<(), QueryError> {
    if expected
        .is_some_and(|id| previous_candidate.map(|old| format!("tpl-{old}")) != Some(id.to_owned()))
    {
        return Err(QueryError::Invalid);
    }
    Ok(())
}

/// # Errors
/// A new alias may not exceed 256; existing alias updates reserve no new slot.
pub fn validate_alias_capacity(
    previous_candidate: Option<&str>,
    alias_count: i64,
) -> Result<(), QueryError> {
    if previous_candidate.is_none() && alias_count >= 256 {
        return Err(QueryError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::{ResourceShape, TemplateArtifact, TemplateBinding};
    fn publication(id: u8, owner: u64, state: PublicationState) -> Publication {
        let candidate_id = format!("{id:032x}");
        Publication {
            candidate_id: candidate_id.clone(),
            binding: TemplateBinding {
                guest: None,
                template_id: format!("tpl-{candidate_id}"),
                revision: "r1".into(),
                owner: Some(owner),
                kernel: TemplateArtifact {
                    path: "/trusted/kernel".into(),
                    sha256: "1".repeat(64),
                    size_bytes: 4096,
                },
                rootfs: TemplateArtifact {
                    path: "/trusted/rootfs".into(),
                    sha256: "2".repeat(64),
                    size_bytes: 1024 * 1024 * 1024,
                },
                resources: ResourceShape {
                    cpu_count: 1,
                    memory_mib: 128,
                    host_memory_bytes: 129 * 1024 * 1024,
                    disk_bytes: 1024 * 1024 * 1024,
                },
                envd_version: "0.9.0".into(),
                startup: None,
            },
            image_digest: "3".repeat(64),
            free_bytes: 4096,
            min_free_bytes: 4096,
            state,
            checked: false,
            validation_sandbox: None,
            validation_certificate: None,
            failure: None,
        }
    }

    #[test]
    fn owned_ready_pages_preserve_cursor_and_stable_id_order() {
        let ready = PublicationState::Ready;
        let values = vec![
            publication(4, 1, ready),
            publication(2, 1, ready),
            publication(1, 2, ready),
            publication(3, 1, PublicationState::Checking),
        ];
        let selected = select_templates(1, values.clone(), 1, None).unwrap();
        assert_eq!(selected.items[0].candidate_id, format!("{:032x}", 2));
        let cursor = selected.next_cursor.unwrap();
        let next = select_templates(1, values, 1, Some(&cursor)).unwrap();
        assert_eq!(next.items[0].candidate_id, format!("{:032x}", 4));
        assert!(next.next_cursor.is_none());
        for limit in [0, 101] {
            assert!(validate_page(limit, None).is_err());
        }
        for cursor in ["alias", "tpl-bad", ""] {
            assert!(validate_page(1, Some(cursor)).is_err());
        }
    }

    #[test]
    fn alias_cas_owner_and_global_limit_match_original_contract() {
        let value = publication(1, 1, PublicationState::Ready);
        let target = value.binding.template_id.as_str();
        assert!(validate_alias_update(1, "latest", target, &value, None, None, 255).is_ok());
        assert!(validate_alias_update(1, "latest", target, &value, None, None, 256).is_err());
        // Updating an existing alias still works at the hard bound.
        assert!(
            validate_alias_update(
                1,
                "latest",
                target,
                &value,
                Some(&value.candidate_id),
                Some(target),
                256
            )
            .is_ok()
        );
        assert!(validate_alias_update(1, "latest", target, &value, None, Some(target), 0).is_err());
        assert!(validate_alias_update(2, "latest", target, &value, None, None, 0).is_err());
        assert!(
            validate_alias_update(1, "latest", "another-alias", &value, None, None, 0).is_err()
        );
        assert!(validate_alias_update(1, "tpl-reserved", target, &value, None, None, 0).is_err());
        let unready = publication(1, 1, PublicationState::Checking);
        assert!(validate_alias_update(1, "latest", target, &unready, None, None, 0).is_err());
    }
}
