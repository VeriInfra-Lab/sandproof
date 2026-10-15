//! Demand-driven periodic keepalive schedule. It owns no task, timer or frame.
//! A completed Start/data send resets the period; keepalive sends do not. An
//! overdue tick is consumed once, then missed periods are skipped on the same
//! grid, like the pinned Go ticker. Send completion is supplied by the owner.
use std::time::Duration;
use tokio::time::Instant;

pub(crate) struct Clock {
    interval: Duration,
    next: Instant,
}
impl Clock {
    pub(crate) fn after(sent: Instant, interval: Duration) -> Option<Self> {
        if interval.is_zero() {
            return None;
        }
        Some(Self {
            interval,
            next: sent.checked_add(interval)?,
        })
    }
    pub(crate) const fn deadline(&self) -> Instant {
        self.next
    }

    /// Consume at most one due tick. Advancing early leaves the same deadline;
    /// late consumption preserves phase without an unbounded catch-up loop.
    pub(crate) fn consume(&mut self, now: Instant) -> Option<()> {
        if now < self.next {
            return Some(());
        }
        let remainder = now.duration_since(self.next).as_nanos() % self.interval.as_nanos();
        let remainder = Duration::new(
            u64::try_from(remainder / 1_000_000_000).ok()?,
            u32::try_from(remainder % 1_000_000_000).ok()?,
        );
        self.next = now.checked_add(self.interval.checked_sub(remainder)?)?;
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn delayed_keepalive_skips_missed_ticks_but_data_restarts_the_period() {
        let start = Instant::now();
        let second = Duration::from_secs(1);
        let mut clock = Clock::after(start, second * 10).unwrap();
        // Same virtual-time observations as the pinned Go 1.26.8 ticker:
        // sends blocked from 10 to 34; one overdue tick, next still at 40.
        for (read_at, next_at) in [(10, 20), (34, 40), (40, 50)] {
            tokio::time::advance(start + second * read_at - Instant::now()).await;
            assert!(clock.deadline() <= Instant::now());
            clock.consume(Instant::now()).unwrap();
            assert_eq!(clock.deadline(), start + second * next_at);
        }
        // Data release resets even when there was an already overdue tick.
        for (sent_at, next_at) in [(43, 53), (77, 87)] {
            let sent = start + second * sent_at;
            clock = Clock::after(sent, second * 10).unwrap();
            assert_eq!(clock.deadline(), start + second * next_at);
            clock.consume(sent).unwrap();
            assert_eq!(clock.deadline(), start + second * next_at);
        }
    }

    #[test]
    fn clock_handles_subsecond_periods_huge_skips_and_unrepresentable_deadlines() {
        let start = Instant::now();
        for interval in [
            Duration::from_nanos(1),
            Duration::from_nanos(7),
            Duration::from_millis(50),
            Duration::from_secs(90),
        ] {
            let mut clock = Clock::after(start, interval).unwrap();
            let late = start + Duration::from_secs(1_000_000_000) + Duration::from_nanos(13);
            clock.consume(late).unwrap();
            assert!(clock.deadline() > late);
            assert!(clock.deadline() - late <= interval);
            assert_eq!(
                clock.deadline().duration_since(start).as_nanos() % interval.as_nanos(),
                0
            );
            let deadline = clock.deadline();
            clock.consume(late).unwrap();
            assert_eq!(clock.deadline(), deadline);
        }
        assert!(Clock::after(start, Duration::ZERO).is_none());
        assert!(Clock::after(start, Duration::MAX).is_none());
    }
}
