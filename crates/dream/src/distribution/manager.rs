//! Distribution connection manager.
//!
//! Manages connections to remote nodes and routes messages.

use super::monitor::NodeMonitorRegistry;
use super::protocol::{DistError, DistMessage};
use super::transport::{QuicConnection, QuicTransport};
use super::DIST_MANAGER;
use dashmap::DashMap;
use dream_core::{NodeId, NodeInfo, NodeName, Pid};
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Information about a connected node.
struct ConnectedNode {
    /// The node's info.
    info: NodeInfo,
    /// The QUIC connection.
    connection: QuicConnection,
    /// Sender for outgoing messages.
    tx: mpsc::Sender<DistMessage>,
}

/// The distribution manager.
///
/// Handles all node connections and message routing.
pub struct DistributionManager {
    /// Our node name.
    node_name: String,
    /// Our creation number.
    creation: u32,
    /// Counter for assigning node IDs.
    next_node_id: AtomicU32,
    /// Connected nodes by NodeId.
    nodes: DashMap<u32, ConnectedNode>,
    /// Node ID lookup by address.
    addr_to_node: DashMap<SocketAddr, u32>,
    /// The QUIC transport (if listening).
    transport: RwLock<Option<Arc<QuicTransport>>>,
    /// Node monitor registry.
    monitors: NodeMonitorRegistry,
}

impl DistributionManager {
    /// Create a new distribution manager.
    pub fn new(node_name: String, creation: u32) -> Self {
        Self {
            node_name,
            creation,
            next_node_id: AtomicU32::new(1), // Start at 1, 0 is local
            nodes: DashMap::new(),
            addr_to_node: DashMap::new(),
            transport: RwLock::new(None),
            monitors: NodeMonitorRegistry::new(),
        }
    }

    /// Start listening for incoming connections.
    pub async fn start_listener(
        &self,
        addr: SocketAddr,
        cert_path: Option<impl AsRef<Path>>,
        key_path: Option<impl AsRef<Path>>,
    ) -> Result<(), DistError> {
        let transport = QuicTransport::bind(
            addr,
            self.node_name.clone(),
            self.creation,
            cert_path.as_ref().map(|p| p.as_ref()),
            key_path.as_ref().map(|p| p.as_ref()),
        )
        .await?;

        let transport = Arc::new(transport);
        *self.transport.write() = Some(transport.clone());

        // Spawn accept loop
        let node_name = self.node_name.clone();
        let creation = self.creation;
        tokio::spawn(async move {
            accept_loop(transport, node_name, creation).await;
        });

        Ok(())
    }

    /// Connect to a remote node.
    pub async fn connect_to(&self, addr: SocketAddr) -> Result<NodeId, DistError> {
        // Check if already connected
        if let Some(node_id) = self.addr_to_node.get(&addr) {
            return Err(DistError::AlreadyConnected(NodeId::new(*node_id)));
        }

        // Create a client transport if we don't have one
        let transport = {
            let guard = self.transport.read();
            if let Some(t) = guard.as_ref() {
                t.clone()
            } else {
                drop(guard);
                let t = Arc::new(QuicTransport::client(
                    self.node_name.clone(),
                    self.creation,
                )?);
                *self.transport.write() = Some(t.clone());
                t
            }
        };

        // Connect
        let connection = transport.connect(addr, "localhost").await?;

        // Perform handshake
        let (node_id, node_info) = self.perform_handshake(&connection, addr).await?;

        // Create message sender
        let (tx, rx) = mpsc::channel(1024);

        // Store connection
        self.nodes.insert(
            node_id.as_u32(),
            ConnectedNode {
                info: node_info,
                connection,
                tx,
            },
        );
        self.addr_to_node.insert(addr, node_id.as_u32());

        // Spawn message sender task
        let node_id_u32 = node_id.as_u32();
        tokio::spawn(async move {
            message_sender_loop(rx, node_id_u32).await;
        });

        // Request global registry sync from the new node
        super::global::global_registry().request_sync(node_id);

        tracing::info!(%addr, ?node_id, "Connected to remote node");
        Ok(node_id)
    }

    /// Perform the handshake with a remote node.
    async fn perform_handshake(
        &self,
        connection: &QuicConnection,
        addr: SocketAddr,
    ) -> Result<(NodeId, NodeInfo), DistError> {
        // Send Hello
        let hello = DistMessage::Hello {
            node_name: self.node_name.clone(),
            creation: self.creation,
        };
        connection.send_message(&hello).await?;

        // Wait for Welcome
        let (_send, mut recv) = connection.accept_stream().await?;
        let welcome = QuicConnection::recv_message(&mut recv).await?;

        match welcome {
            DistMessage::Welcome {
                node_name,
                creation,
                assigned_node_id,
            } => {
                let node_id = NodeId::new(assigned_node_id);
                let info = NodeInfo::new(
                    NodeName::new(node_name),
                    node_id,
                    Some(addr),
                    creation,
                );
                Ok((node_id, info))
            }
            _ => Err(DistError::Handshake("expected Welcome message".to_string())),
        }
    }

