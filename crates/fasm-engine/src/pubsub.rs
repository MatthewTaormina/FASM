//! Pub/sub — topics with fan-out delivery via execution-private queues.
//!
//! ## Lifecycle
//! 1. An execution calls `PS_CREATE_TOPIC` (or topics are declared in config).
//! 2. A long-running execution calls `PS_SUBSCRIBE` — its private queue is
//!    registered as a subscriber.
//! 3. Any execution calls `PS_PUBLISH` — the message is fan-out copied to all
//!    subscriber queues.
//! 4. When an execution ends its private queue is dropped automatically.

use crate::queues::PrivateQueue;
use serde_json::Value as JsonValue;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

// ── Topic ─────────────────────────────────────────────────────────────────────

/// A weak reference to a subscriber's private queue plus an execution id.
struct Subscriber {
    execution_id: String,
    queue: PrivateQueue,
}

#[derive(Default)]
struct TopicInner {
    subscribers: Vec<Subscriber>,
}

impl TopicInner {
    fn publish(&self, payload: &JsonValue) {
        for sub in &self.subscribers {
            sub.queue.send(payload.clone());
        }
    }

    fn subscribe(&mut self, execution_id: String, q: PrivateQueue) {
        // Remove any stale entry for this execution first.
        self.subscribers.retain(|s| s.execution_id != execution_id);
        self.subscribers.push(Subscriber {
            execution_id,
            queue: q,
        });
    }

    fn unsubscribe(&mut self, execution_id: &str) {
        self.subscribers.retain(|s| s.execution_id != execution_id);
    }

    fn subscriber_count(&self) -> usize {
        self.subscribers.len()
    }
}

// ── PubSubRegistry ────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct PubSubRegistry(Arc<Mutex<HashMap<String, TopicInner>>>);

impl PubSubRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a topic if it doesn't already exist.
    pub fn create_topic(&self, topic: &str) {
        let mut g = self.0.lock().unwrap();
        g.entry(topic.to_string()).or_default();
    }

    /// Publish `payload` to all subscribers of `topic`.
    ///
    /// Returns the number of subscribers the message was delivered to.
    pub fn publish(&self, topic: &str, payload: &JsonValue) -> usize {
        let g = self.0.lock().unwrap();
        if let Some(t) = g.get(topic) {
            t.publish(payload);
            t.subscriber_count()
        } else {
            0
        }
    }

    /// Subscribe an execution's private queue to a topic.
    ///
    /// The topic is auto-created if it does not yet exist.
    pub fn subscribe(&self, topic: &str, execution_id: String, q: PrivateQueue) {
        let mut g = self.0.lock().unwrap();
        g.entry(topic.to_string())
            .or_default()
            .subscribe(execution_id, q);
    }

    /// Remove an execution's subscription from all topics (called on execution end).
    pub fn unsubscribe_all(&self, execution_id: &str) {
        let mut g = self.0.lock().unwrap();
        for topic in g.values_mut() {
            topic.unsubscribe(execution_id);
        }
    }

    pub fn topics(&self) -> Vec<String> {
        self.0.lock().unwrap().keys().cloned().collect()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_publish_to_one_subscriber() {
        let registry = PubSubRegistry::new();
        registry.create_topic("events");

        let q = PrivateQueue::new();
        registry.subscribe("events", "exec-1".into(), q.clone());

        let count = registry.publish("events", &serde_json::json!({"msg": "hello"}));
        assert_eq!(count, 1);
        assert!(
            q.try_recv().is_some(),
            "subscriber queue should have a message"
        );
    }

    #[test]
    fn test_fan_out_to_multiple_subscribers() {
        let registry = PubSubRegistry::new();
        registry.create_topic("broadcast");

        let q1 = PrivateQueue::new();
        let q2 = PrivateQueue::new();
        let q3 = PrivateQueue::new();
        registry.subscribe("broadcast", "e1".into(), q1.clone());
        registry.subscribe("broadcast", "e2".into(), q2.clone());
        registry.subscribe("broadcast", "e3".into(), q3.clone());

        let count = registry.publish("broadcast", &serde_json::json!(42));
        assert_eq!(count, 3);
        assert!(q1.try_recv().is_some());
        assert!(q2.try_recv().is_some());
        assert!(q3.try_recv().is_some());
    }

    #[test]
    fn test_unsubscribe_removes_from_all_topics() {
        let registry = PubSubRegistry::new();
        registry.create_topic("topic-a");
        registry.create_topic("topic-b");

        let q = PrivateQueue::new();
        registry.subscribe("topic-a", "e1".into(), q.clone());
        registry.subscribe("topic-b", "e1".into(), q.clone());

        registry.unsubscribe_all("e1");

        assert_eq!(registry.publish("topic-a", &serde_json::json!(1)), 0);
        assert_eq!(registry.publish("topic-b", &serde_json::json!(2)), 0);
    }

    #[test]
    fn test_publish_to_nonexistent_topic_returns_zero() {
        let registry = PubSubRegistry::new();
        let count = registry.publish("ghost", &serde_json::json!(null));
        assert_eq!(count, 0, "publishing to unknown topic should return 0");
    }
}
