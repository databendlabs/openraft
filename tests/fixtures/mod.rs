pub mod router;

use actix_raft::{
    Raft,
    memory_storage::{MemoryStorage, MemoryStorageError},
};

/// Type concrete Raft type used during testing.
pub type MemRaft = Raft<MemoryStorageError, router::RaftRouter, MemoryStorage>;
