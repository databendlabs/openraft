//! RaftRouter for managing multiple Raft groups.
//!
//! This module provides [`RaftRouter`], the core component for Multi-Raft deployments
//! that manages multiple Raft groups on a single node.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::RwLock;

use super::MultiRaftError;
use super::MultiRaftTypeConfig;
use crate::Raft;

/// A router for managing multiple Raft groups on a single node.
///
/// `RaftRouter` is the central component in a Multi-Raft deployment.
/// It provides:
///
/// - **Registration**: Add and remove Raft instances
/// - **Lookup**: Find Raft instances by group ID and node ID
/// - **Group management**: Get all nodes in a group
/// - **Lifecycle**: Shutdown all managed Raft instances
///
/// ## Thread Safety
///
/// `RaftRouter` is thread-safe and can be shared across multiple threads using `Arc`.
/// All operations use internal locking for concurrent access.
///
/// ## Example
///
/// ```ignore
/// use std::sync::Arc;
/// use openraft::multi_raft::RaftRouter;
///
/// // Create a router
/// let router = Arc::new(RaftRouter::<MyConfig>::new());
///
/// // Register Raft instances
/// router.register("group_1".to_string(), raft1)?;
/// router.register("group_1".to_string(), raft2)?;
/// router.register("group_2".to_string(), raft3)?;
///
/// // Look up a specific node
/// if let Some(raft) = router.get(&"group_1".to_string(), &node_id) {
///     raft.client_write(request).await?;
/// }
///
/// // Get all nodes in a group
/// let group_nodes = router.get_group(&"group_1".to_string());
///
/// // Shutdown all
/// router.shutdown_all().await?;
/// ```
///
/// ## Design Notes
///
/// `RaftRouter` is a regular instance that can be created multiple times if needed.
/// This provides more flexibility in testing and allows for isolated router instances
/// in different parts of an application.
pub struct RaftRouter<C>
where C: MultiRaftTypeConfig
{
    /// Maps (GroupId, NodeId) -> Raft instance
    /// Using tuple key for efficient lookup by both group and node
    nodes: RwLock<HashMap<(C::GroupId, C::NodeId), Raft<C>>>,

    /// Maps GroupId -> list of NodeIds in that group
    /// Enables efficient "get all nodes in group" queries
    groups: RwLock<BTreeMap<C::GroupId, Vec<C::NodeId>>>,
}

impl<C> Default for RaftRouter<C>
where C: MultiRaftTypeConfig
{
    fn default() -> Self {
        Self::new()
    }
}

