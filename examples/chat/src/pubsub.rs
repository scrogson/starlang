//! PubSub implementation using pg (process groups).
//!
//! A simple publish-subscribe system similar to Phoenix.PubSub.
//! Under the hood, it uses `pg` for distributed process groups,
//! making subscriptions automatically work across nodes.
//!
//! # Example
//!
//! ```ignore
//! use crate::pubsub::PubSub;
//!
//! // Subscribe the current process to a topic
//! PubSub::subscribe("room:lobby");
//!
//! // Or subscribe a specific PID
//! PubSub::subscribe_pid("room:lobby", some_pid);
//!
//! // Broadcast to all subscribers
//! PubSub::broadcast("room:lobby", &MyMessage { ... });
//!
//! // Unsubscribe
//! PubSub::unsubscribe("room:lobby");
//! ```

use dream::dist::pg;
use dream_core::Pid;
use serde::Serialize;

/// PubSub client API.
///
/// This is a stateless API that uses `pg` under the hood.
/// No GenServer process is needed - subscriptions are handled
/// directly by the distributed process groups.
pub struct PubSub;

impl PubSub {
    /// Subscribe the current process to a topic.
    pub fn subscribe(topic: &str) {
        Self::subscribe_pid(topic, dream::current_pid());
    }

    /// Subscribe a specific PID to a topic.
    pub fn subscribe_pid(topic: &str, pid: Pid) {
        pg::join(Self::topic_to_group(topic), pid);
    }

    /// Unsubscribe the current process from a topic.
    pub fn unsubscribe(topic: &str) {
        Self::unsubscribe_pid(topic, dream::current_pid());
    }

    /// Unsubscribe a specific PID from a topic.
    pub fn unsubscribe_pid(topic: &str, pid: Pid) {
        pg::leave(&Self::topic_to_group(topic), pid);
    }

    /// Unsubscribe a PID from all topics.
    ///
    /// Note: This removes the PID from all pg groups,
    /// including non-pubsub groups.
    pub fn unsubscribe_all(pid: Pid) {
        pg::leave_all(pid);
    }

    /// Broadcast a serializable message to all subscribers of a topic.
    ///
    /// This sends to all subscribers across all nodes.
    pub fn broadcast<M: Serialize>(topic: &str, message: &M) {
        Self::do_broadcast(topic, message, None);
    }

    /// Broadcast a serializable message to all subscribers except the specified PID.
    ///
    /// This is useful when the sender is also subscribed to the topic
    /// but shouldn't receive their own message.
    pub fn broadcast_from<M: Serialize>(topic: &str, message: &M, exclude: Pid) {
        Self::do_broadcast(topic, message, Some(exclude));
    }

    /// Internal: broadcast to all subscribers.
    fn do_broadcast<M: Serialize>(topic: &str, message: &M, exclude: Option<Pid>) {
        let group = Self::topic_to_group(topic);
        let members = pg::get_members(&group);

        if let Ok(payload) = postcard::to_allocvec(message) {
            for pid in members {
                if Some(pid) != exclude {
                    let _ = dream::send_raw(pid, payload.clone());
                }
            }
        }
    }

    /// Get the list of subscribers for a topic (local + remote).
    pub fn subscribers(topic: &str) -> Vec<Pid> {
        pg::get_members(&Self::topic_to_group(topic))
    }

    /// Get only local subscribers for a topic.
    pub fn local_subscribers(topic: &str) -> Vec<Pid> {
        pg::get_local_members(&Self::topic_to_group(topic))
    }

    /// Get all topics (actually all pg groups with pubsub prefix).
    pub fn topics() -> Vec<String> {
        pg::all_groups()
            .into_iter()
            .filter_map(|g| g.strip_prefix("pubsub:").map(String::from))
            .collect()
    }

    /// Convert a topic name to a pg group name.
    /// We prefix with "pubsub:" to namespace our groups.
    fn topic_to_group(topic: &str) -> String {
        format!("pubsub:{}", topic)
    }
}
