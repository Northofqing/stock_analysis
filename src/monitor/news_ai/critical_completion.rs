//! Bounded ownership handoff; it neither mints scores nor owns a delivery gate.
use super::AuditedCriticalNews;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::mpsc::{self, OwnedPermit};

const CAPACITY: usize = 5;
struct State {
    closed: bool,
    // Transfer of the same five queue/permit slots after receiver closure.
    // Kept until the last producer/receiver/slot owner ends; never replayed.
    retained: Vec<AuditedCriticalNews>,
}
fn state_lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|poison| poison.into_inner())
}
#[derive(Clone)]
pub struct CriticalCompletionSender {
    sender: mpsc::Sender<AuditedCriticalNews>,
    state: Arc<Mutex<State>>,
}
pub struct CriticalCompletionReceiver {
    receiver: mpsc::Receiver<AuditedCriticalNews>,
    state: Arc<Mutex<State>>,
}
pub struct CriticalCompletionSlot {
    permit: OwnedPermit<AuditedCriticalNews>,
    state: Arc<Mutex<State>>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct CriticalCompletionClosed;
#[derive(Debug, PartialEq, Eq)]
pub enum CriticalCompletionSubmitted { Queued, RetainedReceiverClosed }
pub enum CriticalCompletionWait { Score(AuditedCriticalNews), Deadline, Closed }

/// Fixed capacity; opaque slots must be acquired before the existing model call.
pub fn critical_news_completion_channel() -> (CriticalCompletionSender, CriticalCompletionReceiver) {
    let (sender, receiver) = mpsc::channel(CAPACITY);
    let state = Arc::new(Mutex::new(State { closed: false, retained: Vec::with_capacity(CAPACITY) }));
    (CriticalCompletionSender { sender, state: state.clone() }, CriticalCompletionReceiver { receiver, state })
}
impl CriticalCompletionSender {
    pub async fn reserve(&self) -> Result<CriticalCompletionSlot, CriticalCompletionClosed> {
        let permit = self.sender.clone().reserve_owned().await.map_err(|_| CriticalCompletionClosed)?;
        if state_lock(&self.state).closed {
            drop(permit);
            return Err(CriticalCompletionClosed);
        }
        Ok(CriticalCompletionSlot { permit, state: self.state.clone() })
    }
    pub fn retained_count(&self) -> usize { state_lock(&self.state).retained.len() }
}
impl CriticalCompletionSlot {
    /// No fallible send after acquiring an actual score. Close and submit share
    /// one short lock: a score is queued or moves into the retained owner.
    pub fn submit(self, score: AuditedCriticalNews) -> CriticalCompletionSubmitted {
        let mut state = state_lock(&self.state);
        if state.closed {
            state.retained.push(score);
            drop(self.permit);
            CriticalCompletionSubmitted::RetainedReceiverClosed
        } else {
            // Tokio allows a permit to send even after close; the shared lock
            // excludes that race and keeps close's drain complete.
            drop(self.permit.send(score));
            CriticalCompletionSubmitted::Queued
        }
    }
}
impl CriticalCompletionReceiver {
    /// A tick boundary wins a ready-score race. The score stays owned by the
    /// queue for the bounded drain after the next original N02 phase.
    pub async fn receive_until(&mut self, deadline: tokio::time::Instant) -> CriticalCompletionWait {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => CriticalCompletionWait::Deadline,
            score = self.receiver.recv() => match score {
                Some(score) => CriticalCompletionWait::Score(score),
                None => CriticalCompletionWait::Closed,
            },
        }
    }
    pub fn try_receive(&mut self) -> Option<AuditedCriticalNews> { self.receiver.try_recv().ok() }
    pub fn close(&mut self) {
        let mut state = state_lock(&self.state);
        state.closed = true;
        self.receiver.close();
        while let Ok(score) = self.receiver.try_recv() { state.retained.push(score); }
    }
}
impl Drop for CriticalCompletionReceiver {
    fn drop(&mut self) { self.close(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::task::Poll;
    fn score(item: &str) -> AuditedCriticalNews {
        let mut conn = crate::database::news_ai::tests::connection();
        crate::database::news_ai::tests::critical_fixture(&mut conn, item)
    }
    #[tokio::test(start_paused = true)]
    async fn news_n01_completion_full_queue_deadline_and_cross_tick_keep_owned_scores() {
        let (tx, mut rx) = critical_news_completion_channel();
        let mut expected = Vec::new();
        for i in 0..CAPACITY {
            let value = score(&format!("TEST_CODE_FULL_{i}"));
            expected.push(value.evidence_sha256().to_owned());
            assert_eq!(tx.reserve().await.unwrap().submit(value), CriticalCompletionSubmitted::Queued);
        }
        let mut cancelled = Box::pin(tx.reserve());
        std::future::poll_fn(|cx| {
            assert!(cancelled.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        }).await;
        drop(cancelled); // cancellation cannot make room or start a sixth call
        let mut waiting = Box::pin(tx.reserve());
        std::future::poll_fn(|cx| {
            assert!(waiting.as_mut().poll(cx).is_pending()); // no sixth model slot
            Poll::Ready(())
        }).await;
        assert!(matches!(rx.receive_until(tokio::time::Instant::now()).await, CriticalCompletionWait::Deadline));
        // A ready deadline did not consume or destroy the queued score.
        let first = rx.try_receive().unwrap();
        assert_eq!(first.evidence_sha256(), expected[0]);
        let slot = waiting.await.unwrap();
        let sixth = score("TEST_CODE_NEXT_TICK");
        expected.push(sixth.evidence_sha256().into());
        assert_eq!(slot.submit(sixth), CriticalCompletionSubmitted::Queued);
        let mut seen = vec![first.evidence_sha256().to_owned()];
        // One bounded drain does not wait for an unfinished worker/model.
        for _ in 0..CAPACITY { seen.push(rx.try_receive().unwrap().evidence_sha256().into()); }
        assert_eq!(seen, expected);
        assert!(rx.try_receive().is_none());
        assert_eq!(tx.retained_count(), 0);
    }
    #[tokio::test(start_paused = true)]
    async fn news_n01_completion_nonrunning_wait_and_closed_receiver_retain_real_tokens() {
        let (tx, mut rx) = critical_news_completion_channel();
        let cancelled_slot = tx.reserve().await.unwrap();
        drop(cancelled_slot); // provider/validation failure before score releases its slot
        let reserved = tx.reserve().await.unwrap();
        let value = score("TEST_CODE_NONRUNNING_WAKE");
        let digest = value.evidence_sha256().to_owned();
        reserved.submit(value);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
        let CriticalCompletionWait::Score(value) = rx.receive_until(deadline).await else { panic!("real nonrunning wait wake"); };
        assert_eq!(value.evidence_sha256(), digest);
        drop(value); // explicit consumer refusal completes this handoff, no remint
        let slot = tx.reserve().await.unwrap();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let joining = tokio::task::spawn_blocking(move || {
            let actual = score("TEST_CODE_AUDIT_JOIN_CANCELLED");
            let digest = actual.evidence_sha256().to_owned();
            slot.submit(actual); // real mint and handoff remain inside this owner
            done_tx.send(digest).unwrap();
        });
        drop(joining); // dropping the await handle does not abort the blocking owner
        let audited_digest = done_rx.await.unwrap();
        let handed = rx.try_receive().unwrap();
        assert_eq!(handed.evidence_sha256(), audited_digest);
        drop(handed);
        let mut in_flight = Vec::new();
        for i in 0..CAPACITY {
            let slot = tx.reserve().await.unwrap();
            if i < 2 { slot.submit(score(&format!("TEST_CODE_QUEUED_CLOSE_{i}"))); }
            else { in_flight.push((slot, score(&format!("TEST_CODE_RESERVED_CLOSE_{i}")))); }
        }
        rx.close();
        assert_eq!(tx.retained_count(), 2);
        for (slot, value) in in_flight {
            assert_eq!(slot.submit(value), CriticalCompletionSubmitted::RetainedReceiverClosed);
        }
        assert_eq!(tx.retained_count(), CAPACITY);
        assert!(tx.reserve().await.is_err()); // closed endpoint cannot start another call
        let held = state_lock(&tx.state);
        let distinct = held.retained.iter().map(|v|v.evidence_sha256()).collect::<std::collections::HashSet<_>>();
        assert_eq!(distinct.len(), CAPACITY);
        drop(held);
        drop(rx);
        assert_eq!(tx.retained_count(), CAPACITY); // producer still owns all actual scores
    }
}
