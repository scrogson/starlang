//! Process identifier type.
//!
//! A [`Pid`] uniquely identifies a process within the DREAM runtime. It consists of
//! three components, matching Erlang's PID format `<node.id.creation>`:
//!
//! - **node**: Which node the process belongs to (0 = local node)
//! - **id**: The unique process identifier within that node
//! - **creation**: Distinguishes PIDs across node restarts
//!
//! The creation number prevents stale PIDs from accidentally matching new processes
//! after a node restart.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Global counter for generating unique process IDs.
static PID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Current node creation number. Incremented on each node restart.
static CREATION: AtomicU32 = AtomicU32::new(0);

/// Local node identifier (node 0 is always the local node).
const LOCAL_NODE: u32 = 0;

/// A process identifier.
///
/// Every process in DREAM has a unique `Pid` that can be used to send messages,
/// establish links, create monitors, and query process status.
///
/// The format matches Erlang's `<node.id.creation>` convention.
///
/// # Examples
///
/// ```
/// use dream_core::Pid;
///
/// let pid = Pid::new();
/// println!("Process: {}", pid);  // e.g., "<0.42.0>"
///
/// // Pids can be compared
/// let pid2 = Pid::new();
/// assert_ne!(pid, pid2);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid {
    /// Node identifier. 0 is the local node.
    node: u32,
    /// Unique process identifier within the node.
    id: u64,
    /// Creation number - distinguishes PIDs across node restarts.
    creation: u32,
}

impl Pid {
    /// Creates a new unique process identifier on the local node.
    ///
    /// Each call to `new()` returns a globally unique `Pid`.
    ///
    /// # Examples
    ///
    /// ```
    /// use dream_core::Pid;
    ///
    /// let pid1 = Pid::new();
    /// let pid2 = Pid::new();
    /// assert_ne!(pid1, pid2);
    /// ```
    pub fn new() -> Self {
        Self {
            node: LOCAL_NODE,
            id: PID_COUNTER.fetch_add(1, Ordering::Relaxed),
            creation: CREATION.load(Ordering::Relaxed),
        }
    }

    /// Creates a `Pid` with specific node, id, and creation values.
    ///
    /// This is primarily used for deserialization and testing. In normal usage,
    /// prefer [`Pid::new()`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dream_core::Pid;
    ///
    /// let pid = Pid::from_parts(1, 42, 0);
    /// assert_eq!(pid.node(), 1);
    /// assert_eq!(pid.id(), 42);
    /// assert_eq!(pid.creation(), 0);
    /// ```
    pub const fn from_parts(node: u32, id: u64, creation: u32) -> Self {
        Self { node, id, creation }
    }

    /// Creates a remote `Pid` for a process on another node.
    ///
    /// This is used when receiving PIDs from remote nodes during distribution.
    /// The node ID must be > 0 (0 is reserved for the local node).
    ///
    /// # Examples
    ///
    /// ```
    /// use dream_core::Pid;
    ///
    /// let remote_pid = Pid::remote(1, 100, 5);
    /// assert!(!remote_pid.is_local());
    /// assert_eq!(remote_pid.node(), 1);
    /// ```
    pub const fn remote(node: u32, id: u64, creation: u32) -> Self {
        // Note: We don't enforce node > 0 at compile time, but callers should
        // only use this for remote PIDs
        Self { node, id, creation }
    }

    /// Returns the node identifier.
    ///
    /// A value of 0 indicates the local node.
    #[inline]
    pub const fn node(&self) -> u32 {
        self.node
    }

    /// Returns the process identifier within the node.
    #[inline]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Returns the creation number.
    ///
    /// The creation number distinguishes PIDs across node restarts.
    #[inline]
    pub const fn creation(&self) -> u32 {
        self.creation
    }

    /// Returns `true` if this is a local process.
    #[inline]
    pub const fn is_local(&self) -> bool {
        self.node == LOCAL_NODE
    }
}

/// Increment the creation counter.
///
/// This should be called when the node restarts to invalidate old PIDs.
/// Returns the new creation value.
pub fn increment_creation() -> u32 {
    CREATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Get the current creation value.
pub fn current_creation() -> u32 {
    CREATION.load(Ordering::Relaxed)
}

impl Default for Pid {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pid<{}.{}.{}>", self.node, self.id, self.creation)
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}.{}.{}>", self.node, self.id, self.creation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_uniqueness() {
        let pid1 = Pid::new();
        let pid2 = Pid::new();
        assert_ne!(pid1, pid2);
    }

    #[test]
    fn test_pid_local() {
        let pid = Pid::new();
        assert!(pid.is_local());
        assert_eq!(pid.node(), 0);
    }

    #[test]
    fn test_pid_from_parts() {
        let pid = Pid::from_parts(5, 100, 2);
        assert_eq!(pid.node(), 5);
        assert_eq!(pid.id(), 100);
        assert_eq!(pid.creation(), 2);
        assert!(!pid.is_local());
    }

    #[test]
    fn test_pid_display() {
        let pid = Pid::from_parts(0, 42, 0);
        assert_eq!(format!("{}", pid), "<0.42.0>");
        assert_eq!(format!("{:?}", pid), "Pid<0.42.0>");
    }

    #[test]
    fn test_pid_serialization() {
        let pid = Pid::from_parts(1, 123, 5);
        let bytes = postcard::to_allocvec(&pid).unwrap();
        let decoded: Pid = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(pid, decoded);
    }

    #[test]
    fn test_pid_hash() {
        use std::collections::HashSet;

        let mut set = HashSet::new();
        let pid1 = Pid::new();
        let pid2 = Pid::new();

        set.insert(pid1);
        set.insert(pid2);
        set.insert(pid1); // duplicate

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_creation_distinguishes_pids() {
        // Same node and id but different creation should be different PIDs
        let pid1 = Pid::from_parts(0, 42, 0);
        let pid2 = Pid::from_parts(0, 42, 1);
        assert_ne!(pid1, pid2);
    }

    #[test]
    fn test_increment_creation() {
        let before = current_creation();
        let new_creation = increment_creation();
        assert_eq!(new_creation, before + 1);
        assert_eq!(current_creation(), new_creation);
    }
}
