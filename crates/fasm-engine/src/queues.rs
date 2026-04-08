//! In-memory message queues.
//!
//! Three tiers:
//! - **Execution-private**: created per invocation, destroyed when execution ends.
//! - **Function-scoped**: named queue bound to a specific FASM endpoint (prefix `fn:`).
//! - **Shared**: named global queue; any FASM function can publish or consume.
//!
//! Shared queues provide at-least-once delivery: a message stays `pending` until
//! the handler calls `MQ_ACK`, or times out and is re-queued (up to `max_retries`).

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;
use serde_json::Value as JsonValue;

// ── Message ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Message {
    pub id:      String,
    pub payload: JsonValue,
    pub retries: u32,
}

impl Message {
    pub fn new(payload: JsonValue) -> Self {
        Self { id: Uuid::new_v4().to_string(), payload, retries: 0 }
    }
}

// ── Pending ack entry ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct PendingAck {
    msg:        Message,
    deadline:   Instant,
}

// ── Shared queue internals ────────────────────────────────────────────────────

#[derive(Debug)]
struct SharedQueueInner {
    ready:       VecDeque<Message>,
    in_flight:   HashMap<String, PendingAck>,
    max_retries: u32,
    timeout:     Duration,
    /// notify_rx can be used by the looper to wake up when new messages arrive.
    notify:      tokio::sync::Notify,
}

impl SharedQueueInner {
    fn new(max_retries: u32, timeout_secs: u64) -> Self {
        Self {
            ready:       VecDeque::new(),
            in_flight:   HashMap::new(),
            max_retries,
            timeout:     Duration::from_secs(timeout_secs),
            notify:      tokio::sync::Notify::new(),
        }
    }

    fn enqueue(&mut self, msg: Message) {
        self.ready.push_back(msg);
        self.notify.notify_one();
    }

    fn try_dequeue(&mut self) -> Option<Message> {
        let msg = self.ready.pop_front()?;
        let id = msg.id.clone();
        self.in_flight.insert(id, PendingAck {
            msg: msg.clone(),
            deadline: Instant::now() + self.timeout,
        });
        Some(msg)
    }

    fn ack(&mut self, id: &str) -> bool {
        self.in_flight.remove(id).is_some()
    }

    fn nack(&mut self, id: &str) -> bool {
        if let Some(pending) = self.in_flight.remove(id) {
            let mut msg = pending.msg;
            msg.retries += 1;
            if msg.retries <= self.max_retries {
                self.ready.push_back(msg);
                self.notify.notify_one();
            }
            true
        } else {
            false
        }
    }

    /// Scan for timed-out in-flight messages and re-queue or drop them.
    fn requeue_expired(&mut self) {
        let now = Instant::now();
        let expired: Vec<String> = self.in_flight
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(pending) = self.in_flight.remove(&id) {
                let mut msg = pending.msg;
                msg.retries += 1;
                if msg.retries <= self.max_retries {
                    self.ready.push_back(msg);
                    self.notify.notify_one();
                }
            }
        }
    }

    fn depth(&self) -> usize {
        self.ready.len() + self.in_flight.len()
    }
}

// ── SharedQueue public handle ─────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct SharedQueue(Arc<Mutex<SharedQueueInner>>);

impl SharedQueue {
    pub fn new(max_retries: u32, timeout_secs: u64) -> Self {
        Self(Arc::new(Mutex::new(SharedQueueInner::new(max_retries, timeout_secs))))
    }

    pub fn send(&self, payload: JsonValue) {
        self.0.lock().unwrap().enqueue(Message::new(payload));
    }

    pub fn try_dequeue(&self) -> Option<Message> {
        self.0.lock().unwrap().try_dequeue()
    }

    pub fn ack(&self, id: &str) -> bool {
        self.0.lock().unwrap().ack(id)
    }

    pub fn nack(&self, id: &str) -> bool {
        self.0.lock().unwrap().nack(id)
    }

    pub fn requeue_expired(&self) {
        self.0.lock().unwrap().requeue_expired();
    }

    pub fn depth(&self) -> usize {
        self.0.lock().unwrap().depth()
    }

    /// Wait (async) until a message is available, then dequeue it.
    pub async fn recv_async(&self) -> Message {
        loop {
            {
                let mut g = self.0.lock().unwrap();
                if let Some(msg) = g.try_dequeue() {
                    return msg;
                }
                // Drop the lock before awaiting the notification
                // (we need a reference to the Notify without the lock held).
                // We use a temporary clone of the Arc to notify.
            }
            // Wait for a notification from another sender.
            // Because we've dropped the lock, this is safe.
            // We recreate the notification wait each time.
            tokio::task::yield_now().await;
        }
    }
}

