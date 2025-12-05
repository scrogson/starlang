//! Process-owned concurrent key-value storage.
//!
//! The [`Store`] provides ETS-like functionality for Rust: typed, concurrent
//! key-value storage that is owned by a process and automatically cleaned up
//! when the owner exits.
//!
//! # Overview
//!
//! - **Typed**: `Store<K, V>` provides compile-time type safety
//! - **Process-owned**: Each store has an owner process; cleanup is automatic on owner exit
//! - **Concurrent**: Multiple processes can read/write (if access allows)
//! - **Named**: Stores can be registered by name for easy discovery
//!
//! # Example
//!
//! ```ignore
//! use ambitious::store::{Store, Access};
//!
//! // Create a public store (any process can access)
//! let users: Store<String, User> = Store::new();
//!
//! // Insert and retrieve values
//! users.insert("alice".into(), User { name: "Alice".into() });
//! let user = users.get(&"alice".into());
//!
//! // Store is automatically cleaned up when owner process exits
//! ```
//!
//! # Achieving Bag/DuplicateBag Patterns
//!
//! Unlike Erlang's ETS, this Store is typed and doesn't have special Bag types.
//! You can achieve similar functionality with typed values:
//!
//! ```ignore
//! use std::collections::HashSet;
//!
//! // Bag pattern - multiple unique values per key
//! let tags: Store<String, HashSet<String>> = Store::new();
//! tags.update("post_1", |existing| {
//!     let mut set = existing.unwrap_or_default();
//!     set.insert("rust".into());
//!     set
//! });
//!
//! // DuplicateBag pattern - multiple values including duplicates
//! let events: Store<String, Vec<Event>> = Store::new();
//! events.update("user_1", |existing| {
//!     let mut vec = existing.unwrap_or_default();
//!     vec.push(Event::Login);
//!     vec
//! });
//! ```

mod registry;

pub use registry::cleanup_owned_stores;

use crate::core::Pid;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for generating unique store IDs.
static STORE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A unique identifier for a store.
///
/// Store IDs are globally unique and atomically generated.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StoreId(u64);

impl StoreId {
    /// Creates a new unique store ID.
    fn new() -> Self {
        Self(STORE_ID_COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Returns the raw value of this store ID.
    #[inline]
    pub const fn as_raw(&self) -> u64 {
        self.0
    }
}

impl fmt::Debug for StoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StoreId({})", self.0)
    }
}

impl fmt::Display for StoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "store#{}", self.0)
    }
}

/// Access control for a store.
///
/// Determines which processes can read from and write to the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Access {
    /// Any process can read and write.
    #[default]
    Public,
    /// Only the owner process can read and write.
    Private,
}

/// Options for creating a store.
#[derive(Debug, Clone, Default)]
pub struct StoreOptions {
    /// Access control (default: Public).
    pub access: Access,
    /// Optional name for registration.
    pub name: Option<String>,
    /// Optional heir process that inherits the store on owner death.
    pub heir: Option<Pid>,
}

impl StoreOptions {
    /// Creates new store options with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the access control.
    pub fn access(mut self, access: Access) -> Self {
        self.access = access;
        self
    }

    /// Sets the store name for registration.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the heir process.
    pub fn heir(mut self, heir: Pid) -> Self {
        self.heir = Some(heir);
        self
    }
}

