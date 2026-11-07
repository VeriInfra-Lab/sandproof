//! Exact SDK renewal acknowledgements on the original journal. Confirmation
//! permits bounded history compaction, never execution, extension or release.
use super::{
    authenticated, codec,
    model::{Entry, Session},
    source_work,
};
use crate::controller::transaction::WorldTransaction;
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, ConfirmedRenewal, ExpectedSession,
    LifecycleOperation, RENEWAL_CONFIRMATION_BODY_BYTES, RenewalConfirmation,
};

#[cfg(test)]
#[path = "renewal_confirmation_tests.rs"]
mod tests;

pub(super) fn is_sdk_renewal(intent: &AcceptedLifecycleIntent) -> bool {
    intent.request.binding.key.window.get() == 2
        && matches!(intent.request.operation, LifecycleOperation::Renew { .. })
}

#[expect(
    clippy::explicit_iter_loop,
    reason = "fixed extraction uses the registered slice iterator"
)]
pub(super) fn has_unconfirmed_sdk(session: &Session) -> bool {
    for entry in session.entries.iter() {
        if is_sdk_renewal(&entry.intent)
            && session.confirmed_renewal.as_ref().is_none_or(|confirmed| {
                entry.intent.accepted_version > confirmed.request.accepted_version
            })
        {
            return true;
        }
    }
    false
}

/// The exact deletion decision, independent of SQL or a network callback.
/// Only the previous full checkpoint's own applied SDK row can disappear.
/// Public, current, unapplied and original-operation-pinned rows survive.
fn discard_previous(
    entry: &Entry,
    previous: &RenewalConfirmation,
    current_version: u64,
    source_pinned: bool,
) -> bool {
    crate::lifecycle_renewal::can_discard_confirmed_sdk(
        &entry.intent,
        entry.applied_deadline,
        entry.previous_effective_end_at_ms,
        entry.local_operation,
        previous,
        current_version,
        source_pinned,
    )
}

/// Validate the retained full checkpoint and the actual removed frontier.
/// Reachability of this summary is a business proof obligation of the concrete
/// transition below; a digest/shape check does not prove a history occurred.
pub(super) trait RenewalEncoding {
    fn encode(value: &RenewalConfirmation) -> Result<Vec<u8>, serde_json::Error>;
}
pub(super) struct StandardRenewalEncoding;
impl RenewalEncoding for StandardRenewalEncoding {
    fn encode(value: &RenewalConfirmation) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }
}