impl<C> RaftRouter<C>
where C: MultiRaftTypeConfig
{
    /// Creates a new empty `RaftRouter`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let router = RaftRouter::<MyConfig>::new();
    /// ```
    pub fn new() -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            groups: RwLock::new(BTreeMap::new()),
        }
    }

    /// Registers a Raft instance with the router.
    ///
    /// The Raft instance will be associated with the given `group_id` and its own `node_id`.
    ///
    /// # Errors
    ///
    /// Returns [`MultiRaftError::AlreadyRegistered`] if a node with the same
    /// (group_id, node_id) pair is already registered.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let raft = Raft::new(node_id, config, network, storage, sm).await?;
    /// router.register("my_group".to_string(), raft)?;
    /// ```
    pub fn register(&self, group_id: C::GroupId, raft: Raft<C>) -> Result<(), MultiRaftError<C::GroupId, C::NodeId>> {
        let node_id = raft.node_id().clone();
        let key = (group_id.clone(), node_id.clone());

        // First, check and insert into nodes map
        {
            let mut nodes = self.nodes.write().unwrap();
            if nodes.contains_key(&key) {
                return Err(MultiRaftError::AlreadyRegistered { group_id, node_id });
            }
            nodes.insert(key, raft);
        }

        // Then update groups index
        {
            let mut groups = self.groups.write().unwrap();
            groups.entry(group_id).or_default().push(node_id);
        }

        Ok(())
    }

    /// Unregisters and returns a Raft instance from the router.
    ///
    /// # Returns
    ///
    /// - `Some(Raft)` if the node was found and removed
    /// - `None` if no node with the given (group_id, node_id) was registered
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(raft) = router.unregister(&"my_group".to_string(), &node_id) {
    ///     raft.shutdown().await?;
    /// }
    /// ```
    pub fn unregister(&self, group_id: &C::GroupId, node_id: &C::NodeId) -> Option<Raft<C>> {
        let key = (group_id.clone(), node_id.clone());

        // Remove from nodes map
        let raft = {
            let mut nodes = self.nodes.write().unwrap();
            nodes.remove(&key)
        };

        // If found, also remove from groups index
        if raft.is_some() {
            let mut groups = self.groups.write().unwrap();
            if let Some(node_ids) = groups.get_mut(group_id) {
                node_ids.retain(|id| id != node_id);
                // Remove the group entry if no nodes left
                if node_ids.is_empty() {
                    groups.remove(group_id);
                }
            }
        }

        raft
    }

    /// Gets a Raft instance by group ID and node ID.
    ///
    /// Returns a clone of the `Raft` handle, which is cheap because `Raft`
    /// internally uses `Arc`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(raft) = router.get(&"my_group".to_string(), &node_id) {
    ///     let response = raft.client_write(request).await?;
    /// }
    /// ```
    pub fn get(&self, group_id: &C::GroupId, node_id: &C::NodeId) -> Option<Raft<C>> {
        let key = (group_id.clone(), node_id.clone());
        let nodes = self.nodes.read().unwrap();
        nodes.get(&key).cloned()
    }

    /// Gets all Raft instances in a group.
    ///
    /// Returns an empty vector if the group doesn't exist.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let nodes = router.get_group(&"my_group".to_string());
    /// for raft in nodes {
    ///     println!("Node: {:?}", raft.node_id());
    /// }
    /// ```
    pub fn get_group(&self, group_id: &C::GroupId) -> Vec<Raft<C>> {
        let groups = self.groups.read().unwrap();
        let nodes = self.nodes.read().unwrap();

        groups
            .get(group_id)
            .map(|node_ids| {
                node_ids
                    .iter()
                    .filter_map(|node_id| {
                        let key = (group_id.clone(), node_id.clone());
                        nodes.get(&key).cloned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Gets all registered Raft instances.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let all_nodes = router.all_nodes();
    /// println!("Total nodes: {}", all_nodes.len());
    /// ```
    pub fn all_nodes(&self) -> Vec<Raft<C>> {
        let nodes = self.nodes.read().unwrap();
        nodes.values().cloned().collect()
    }

    /// Gets all registered group IDs.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let groups = router.all_groups();
    /// for group_id in groups {
    ///     println!("Group: {}", group_id);
    /// }
    /// ```
    pub fn all_groups(&self) -> Vec<C::GroupId> {
        let groups = self.groups.read().unwrap();
        groups.keys().cloned().collect()
    }

    /// Returns the total number of registered Raft instances.
    ///
    /// # Example
    ///
    /// ```ignore
    /// println!("Total nodes: {}", router.len());
    /// ```
    pub fn len(&self) -> usize {
        let nodes = self.nodes.read().unwrap();
        nodes.len()
    }

    /// Returns `true` if no Raft instances are registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the number of registered groups.
    pub fn group_count(&self) -> usize {
        let groups = self.groups.read().unwrap();
        groups.len()
    }

    /// Checks if a specific node is registered.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if router.contains(&"my_group".to_string(), &node_id) {
    ///     println!("Node is registered");
    /// }
    /// ```
    pub fn contains(&self, group_id: &C::GroupId, node_id: &C::NodeId) -> bool {
        let key = (group_id.clone(), node_id.clone());
        let nodes = self.nodes.read().unwrap();
        nodes.contains_key(&key)
    }

    /// Checks if a group exists (has at least one node registered).
    pub fn contains_group(&self, group_id: &C::GroupId) -> bool {
        let groups = self.groups.read().unwrap();
        groups.contains_key(group_id)
    }

    /// Shuts down all registered Raft instances.
    ///
    /// This method:
    /// 1. Collects all registered Raft instances
    /// 2. Calls `shutdown()` on each one
    /// 3. Clears the router
    ///
    /// # Errors
    ///
    /// Returns the first error encountered during shutdown. Even if an error
    /// occurs, the method continues to shut down remaining nodes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// router.shutdown_all().await?;
    /// assert!(router.is_empty());
    /// ```
    pub async fn shutdown_all(&self) -> Result<(), crate::type_config::alias::JoinErrorOf<C>> {
        // Collect all Raft instances first to avoid holding locks during async operations
        let rafts: Vec<Raft<C>> = {
            let nodes = self.nodes.read().unwrap();
            nodes.values().cloned().collect()
        };

        // Clear the router
        {
            let mut nodes = self.nodes.write().unwrap();
            let mut groups = self.groups.write().unwrap();
            nodes.clear();
            groups.clear();
        }

        // Shutdown all Raft instances
        let mut first_error = None;
        for raft in rafts {
            if let Err(e) = raft.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(e);
            }
            // Continue shutting down other nodes even if one fails
        }

        match first_error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Removes all nodes in a specific group.
    ///
    /// Unlike [`shutdown_all`](Self::shutdown_all), this only removes nodes from the router
    /// without calling `shutdown()` on them. Use this if you want to manage shutdown yourself.
    ///
    /// # Returns
    ///
    /// The removed Raft instances, or an empty vector if the group doesn't exist.
    pub fn remove_group(&self, group_id: &C::GroupId) -> Vec<Raft<C>> {
        let node_ids: Vec<C::NodeId> = {
            let groups = self.groups.read().unwrap();
            groups.get(group_id).cloned().unwrap_or_default()
        };

        let mut removed = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            if let Some(raft) = self.unregister(group_id, &node_id) {
                removed.push(raft);
            }
        }

        removed
    }

    /// Clears all registered nodes without shutting them down.
    ///
    /// Use this for testing or when you want to manage Raft lifecycle separately.
    pub fn clear(&self) {
        let mut nodes = self.nodes.write().unwrap();
        let mut groups = self.groups.write().unwrap();
        nodes.clear();
        groups.clear();
    }
}

// Note: RaftRouter is Send + Sync when its internal types are Send + Sync.
// In singlethreaded mode, Raft<C> might not be Send + Sync, so RaftRouter
// also won't be Send + Sync, which is correct for single-threaded runtimes.

impl<C> std::fmt::Debug for RaftRouter<C>
where C: MultiRaftTypeConfig
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nodes = self.nodes.read().unwrap();
        let groups = self.groups.read().unwrap();

        f.debug_struct("RaftRouter")
            .field("node_count", &nodes.len())
            .field("group_count", &groups.len())
            .field("groups", &groups.keys().collect::<Vec<_>>())
            .finish()
    }
}
