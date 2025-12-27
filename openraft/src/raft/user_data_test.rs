//! Test the UserData type configuration.

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use crate::declare_raft_types;

/// Custom UserData with interior mutability via AtomicU64.
#[derive(Default)]
pub struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) -> u64 {
        self.0.fetch_add(1, Ordering::Relaxed)
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

declare_raft_types!(
    TestConfig:
        D = u64,
        R = (),
        UserData = Counter,
);

/// Verify that a custom UserData type with interior mutability compiles.
#[test]
fn test_user_data_type_with_interior_mutability() {
    use crate::RaftTypeConfig;

    // Verify Default is implemented and works
    let user_data = <TestConfig as RaftTypeConfig>::UserData::default();
    assert_eq!(user_data.get(), 0);

    // Verify interior mutability works through shared reference
    user_data.inc();
    user_data.inc();
    assert_eq!(user_data.get(), 2);
}

/// Verify that the default UserData type is `()`.
#[test]
fn test_default_user_data_is_unit() {
    use crate::RaftTypeConfig;

    declare_raft_types!(
        DefaultConfig:
            D = u64,
            R = (),
    );

    let _user_data: <DefaultConfig as RaftTypeConfig>::UserData = ();
}