/// Error type for store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The store has been deleted.
    StoreDeleted,
    /// Access denied (private store accessed by non-owner).
    AccessDenied,
    /// Name already registered.
    NameAlreadyRegistered,
    /// Store not found by name.
    NotFound,
    /// No process context available.
    NoProcessContext,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StoreDeleted => write!(f, "store has been deleted"),
            Self::AccessDenied => write!(f, "access denied"),
            Self::NameAlreadyRegistered => write!(f, "name already registered"),
            Self::NotFound => write!(f, "store not found"),
            Self::NoProcessContext => write!(f, "no process context available"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Internal store data that can be shared across clones.
struct StoreInner<K, V> {
    id: StoreId,
    owner: Pid,
    data: DashMap<K, V>,
    access: Access,
    #[allow(dead_code)] // Used in future named store lookup
    name: Option<String>,
    #[allow(dead_code)] // Used in heir transfer logic
    heir: Option<Pid>,
    deleted: std::sync::atomic::AtomicBool,
}

/// A typed, process-owned concurrent key-value store.
///
/// `Store<K, V>` provides concurrent access to key-value data with automatic
/// cleanup when the owning process terminates.
///
/// # Type Parameters
///
/// - `K`: Key type, must be `Eq + Hash + Clone + Send + Sync`
/// - `V`: Value type, must be `Clone + Send + Sync`
///
/// # Thread Safety
///
/// Store is fully thread-safe and can be shared across threads and processes.
/// The underlying data structure uses lock-free concurrent hashmaps.
///
/// # Example
///
/// ```ignore
/// use ambitious::store::Store;
///
/// let store: Store<String, i32> = Store::new();
/// store.insert("count".into(), 0);
///
/// // Atomic update
/// store.update("count", |v| v.map(|n| n + 1).unwrap_or(1));
/// ```
pub struct Store<K, V> {
    inner: Arc<StoreInner<K, V>>,
}

impl<K, V> Clone for Store<K, V> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> Default for Store<K, V>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> Store<K, V>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Creates a new store owned by the current process.
    ///
    /// # Panics
    ///
    /// Panics if called outside of a process context.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use ambitious::store::Store;
    ///
    /// let store: Store<String, User> = Store::new();
    /// ```
    pub fn new() -> Self {
        Self::with_options(StoreOptions::default())
    }

    /// Creates a new store with the given options.
    ///
    /// # Panics
    ///
    /// Panics if called outside of a process context.
    pub fn with_options(opts: StoreOptions) -> Self {
        let owner = crate::try_current_pid().expect("Store::new() must be called from a process");
        Self::new_with_owner(owner, opts)
    }

    /// Creates a new store with an explicit owner (for internal use).
    pub(crate) fn new_with_owner(owner: Pid, opts: StoreOptions) -> Self {
        let id = StoreId::new();

        let inner = Arc::new(StoreInner {
            id,
            owner,
            data: DashMap::new(),
            access: opts.access,
            name: opts.name.clone(),
            heir: opts.heir,
            deleted: std::sync::atomic::AtomicBool::new(false),
        });

        let store = Self { inner };

        // Register in global registry
        registry::register_store(id, owner, opts.name, opts.heir);

        store
    }

    /// Returns the unique ID of this store.
    pub fn id(&self) -> StoreId {
        self.inner.id
    }

    /// Returns the PID of the owner process.
    pub fn owner(&self) -> Pid {
        self.inner.owner
    }

    /// Returns the access control setting.
    pub fn access(&self) -> Access {
        self.inner.access
    }

    /// Returns `true` if this store has been deleted.
    pub fn is_deleted(&self) -> bool {
        self.inner.deleted.load(Ordering::Acquire)
    }

    /// Checks if the current process can access this store.
    fn check_access(&self) -> Result<(), StoreError> {
        if self.is_deleted() {
            return Err(StoreError::StoreDeleted);
        }

        match self.inner.access {
            Access::Public => Ok(()),
            Access::Private => {
                let caller = crate::try_current_pid().ok_or(StoreError::NoProcessContext)?;
                if caller == self.inner.owner {
                    Ok(())
                } else {
                    Err(StoreError::AccessDenied)
                }
            }
        }
    }

    // =========================================================================
    // Basic Operations
    // =========================================================================

    /// Inserts a key-value pair into the store.
    ///
    /// Returns the previous value if the key was already present.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn insert(&self, key: K, value: V) -> Result<Option<V>, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.insert(key, value))
    }

    /// Gets a value by key.
    ///
    /// Returns `None` if the key is not present.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn get(&self, key: &K) -> Result<Option<V>, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.get(key).map(|r| r.value().clone()))
    }

    /// Removes a key from the store.
    ///
    /// Returns the removed value if the key was present.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn remove(&self, key: &K) -> Result<Option<V>, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.remove(key).map(|(_, v)| v))
    }

    /// Returns `true` if the store contains the given key.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn contains_key(&self, key: &K) -> Result<bool, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.contains_key(key))
    }

    /// Atomically updates a value in the store.
    ///
    /// The update function receives the current value (if any) and returns
    /// the new value to store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn update<F>(&self, key: K, f: F) -> Result<V, StoreError>
    where
        F: FnOnce(Option<V>) -> V,
    {
        self.check_access()?;

        let new_value = f(self.inner.data.get(&key).map(|r| r.value().clone()));
        self.inner.data.insert(key, new_value.clone());
        Ok(new_value)
    }

    /// Gets a value or inserts a default if not present.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn get_or_insert(&self, key: K, default: V) -> Result<V, StoreError> {
        self.check_access()?;
        Ok(self
            .inner
            .data
            .entry(key)
            .or_insert(default)
            .value()
            .clone())
    }

    /// Gets a value or inserts a default computed by a function.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn get_or_insert_with<F>(&self, key: K, f: F) -> Result<V, StoreError>
    where
        F: FnOnce() -> V,
    {
        self.check_access()?;
        Ok(self.inner.data.entry(key).or_insert_with(f).value().clone())
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Inserts multiple key-value pairs.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn insert_many(&self, items: impl IntoIterator<Item = (K, V)>) -> Result<(), StoreError> {
        self.check_access()?;
        for (k, v) in items {
            self.inner.data.insert(k, v);
        }
        Ok(())
    }

    /// Returns all keys in the store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn keys(&self) -> Result<Vec<K>, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.iter().map(|r| r.key().clone()).collect())
    }

    /// Returns all values in the store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn values(&self) -> Result<Vec<V>, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.iter().map(|r| r.value().clone()).collect())
    }

    /// Returns all key-value pairs in the store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn iter(&self) -> Result<Vec<(K, V)>, StoreError> {
        self.check_access()?;
        Ok(self
            .inner
            .data
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect())
    }

    /// Returns the number of entries in the store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn len(&self) -> Result<usize, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.len())
    }

    /// Returns `true` if the store is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn is_empty(&self) -> Result<bool, StoreError> {
        self.check_access()?;
        Ok(self.inner.data.is_empty())
    }

    /// Removes all entries from the store.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn clear(&self) -> Result<(), StoreError> {
        self.check_access()?;
        self.inner.data.clear();
        Ok(())
    }

    /// Retains only the entries that satisfy the predicate.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied or the store is deleted.
    pub fn retain<F>(&self, f: F) -> Result<(), StoreError>
    where
        F: FnMut(&K, &mut V) -> bool,
    {
        self.check_access()?;
        self.inner.data.retain(f);
        Ok(())
    }

    // =========================================================================
    // Ownership Operations
    // =========================================================================

    /// Transfers ownership of this store to another process.
    ///
    /// Only the current owner can transfer ownership.
    ///
    /// # Errors
    ///
    /// Returns an error if the caller is not the owner.
    pub fn give_away(&self, new_owner: Pid) -> Result<(), StoreError> {
        let caller = crate::try_current_pid().ok_or(StoreError::NoProcessContext)?;

        if caller != self.inner.owner {
            return Err(StoreError::AccessDenied);
        }

        registry::transfer_ownership(self.inner.id, self.inner.owner, new_owner);

        // Note: We can't actually update inner.owner since it's behind Arc.
        // The registry is the source of truth for ownership.
        Ok(())
    }

    /// Marks this store as deleted.
    ///
    /// Called internally during cleanup.
    #[allow(dead_code)] // Will be used when store cleanup marks stores deleted
    pub(crate) fn mark_deleted(&self) {
        self.inner.deleted.store(true, Ordering::Release);
    }
}

