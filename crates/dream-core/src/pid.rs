//! Process identifier type.
//!
//! A [`Pid`] uniquely identifies a process within the DREAM runtime. It consists of
//! a node identifier (for future distribution support) and a unique process ID.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique process IDs.
static PID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Local node identifier (node 0 is always the local node).
const LOCAL_NODE: u32 = 0;

/// A process identifier.
///
/// Every process in DREAM has a unique `Pid` that can be used to send messages,
/// establish links, create monitors, and query process status.
///
/// # Examples
///
/// ```
/// use dream_core::Pid;
///
/// let pid = Pid::new();
/// println!("Process: {}", pid);
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
        }
    }

    /// Creates a `Pid` with specific node and id values.
    ///
    /// This is primarily used for deserialization and testing. In normal usage,
    /// prefer [`Pid::new()`].
    ///
    /// # Examples
    ///
    /// ```
    /// use dream_core::Pid;
    ///
    /// let pid = Pid::from_parts(1, 42);
    /// assert_eq!(pid.node(), 1);
    /// assert_eq!(pid.id(), 42);
    /// ```
    pub const fn from_parts(node: u32, id: u64) -> Self {
        Self { node, id }
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

    /// Returns `true` if this is a local process.
    #[inline]
    pub const fn is_local(&self) -> bool {
        self.node == LOCAL_NODE
    }
}

impl Default for Pid {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Pid<{}.{}>", self.node, self.id)
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}.{}>", self.node, self.id)
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
        let pid = Pid::from_parts(5, 100);
        assert_eq!(pid.node(), 5);
        assert_eq!(pid.id(), 100);
        assert!(!pid.is_local());
    }

    #[test]
    fn test_pid_display() {
        let pid = Pid::from_parts(0, 42);
        assert_eq!(format!("{}", pid), "<0.42>");
        assert_eq!(format!("{:?}", pid), "Pid<0.42>");
    }

    #[test]
    fn test_pid_serialization() {
        let pid = Pid::from_parts(1, 123);
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
}
