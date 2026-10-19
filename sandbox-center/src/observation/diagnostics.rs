//! Finite operator evidence from the original status round, never a health input.
use super::{HostStatus, NodeObservationPolicy, NodeRegistration};
use std::{
    fmt,
    io::Write,
    sync::atomic::{AtomicUsize, Ordering},
};

const RECORD_LIMIT: usize = 64;
static EMITTED: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    TransportUnavailable,
    ObservationRejected,
    ObservedAfterPriorExpiry,
}

struct Record {
    node_index: usize,
    outcome: Outcome,
    before_ms: u64,
    after_ms: u64,
    host_ms: Option<u64>,
    prior_healthy_until_ms: u64,
    window_ms: u64,
    skew_ms: u64,
}

impl Record {
    fn from_observation(
        node_index: usize,
        node: &NodeRegistration,
        status: Option<&HostStatus>,
        policy: NodeObservationPolicy,
        before_ms: u64,
        after_ms: u64,
        accepted: bool,
    ) -> Option<Self> {
        let outcome = if status.is_none() {
            Outcome::TransportUnavailable
        } else if !accepted {
            Outcome::ObservationRejected
        } else if node.healthy_until_ms != 0 && node.healthy_until_ms <= after_ms {
            Outcome::ObservedAfterPriorExpiry
        } else {
            return None;
        };
        Some(Self {
            node_index,
            outcome,
            before_ms,
            after_ms,
            host_ms: status.map(|value| value.observed_at_ms),
            prior_healthy_until_ms: node.healthy_until_ms,
            window_ms: policy.health_window_ms,
            skew_ms: policy.max_clock_skew_ms,
        })
    }
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Only enum labels and numbers: no identity strings, paths, keys,
        // peer errors, request payload or template metadata can reach stderr.
        write!(
            f,
            "center_observation node_index={} outcome={:?} before_ms={} after_ms={} host_ms={:?} prior_healthy_until_ms={} window_ms={} skew_ms={}",
            self.node_index,
            self.outcome,
            self.before_ms,
            self.after_ms,
            self.host_ms,
            self.prior_healthy_until_ms,
            self.window_ms,
            self.skew_ms
        )
    }
}

fn write_record(record: &Record, count: &AtomicUsize, output: &mut impl Write) {
    let Ok(previous) = count.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        (value < RECORD_LIMIT).then(|| value + 1)
    }) else {
        return;
    };
    // At most 64 attempts per process, including broken sinks. No retry,
    // retained queue or diagnostic panic changes the original observation.
    let _ = writeln!(output, "{record} remaining={}", RECORD_LIMIT - previous - 1);
}

pub(super) fn emit(
    index: usize,
    node: &NodeRegistration,
    status: Option<&HostStatus>,
    policy: NodeObservationPolicy,
    before: u64,
    after: u64,
    accepted: bool,
) {
    if let Some(record) =
        Record::from_observation(index, node, status, policy, before, after, accepted)
    {
        write_record(&record, &EMITTED, &mut std::io::stderr().lock());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_center_core::{NodeMode, NodeRole};
    use sandproof_protocol::internal::{INTERNAL_PROTOCOL_VERSION, lifecycle::Resources};

    fn node() -> NodeRegistration {
        NodeRegistration {
            identity: sandproof_protocol::internal::NodeIdentity {
                deployment_id: "private-deployment".try_into().unwrap(),
                node_id: "private-node".try_into().unwrap(),
                node_generation: 1.try_into().unwrap(),
            },
            retention_slots: 2,
            role: NodeRole::Runtime,
            limit: Resources::ZERO,
            mode: NodeMode::Accepting,
            healthy_until_ms: 1100,
            free: Resources::ZERO,
            templates: vec![],
            protocol_version: INTERNAL_PROTOCOL_VERSION,
        }
    }

    #[test]
    fn original_rejections_and_expired_prior_lease_remain_distinct() {
        let node = node();
        let policy = NodeObservationPolicy {
            health_window_ms: 1000,
            max_clock_skew_ms: 10,
        };
        let mut status = HostStatus {
            node: node.identity.clone(),
            retention_slots: 2,
            observed_at_ms: 150,
            free: Resources::ZERO,
        };
        assert!(
            Record::from_observation(0, &node, Some(&status), policy, 100, 200, true).is_none()
        );
        let unavailable =
            Record::from_observation(0, &node, None, policy, 100, 200, false).unwrap();
        assert_eq!(unavailable.outcome, Outcome::TransportUnavailable);
        assert!(unavailable.host_ms.is_none());
        for (before, after, host_ms) in [(100, 1100, 150), (100, 200, 300)] {
            status.observed_at_ms = host_ms;
            let accepted =
                super::super::accepted_observation(&node, &status, policy, before, after);
            assert!(accepted.is_none());
            let rejected = Record::from_observation(
                0,
                &node,
                Some(&status),
                policy,
                before,
                after,
                accepted.is_some(),
            )
            .unwrap();
            assert_eq!(rejected.outcome, Outcome::ObservationRejected);
            assert_eq!(
                (rejected.before_ms, rejected.after_ms, rejected.host_ms),
                (before, after, Some(host_ms))
            );
        }
        status.observed_at_ms = 1150;
        let accepted = super::super::accepted_observation(&node, &status, policy, 1100, 1200);
        assert_eq!(accepted, Some((2100, Resources::ZERO)));
        let recovered = Record::from_observation(
            0,
            &node,
            Some(&status),
            policy,
            1100,
            1200,
            accepted.is_some(),
        )
        .unwrap();
        assert_eq!(recovered.outcome, Outcome::ObservedAfterPriorExpiry);
        assert_eq!(recovered.prior_healthy_until_ms, 1100);
    }

    #[test]
    fn diagnostics_have_a_process_total_bound_and_never_retry_a_failed_sink() {
        let record = Record::from_observation(
            usize::MAX,
            &node(),
            None,
            NodeObservationPolicy {
                health_window_ms: u64::MAX,
                max_clock_skew_ms: u64::MAX,
            },
            u64::MAX,
            u64::MAX,
            false,
        )
        .unwrap();
        let count = AtomicUsize::new(0);
        let mut bytes = vec![];
        for _ in 0..1000 {
            write_record(&record, &count, &mut bytes);
        }
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text.lines().count(), RECORD_LIMIT);
        assert!(text.lines().all(|line| line.len() < 512));
        assert!(text.ends_with("remaining=0\n"));
        assert!(!text.contains("private-node") && !text.contains("private-deployment"));
        assert_eq!(count.load(Ordering::Relaxed), RECORD_LIMIT);

        let count = AtomicUsize::new(0);
        let mut broken = std::io::Cursor::new([0_u8; 0]);
        write_record(&record, &count, &mut broken);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        let mut recovered = vec![];
        write_record(&record, &count, &mut recovered);
        assert!(
            String::from_utf8(recovered)
                .unwrap()
                .ends_with("remaining=62\n")
        );
    }
}