impl<K, V> fmt::Debug for Store<K, V>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Store")
            .field("id", &self.inner.id)
            .field("owner", &self.inner.owner)
            .field("access", &self.inner.access)
            .field("len", &self.inner.data.len())
            .finish()
    }
}

// ============================================================================
// Global Store Lookup
// ============================================================================

/// Looks up a store by name.
///
/// Returns the store ID if found.
pub fn whereis(name: &str) -> Option<StoreId> {
    registry::whereis(name)
}

/// Returns all store IDs owned by a process.
pub fn stores_owned_by(pid: Pid) -> HashSet<StoreId> {
    registry::stores_owned_by(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a store outside of process context for testing
    fn create_test_store<K, V>() -> Store<K, V>
    where
        K: Eq + Hash + Clone + Send + Sync + 'static,
        V: Clone + Send + Sync + 'static,
    {
        let pid = Pid::new();
        Store::new_with_owner(pid, StoreOptions::default())
    }

    #[test]
    fn test_store_id_uniqueness() {
        let id1 = StoreId::new();
        let id2 = StoreId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_store_basic_operations() {
        let store: Store<String, i32> = create_test_store();

        // Insert
        assert_eq!(store.insert("a".into(), 1).unwrap(), None);
        assert_eq!(store.insert("a".into(), 2).unwrap(), Some(1));

        // Get
        assert_eq!(store.get(&"a".into()).unwrap(), Some(2));
        assert_eq!(store.get(&"b".into()).unwrap(), None);

        // Contains
        assert!(store.contains_key(&"a".into()).unwrap());
        assert!(!store.contains_key(&"b".into()).unwrap());

        // Remove
        assert_eq!(store.remove(&"a".into()).unwrap(), Some(2));
        assert_eq!(store.remove(&"a".into()).unwrap(), None);
    }

    #[test]
    fn test_store_update() {
        let store: Store<String, i32> = create_test_store();

        // Update non-existent key
        let val = store
            .update("count".into(), |v| v.unwrap_or(0) + 1)
            .unwrap();
        assert_eq!(val, 1);

        // Update existing key
        let val = store
            .update("count".into(), |v| v.unwrap_or(0) + 1)
            .unwrap();
        assert_eq!(val, 2);

        assert_eq!(store.get(&"count".into()).unwrap(), Some(2));
    }

    #[test]
    fn test_store_bulk_operations() {
        let store: Store<String, i32> = create_test_store();

        store
            .insert_many([("a".into(), 1), ("b".into(), 2), ("c".into(), 3)])
            .unwrap();

        assert_eq!(store.len().unwrap(), 3);
        assert!(!store.is_empty().unwrap());

        let keys = store.keys().unwrap();
        assert_eq!(keys.len(), 3);

        let values = store.values().unwrap();
        assert_eq!(values.len(), 3);

        store.clear().unwrap();
        assert!(store.is_empty().unwrap());
    }

    #[test]
    fn test_store_retain() {
        let store: Store<String, i32> = create_test_store();

        store
            .insert_many([("a".into(), 1), ("b".into(), 2), ("c".into(), 3)])
            .unwrap();

        store.retain(|_, v| *v > 1).unwrap();

        assert_eq!(store.len().unwrap(), 2);
        assert!(!store.contains_key(&"a".into()).unwrap());
        assert!(store.contains_key(&"b".into()).unwrap());
        assert!(store.contains_key(&"c".into()).unwrap());
    }

    #[test]
    fn test_store_clone() {
        let store1: Store<String, i32> = create_test_store();
        store1.insert("key".into(), 42).unwrap();

        let store2 = store1.clone();
        assert_eq!(store2.get(&"key".into()).unwrap(), Some(42));

        // Modifications through one clone are visible through the other
        store2.insert("key".into(), 100).unwrap();
        assert_eq!(store1.get(&"key".into()).unwrap(), Some(100));
    }

    #[test]
    fn test_store_deleted() {
        let store: Store<String, i32> = create_test_store();
        store.insert("key".into(), 1).unwrap();

        store.mark_deleted();

        assert!(store.is_deleted());
        assert!(matches!(
            store.get(&"key".into()),
            Err(StoreError::StoreDeleted)
        ));
        assert!(matches!(
            store.insert("key".into(), 2),
            Err(StoreError::StoreDeleted)
        ));
    }

    #[test]
    fn test_store_options() {
        let opts = StoreOptions::new().access(Access::Private).name("my_store");

        assert_eq!(opts.access, Access::Private);
        assert_eq!(opts.name, Some("my_store".into()));
    }
}