pub(super) fn validate_session_using<E: RenewalEncoding>(
    session: &Session,
) -> Result<(), JournalError> {
    let Some(confirmed) = session.confirmed_renewal.as_ref() else {
        return if session.compacted_sdk_through.is_none() {
            Ok(())
        } else {
            Err(JournalError::CorruptStore)
        };
    };
    if !confirmed.is_sdk_renewal()
        || E::encode(confirmed)?.len() > RENEWAL_CONFIRMATION_BODY_BYTES
        || session.local.is_none()
        || checkpoint_entry_invalid(session, confirmed)
        || session.compacted_sdk_through.is_some_and(|through| {
            through < 2
                || through >= confirmed.request.accepted_version
                || contains_accepted_version(session, through)
        })
        || has_unapplied_sdk_through(session, confirmed.request.accepted_version)
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

/// Exact adjacency remains required above the actually removed SDK frontier.
/// A retained gap can only precede or reach the full saved confirmation.
/// This does not authorize an arbitrary missing public/unknown row: only
/// `discard_previous` can create these gaps, and its preservation must be proved.
pub(super) fn follows_retained(
    session: &Session,
    previous: u64,
    intent: &AcceptedLifecycleIntent,
) -> bool {
    match intent.request.binding.expected {
        ExpectedSession::Absent => previous == 0,
        ExpectedSession::Version(expected) if expected == previous => previous > 0,
        ExpectedSession::Version(expected) => {
            previous > 0
                && expected > previous
                && session
                    .compacted_sdk_through
                    .is_some_and(|through| expected <= through)
                && session.confirmed_renewal.as_ref().is_some_and(|confirmed| {
                    intent.accepted_version <= confirmed.request.accepted_version
                })
        }
    }
}

/// The last saved application must still equal the actual native lifetime.
/// Exact previous ends are checked along retained adjacency. Across an actual
/// SDK gap the saved predecessor remains explicit and bounded; it is not
/// recomputed from Create or from the final native deadline.
pub(super) fn validate_lifetimes(
    session: &Session,
    lifetime: crate::Lifetime,
) -> Result<(), JournalError> {
    let start = lifetime.started_at_ms.ok_or(JournalError::CorruptStore)?;
    let initial = session
        .entries
        .first()
        .and_then(|entry| {
            if let LifecycleOperation::Create { create, .. } = &entry.intent.request.operation {
                i64::try_from(create.timeout_ms)
                    .ok()
                    .and_then(|duration| start.checked_add(duration))
            } else {
                None
            }
        })
        .ok_or(JournalError::CorruptStore)?;
    if initial > lifetime.max_deadline_ms {
        return Err(JournalError::CorruptStore);
    }
    validate_applied_lifetimes(session, lifetime, start, initial)
}

fn validate_applied_lifetimes(
    session: &Session,
    lifetime: crate::Lifetime,
    start: i64,
    initial: i64,
) -> Result<(), JournalError> {
    let mut prior_end = Some(initial);
    let mut prior_version = 0;
    let mut last_applied = None;
    for entry in &session.entries {
        if prior_version > 0
            && entry.intent.request.binding.expected != ExpectedSession::Version(prior_version)
        {
            prior_end = None;
        }
        if let Some(end) = entry.applied_deadline {
            let before = entry
                .previous_effective_end_at_ms
                .ok_or(JournalError::CorruptStore)?;
            if before < start
                || before > lifetime.max_deadline_ms
                || end < start
                || end > lifetime.max_deadline_ms
                || prior_end.is_some_and(|prior| prior != before)
            {
                return Err(JournalError::CorruptStore);
            }
            prior_end = Some(end);
            last_applied = Some(end);
        }
        prior_version = entry.intent.accepted_version;
    }
    if last_applied.is_some_and(|end| lifetime.deadline_ms != end) {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}

impl ControllerJournal {
    /// Recheck the exact applied renewal before acknowledging the center's
    /// saved result. A replay never refreshes time or changes a native deadline.
    /// # Errors
    /// Foreign/full-result conflicts, missing applications, unconfirmed earlier
    /// SDK work, invalid authority and unknown commits provide no success ACK.
    pub fn confirm_lifecycle_renewal(
        &mut self,
        result: &RenewalConfirmation,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<ConfirmedRenewal, JournalError> {
        if !result.is_sdk_renewal()
            || serde_json::to_vec(result)?.len() > RENEWAL_CONFIRMATION_BODY_BYTES
        {
            return Err(JournalError::Conflict);
        }
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        codec::validate(&tx, &tx.world())?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, &result.request, context)?;
        let (s, e) = inbox.find(&result.request)?.ok_or(JournalError::Conflict)?;
        let session = &inbox.sessions[s];
        let entry = &session.entries[e];
        if session.local.is_none()
            || entry.applied_deadline != Some(result.effective_end_at_ms)
            || entry.previous_effective_end_at_ms.is_none()
        {
            return Err(JournalError::Conflict);
        }
        let ack = ConfirmedRenewal {
            result: result.clone(),
        };
        if session.confirmed_renewal.as_ref() == Some(result) {
            return Ok(ack);
        }
        let previous = session.confirmed_renewal.clone();
        if previous
            .as_ref()
            .is_some_and(|old| old.request.accepted_version >= result.request.accepted_version)
            || session.entries.iter().any(|old| {
                is_sdk_renewal(&old.intent)
                    && old.intent.accepted_version < result.request.accepted_version
                    && previous.as_ref().is_none_or(|confirmed| {
                        old.intent.accepted_version > confirmed.request.accepted_version
                    })
            })
        {
            return Err(JournalError::Conflict);
        }
        let source_pinned = if let Some(previous) = previous.as_ref() {
            let pinned = source_work::references_intent(&tx, &previous.request)?;
            if pinned
                || session.entries.iter().any(|entry| {
                    entry.intent == previous.request && entry.local_operation.is_some()
                })
            {
                // Preserve the previous complete checkpoint until its exact
                // responsibility is settled. Advancing despite this pin would
                // forget which retained SDK row can later be reclaimed.
                return Err(JournalError::UnresolvedAction);
            }
            pinned
        } else {
            false
        };
        let session = &mut inbox.sessions[s];
        if let Some(previous) = previous {
            let current = session.current_version;
            let mut removed = session.compacted_sdk_through;
            session.entries.retain(|entry| {
                if discard_previous(entry, &previous, current, source_pinned) {
                    let version = entry.intent.accepted_version;
                    removed = Some(removed.map_or(version, |old| old.max(version)));
                    false
                } else {
                    true
                }
            });
            session.compacted_sdk_through = removed;
        }
        session.confirmed_renewal = Some(result.clone());
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(ack)
    }
}

fn checkpoint_entry_invalid(session: &Session, confirmed: &RenewalConfirmation) -> bool {
    for entry in &session.entries {
        if entry.intent == confirmed.request {
            return entry.applied_deadline != Some(confirmed.effective_end_at_ms)
                || entry.previous_effective_end_at_ms.is_none();
        }
    }
    true
}

fn contains_accepted_version(session: &Session, version: u64) -> bool {
    for entry in &session.entries {
        if entry.intent.accepted_version == version {
            return true;
        }
    }
    false
}

fn has_unapplied_sdk_through(session: &Session, version: u64) -> bool {
    for entry in &session.entries {
        if is_sdk_renewal(&entry.intent)
            && entry.intent.accepted_version <= version
            && entry.applied_deadline.is_none()
        {
            return true;
        }
    }
    false
}