// ── Execution-private queue ────────────────────────────────────────────────────

#[derive(Debug)]
struct PrivateQueueInner(VecDeque<JsonValue>);

#[derive(Clone, Debug)]
pub struct PrivateQueue(Arc<Mutex<PrivateQueueInner>>);

impl PrivateQueue {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(PrivateQueueInner(VecDeque::new()))))
    }

    pub fn send(&self, payload: JsonValue) {
        self.0.lock().unwrap().0.push_back(payload);
    }

    pub fn try_recv(&self) -> Option<JsonValue> {
        self.0.lock().unwrap().0.pop_front()
    }
}

impl Default for PrivateQueue {
    fn default() -> Self { Self::new() }
}

// ── QueueRegistry ─────────────────────────────────────────────────────────────

/// Central registry of named shared queues.
#[derive(Clone, Debug, Default)]
pub struct QueueRegistry(Arc<Mutex<HashMap<String, SharedQueue>>>);

impl QueueRegistry {
    pub fn new() -> Self { Self::default() }

    /// Get or create a named shared queue.
    pub fn get_or_create(&self, name: &str, max_retries: u32, timeout_secs: u64) -> SharedQueue {
        let mut g = self.0.lock().unwrap();
        g.entry(name.to_string())
         .or_insert_with(|| SharedQueue::new(max_retries, timeout_secs))
         .clone()
    }

    /// Look up an existing named queue (returns None if not registered).
    pub fn get(&self, name: &str) -> Option<SharedQueue> {
        self.0.lock().unwrap().get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enqueue_and_dequeue() {
        let q = SharedQueue::new(3, 30);
        q.send(serde_json::json!({"item": 1}));
        q.send(serde_json::json!({"item": 2}));
        let m = q.try_dequeue().expect("should have a message");
        assert_eq!(m.payload["item"], 1);
        assert_eq!(q.depth(), 2); // 1 ready + 1 in-flight
    }

    #[test]
    fn test_ack_removes_in_flight() {
        let q = SharedQueue::new(3, 30);
        q.send(serde_json::json!(42));
        let msg = q.try_dequeue().unwrap();
        assert!(q.ack(&msg.id), "ack should succeed");
        assert_eq!(q.depth(), 0);
    }

    #[test]
    fn test_nack_requeues_message() {
        let q = SharedQueue::new(3, 30);
        q.send(serde_json::json!("hello"));
        let msg = q.try_dequeue().unwrap();
        let id = msg.id.clone();
        assert!(q.nack(&id), "nack should succeed");
        // Message should be back in ready queue
        let requeued = q.try_dequeue().expect("message should be requeued after nack");
        assert_eq!(requeued.retries, 1);
    }

    #[test]
    fn test_message_dropped_after_max_retries() {
        let q = SharedQueue::new(2, 30); // max 2 retries
        q.send(serde_json::json!("test"));
        let m = q.try_dequeue().unwrap();
        q.nack(&m.id); // retry 1
        let m = q.try_dequeue().unwrap();
        q.nack(&m.id); // retry 2
        let m = q.try_dequeue().unwrap();
        q.nack(&m.id); // retry 3 — exceeds max, should be dropped
        assert_eq!(q.depth(), 0, "message should be dropped after max_retries");
    }

    #[test]
    fn test_requeue_expired_sends_timed_out_messages_back() {
        let q = SharedQueue::new(3, 0); // 0 second timeout → immediately expired
        q.send(serde_json::json!("urgent"));
        let msg = q.try_dequeue().unwrap();
        // Don't ack — let it expire
        // With a 0s timeout, requeue_expired should put it back immediately
        q.requeue_expired();
        assert!(q.try_dequeue().is_some(), "expired message should be requeued");
    }

    #[test]
    fn test_queue_registry_get_or_create() {
        let registry = QueueRegistry::new();
        let q1 = registry.get_or_create("orders", 3, 30);
        let q2 = registry.get_or_create("orders", 3, 30);
        // Both should refer to the same underlying queue
        q1.send(serde_json::json!(1));
        assert_eq!(q2.depth(), 1, "both handles should share the same queue");
    }

    #[test]
    fn test_private_queue_send_recv() {
        let q = PrivateQueue::new();
        q.send(serde_json::json!("msg1"));
        q.send(serde_json::json!("msg2"));
        assert_eq!(q.try_recv(), Some(serde_json::json!("msg1")));
        assert_eq!(q.try_recv(), Some(serde_json::json!("msg2")));
        assert_eq!(q.try_recv(), None);
    }
}