    /// Disconnect from a node.
    pub fn disconnect_from(&self, node_id: NodeId) -> Result<(), DistError> {
        if let Some((_, node)) = self.nodes.remove(&node_id.as_u32()) {
            node.connection.close("disconnect requested");
            if let Some(addr) = node.info.addr {
                self.addr_to_node.remove(&addr);
            }

            // Notify monitors
            self.monitors.notify_node_down(node_id, "disconnect requested".to_string());

            tracing::info!(?node_id, "Disconnected from node");
            Ok(())
        } else {
            Err(DistError::NotConnected(node_id))
        }
    }

    /// Send a message to a remote process.
    pub fn send_to_remote(&self, pid: Pid, payload: Vec<u8>) -> Result<(), DistError> {
        let node_id = pid.node();

        if let Some(node) = self.nodes.get(&node_id) {
            let msg = DistMessage::Send {
                to: pid,
                from: dream_runtime::try_current_pid(),
                payload,
            };

            // Non-blocking send
            if node.tx.try_send(msg).is_err() {
                tracing::warn!(?pid, "Message queue full for remote node");
            }
            Ok(())
        } else {
            Err(DistError::NotConnected(NodeId::new(node_id)))
        }
    }

    /// Get list of connected nodes.
    pub fn connected_nodes(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|r| NodeId::new(*r.key())).collect()
    }

    /// Get info about a connected node.
    pub fn get_node_info(&self, node_id: NodeId) -> Option<NodeInfo> {
        self.nodes.get(&node_id.as_u32()).map(|n| n.info.clone())
    }

    /// Allocate a new node ID for an incoming connection.
    pub fn allocate_node_id(&self) -> NodeId {
        NodeId::new(self.next_node_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Get the monitor registry.
    pub fn monitors(&self) -> &NodeMonitorRegistry {
        &self.monitors
    }

    /// Get a node's message sender.
    pub(crate) fn get_node_tx(&self, node_id: u32) -> Option<mpsc::Sender<DistMessage>> {
        self.nodes.get(&node_id).map(|n| n.tx.clone())
    }
}

/// Accept loop for incoming connections.
async fn accept_loop(transport: Arc<QuicTransport>, node_name: String, creation: u32) {
    loop {
        if let Some(connection) = transport.accept().await {
            let node_name = node_name.clone();
            let creation = creation;

            tokio::spawn(async move {
                if let Err(e) = handle_incoming_connection(connection, node_name, creation).await {
                    tracing::error!(error = %e, "Failed to handle incoming connection");
                }
            });
        }
    }
}

/// Handle an incoming connection.
async fn handle_incoming_connection(
    connection: QuicConnection,
    our_node_name: String,
    our_creation: u32,
) -> Result<(), DistError> {
    // Wait for Hello
    let (_send, mut recv) = connection.accept_stream().await?;
    let hello = QuicConnection::recv_message(&mut recv).await?;

    let (remote_name, remote_creation) = match hello {
        DistMessage::Hello { node_name, creation } => (node_name, creation),
        _ => return Err(DistError::Handshake("expected Hello message".to_string())),
    };

    // Get the manager and allocate a node ID
    let manager = DIST_MANAGER.get().ok_or(DistError::NotInitialized)?;
    let assigned_id = manager.allocate_node_id();

    // Send Welcome
    let welcome = DistMessage::Welcome {
        node_name: our_node_name,
        creation: our_creation,
        assigned_node_id: assigned_id.as_u32(),
    };
    connection.send_message(&welcome).await?;

    // Store the connection
    let addr = connection.remote_address();
    let (tx, rx) = mpsc::channel(1024);

    let info = NodeInfo::new(
        NodeName::new(&remote_name),
        assigned_id,
        Some(addr),
        remote_creation,
    );

    manager.nodes.insert(
        assigned_id.as_u32(),
        ConnectedNode {
            info,
            connection,
            tx,
        },
    );
    manager.addr_to_node.insert(addr, assigned_id.as_u32());

    // Spawn message handling tasks
    let node_id = assigned_id.as_u32();
    tokio::spawn(async move {
        message_sender_loop(rx, node_id).await;
    });

    // Spawn receiver loop
    tokio::spawn(async move {
        message_receiver_loop(node_id).await;
    });

    tracing::info!(
        remote_name = %remote_name,
        ?assigned_id,
        "Accepted incoming connection"
    );

    Ok(())
}

/// Loop to send messages to a remote node.
async fn message_sender_loop(mut rx: mpsc::Receiver<DistMessage>, node_id: u32) {
    while let Some(msg) = rx.recv().await {
        let manager = match DIST_MANAGER.get() {
            Some(m) => m,
            None => break,
        };

        if let Some(node) = manager.nodes.get(&node_id) {
            if let Err(e) = node.connection.send_message(&msg).await {
                tracing::error!(error = %e, node_id, "Failed to send message");
                break;
            }
        } else {
            break;
        }
    }

    // Connection closed or error - clean up
    if let Some(manager) = DIST_MANAGER.get() {
        if let Some((_, node)) = manager.nodes.remove(&node_id) {
            if let Some(addr) = node.info.addr {
                manager.addr_to_node.remove(&addr);
            }
            manager.monitors.notify_node_down(
                NodeId::new(node_id),
                "connection closed".to_string(),
            );
        }
    }
}

/// Loop to receive messages from a remote node.
async fn message_receiver_loop(node_id: u32) {
    loop {
        let manager = match DIST_MANAGER.get() {
            Some(m) => m,
            None => break,
        };

        let connection = match manager.nodes.get(&node_id) {
            Some(node) => {
                // Accept a stream
                match node.connection.accept_stream().await {
                    Ok((_, recv)) => recv,
                    Err(_) => break,
                }
            }
            None => break,
        };

        // We need to drop the borrow before calling recv_message
        let mut recv = connection;
        match QuicConnection::recv_message(&mut recv).await {
            Ok(msg) => {
                handle_incoming_message(node_id, msg).await;
            }
            Err(DistError::ConnectionClosed) => break,
            Err(e) => {
                tracing::error!(error = %e, node_id, "Error receiving message");
                break;
            }
        }
    }

    // Clean up
    if let Some(manager) = DIST_MANAGER.get() {
        if let Some((_, node)) = manager.nodes.remove(&node_id) {
            if let Some(addr) = node.info.addr {
                manager.addr_to_node.remove(&addr);
            }
            manager.monitors.notify_node_down(
                NodeId::new(node_id),
                "connection closed".to_string(),
            );
        }
    }
}

/// Handle an incoming message from a remote node.
async fn handle_incoming_message(from_node: u32, msg: DistMessage) {
    match msg {
        DistMessage::Send { to, from: _, payload } => {
            // Deliver to local process
            if to.is_local() {
                if let Some(handle) = dream_process::global::try_handle() {
                    let _ = handle.registry().send_raw(to, payload);
                }
            } else {
                tracing::warn!(?to, "Received message for non-local PID");
            }
        }
        DistMessage::Ping { seq } => {
            // Respond with pong
            if let Some(manager) = DIST_MANAGER.get() {
                if let Some(node) = manager.nodes.get(&from_node) {
                    let _ = node.tx.try_send(DistMessage::Pong { seq });
                }
            }
        }
        DistMessage::Pong { seq } => {
            tracing::trace!(seq, from_node, "Received pong");
        }
        DistMessage::MonitorNode { requesting_pid } => {
            if let Some(manager) = DIST_MANAGER.get() {
                manager.monitors.add_remote_monitor(NodeId::new(from_node), requesting_pid);
            }
        }
        DistMessage::DemonitorNode { requesting_pid } => {
            if let Some(manager) = DIST_MANAGER.get() {
                manager.monitors.remove_remote_monitor(NodeId::new(from_node), requesting_pid);
            }
        }
        DistMessage::NodeGoingDown { reason } => {
            tracing::info!(from_node, %reason, "Remote node going down");
            // The connection will close and trigger cleanup
        }
        DistMessage::GlobalRegistry { payload } => {
            // Handle global registry message
            if let Ok(msg) = postcard::from_bytes::<super::global::GlobalRegistryMessage>(&payload) {
                super::global::global_registry().handle_message(msg, NodeId::new(from_node));
            }
        }
        _ => {
            tracing::warn!(?msg, "Unexpected message type");
        }
    }
}

// === Public API Functions ===

/// Connect to a remote node.
///
/// Returns the `NodeId` assigned to the remote node.
///
/// # Example
///
/// ```ignore
/// let node_id = dream::dist::connect("192.168.1.100:9000").await?;
/// ```
pub async fn connect(addr: &str) -> Result<NodeId, DistError> {
    let manager = DIST_MANAGER.get().ok_or(DistError::NotInitialized)?;
    let socket_addr: SocketAddr = addr
        .parse()
        .map_err(|e| DistError::InvalidAddress(format!("{}: {}", addr, e)))?;
    manager.connect_to(socket_addr).await
}

/// Disconnect from a node.
pub fn disconnect(node_id: NodeId) -> Result<(), DistError> {
    let manager = DIST_MANAGER.get().ok_or(DistError::NotInitialized)?;
    manager.disconnect_from(node_id)
}

/// Get list of connected nodes.
pub fn nodes() -> Vec<NodeId> {
    DIST_MANAGER
        .get()
        .map(|m| m.connected_nodes())
        .unwrap_or_default()
}

/// Get info about a connected node.
pub fn node_info(node_id: NodeId) -> Option<NodeInfo> {
    DIST_MANAGER.get().and_then(|m| m.get_node_info(node_id))
}

/// Send a message to a remote process.
///
/// This is called by the process registry when sending to a non-local PID.
pub(crate) fn send_remote(pid: Pid, payload: Vec<u8>) -> Result<(), DistError> {
    let manager = DIST_MANAGER.get().ok_or(DistError::NotInitialized)?;
    manager.send_to_remote(pid, payload)
}
