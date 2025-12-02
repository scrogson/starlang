//! Global process registry for distributed DREAM.
//!
//! Allows processes to be registered by name across all connected nodes.
//! Similar to Erlang's `:global` module.
//!
//! # How it works
//!
//! - When a process is registered globally, it's announced to all connected nodes
//! - Each node maintains a local cache of global registrations
//! - Lookups check the local cache first
//! - The owning node is authoritative for a name

use super::protocol::DistMessage;
use super::DIST_MANAGER;
use dashmap::DashMap;
use dream_core::{NodeId, Pid};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Global registry instance.
static GLOBAL_REGISTRY: OnceLock<GlobalRegistry> = OnceLock::new();

/// Message types for global registry synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GlobalRegistryMessage {
    /// Register a name globally.
    Register {
        /// The name to register.
        name: String,
        /// The PID to register under the name.
        pid: Pid,
    },
    /// Unregister a name.
    Unregister {
        /// The name to unregister.
        name: String,
    },
    /// Request the full registry (sent on connect).
    SyncRequest,
    /// Full registry sync response.
    SyncResponse {
        /// The entries in the registry.
        entries: Vec<(String, Pid)>,
    },
}

/// The global process registry.
pub struct GlobalRegistry {
    /// Name -> PID mapping.
    names: DashMap<String, Pid>,
}

impl GlobalRegistry {
    /// Create a new global registry.
    pub fn new() -> Self {
        Self {
            names: DashMap::new(),
        }
    }

    /// Register a process globally.
    ///
    /// Returns `false` if the name is already registered.
    pub fn register(&self, name: String, pid: Pid) -> bool {
        // Check if already registered
        if self.names.contains_key(&name) {
            return false;
        }

        self.names.insert(name.clone(), pid);

        // Broadcast to all connected nodes
        self.broadcast_register(&name, pid);

        true
    }

    /// Unregister a global name.
    pub fn unregister(&self, name: &str) -> Option<Pid> {
        let result = self.names.remove(name).map(|(_, pid)| pid);

        if result.is_some() {
            // Broadcast unregister to all nodes
            self.broadcast_unregister(name);
        }

        result
    }

    /// Look up a process by global name.
    pub fn whereis(&self, name: &str) -> Option<Pid> {
        self.names.get(name).map(|r| *r.value())
    }

    /// Get all registered names.
    pub fn registered(&self) -> Vec<String> {
        self.names.iter().map(|r| r.key().clone()).collect()
    }

    /// Handle an incoming global registry message from another node.
    pub fn handle_message(&self, msg: GlobalRegistryMessage, from_node: NodeId) {
        match msg {
            GlobalRegistryMessage::Register { name, pid } => {
                // Only accept if not already registered locally with a different PID
                self.names.entry(name).or_insert(pid);
            }
            GlobalRegistryMessage::Unregister { name } => {
                self.names.remove(&name);
            }
            GlobalRegistryMessage::SyncRequest => {
                // Send our registrations to the requesting node
                let entries: Vec<(String, Pid)> = self
                    .names
                    .iter()
                    .map(|r| (r.key().clone(), *r.value()))
                    .collect();

                if let Some(manager) = DIST_MANAGER.get() {
                    if let Some(tx) = manager.get_node_tx(from_node.as_u32()) {
                        let msg = GlobalRegistryMessage::SyncResponse { entries };
                        if let Ok(payload) = postcard::to_allocvec(&msg) {
                            let _ = tx.try_send(DistMessage::GlobalRegistry { payload });
                        }
                    }
                }
            }
            GlobalRegistryMessage::SyncResponse { entries } => {
                // Merge remote registrations into our cache
                for (name, pid) in entries {
                    self.names.entry(name).or_insert(pid);
                }
            }
        }
    }

    /// Broadcast a registration to all connected nodes.
    fn broadcast_register(&self, name: &str, pid: Pid) {
        let msg = GlobalRegistryMessage::Register {
            name: name.to_string(),
            pid,
        };
        self.broadcast_global_message(&msg);
    }

    /// Broadcast an unregistration to all connected nodes.
    fn broadcast_unregister(&self, name: &str) {
        let msg = GlobalRegistryMessage::Unregister {
            name: name.to_string(),
        };
        self.broadcast_global_message(&msg);
    }

    /// Broadcast a global registry message to all connected nodes.
    fn broadcast_global_message(&self, msg: &GlobalRegistryMessage) {
        if let Some(manager) = DIST_MANAGER.get() {
            if let Ok(payload) = postcard::to_allocvec(msg) {
                let dist_msg = DistMessage::GlobalRegistry { payload };
                for node_id in manager.connected_nodes() {
                    if let Some(tx) = manager.get_node_tx(node_id.as_u32()) {
                        let _ = tx.try_send(dist_msg.clone());
                    }
                }
            }
        }
    }

    /// Request sync from a newly connected node.
    pub fn request_sync(&self, node_id: NodeId) {
        if let Some(manager) = DIST_MANAGER.get() {
            if let Some(tx) = manager.get_node_tx(node_id.as_u32()) {
                let msg = GlobalRegistryMessage::SyncRequest;
                if let Ok(payload) = postcard::to_allocvec(&msg) {
                    let _ = tx.try_send(DistMessage::GlobalRegistry { payload });
                }
            }
        }
    }
}

impl Default for GlobalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Get or initialize the global registry.
pub fn global_registry() -> &'static GlobalRegistry {
    GLOBAL_REGISTRY.get_or_init(GlobalRegistry::new)
}

// === Public API ===

/// Register a process globally by name.
///
/// The process will be visible to all connected nodes.
/// Returns `false` if the name is already registered.
///
/// # Example
///
/// ```ignore
/// let pid = dream::spawn(|| async { /* ... */ });
/// dream::dist::global::register("my_service", pid);
/// ```
pub fn register(name: impl Into<String>, pid: Pid) -> bool {
    global_registry().register(name.into(), pid)
}

/// Unregister a global name.
pub fn unregister(name: &str) -> Option<Pid> {
    global_registry().unregister(name)
}

/// Look up a process by global name.
///
/// Returns the PID if found, regardless of which node owns it.
///
/// # Example
///
/// ```ignore
/// if let Some(pid) = dream::dist::global::whereis("my_service") {
///     dream::send_raw(pid, message);  // Works even if remote!
/// }
/// ```
pub fn whereis(name: &str) -> Option<Pid> {
    global_registry().whereis(name)
}

/// Get all globally registered names.
pub fn registered() -> Vec<String> {
    global_registry().registered()
}
