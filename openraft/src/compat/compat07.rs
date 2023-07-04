//! This mod provides types that can be deserialized from data written by either v0.7 or
//! the latest openraft.
//!
//! This mod is enabled by feature flag `compat-07`. See: [feature-flag-compat-07](https://datafuselabs.github.io/openraft/feature-flags)
//! Ap application does not needs to enable this feature if it chooses to manually upgrade v0.7
//! on-disk data.
//!
//! In v0.7 compatible mode, openraft enables feature flags:
//! - `serde`: it adds `serde` implementation to types such as `LogId`.
//! Since generic type `NodeId` and `Node` are introduced in v0.8, the v0.7 compatible mode only
//! works when `NodeId=u64` and `Node=EmptyNode`.
//!
//! The compatible mode still need an application to upgrade codes, but it does not require
//! any manual upgrade to the on-disk data.
//!
//! An application that tries to upgrade from v0.7 can use types in this mod to replace the
//! corresponding ones in a `RaftStorage` implementation, so that v0.7 data and v0.8 data can both
//! be read. [rocksstore-compat07](https://github.com/datafuselabs/openraft/tree/main/rocksstore-compat07) is an example using these type to implement an upgraded RaftStorage
//!
//! This mod also provides a testing suite [`testing::Suite07`] to ensure old data will be correctly
//! read. An application should ensure its storage to pass test suite like [rocksstore-compat07/compatibility_test.rs](https://github.com/datafuselabs/openraft/blob/main/rocksstore-compat07/src/compatibility_test.rs) does:
//! ```ignore
//! use openraft::compat;
//!
//! struct Builder07;
//! struct BuilderLatest;
//!
//! #[async_trait::async_trait]
//! impl compat::testing::StoreBuilder07 for Builder07 {
//!     type D = rocksstore07::RocksRequest;
//!     type R = rocksstore07::RocksResponse;
//!     type S = Arc<rocksstore07::RocksStore>;
//!
//!     async fn build(&self, p: &Path) -> Arc<rocksstore07::RocksStore> {
//!         rocksstore07::RocksStore::new(p).await
//!     }
//!
//!     fn sample_app_data(&self) -> Self::D {
//!         rocksstore07::RocksRequest::Set { key: s("foo"), value: s("bar") }
//!     }
//! }
//!
//! #[async_trait::async_trait]
//! impl compat::testing::StoreBuilder for BuilderLatest {
//!     type C = crate::Config;
//!     type S = Arc<crate::RocksStore>;
//!
//!     async fn build(&self, p: &Path) -> Arc<crate::RocksStore> {
//!         crate::RocksStore::new(p).await
//!     }
//!
//!     fn sample_app_data(&self) -> <<Self as compat::testing::StoreBuilder>::C as openraft::RaftTypeConfig>::D {
//!         crate::RocksRequest::Set { key: s("foo"), value: s("bar") }
//!     }
//! }
//!
//! #[tokio::test]
//! async fn test_compatibility_with_rocksstore_07() -> anyhow::Result<()> {
//!     compat::testing::Suite07 {
//!         builder07: Builder07,
//!         builder_latest: BuilderLatest,
//!     }.test_all().await?;
//!     Ok(())
//! }
//!
//! fn s(v: impl ToString) -> String {
//!     v.to_string()
//! }
//! ```

mod entry;
mod entry_payload;
mod log_id;
mod membership;
mod snapshot_meta;
mod stored_membership;
mod vote;

pub use entry::Entry;
pub use entry_payload::EntryPayload;
pub use log_id::LogId;
pub use membership::Membership;
pub use snapshot_meta::SnapshotMeta;
pub use stored_membership::StoredMembership;
pub use vote::Vote;

pub mod testing {
    use std::path::Path;

    use maplit::btreemap;
    use maplit::btreeset;

    use crate::compat;
    use crate::compat::compat07;
    use crate::entry::RaftPayload;
    use crate::log_id::RaftLogId;

    /// Build a v0.7 `RaftStorage` implementation for compatibility test.
    #[async_trait::async_trait]
    pub trait StoreBuilder07 {
        type D: or07::AppData;
        type R: or07::AppDataResponse;
        type S: or07::RaftStorage<Self::D, Self::R>;

        /// Build a store that is backed by data stored on file system.
        async fn build(&self, p: &Path) -> Self::S;

        /// Build an `AppData` for testing. It has to always produce the same data.
        fn sample_app_data(&self) -> Self::D;
    }

    /// A test suite that ensures data written by an older version `RaftStorage` implementation can
    /// be read correctly by a newer version `RaftStorage` implementation.
    #[allow(unused_qualifications)]
    pub struct Suite07<B07, BLatest>
    where
        B07: compat07::testing::StoreBuilder07,
        BLatest: compat::testing::StoreBuilder,
    {
        pub builder07: B07,
        pub builder_latest: BLatest,
    }

    #[allow(unused_qualifications)]
    impl<B07, BLatest> Suite07<B07, BLatest>
    where
        B07: compat07::testing::StoreBuilder07,
        BLatest: compat::testing::StoreBuilder,
    {
        pub async fn test_all(&self) -> anyhow::Result<()> {
            self.test_v07_hard_state_voted_for_none().await?;
            self.test_v07_hard_state_voted_for_some().await?;
            self.test_v08_vote().await?;
            self.test_v07_append_log().await?;
            self.test_v07_truncate_purge().await?;
            self.test_v07_apply().await?;
            self.test_v07_snapshot().await?;

            Ok(())
        }

        async fn test_v07_hard_state_voted_for_none(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                or07::RaftStorage::save_hard_state(&s7, &or07::HardState {
                    current_term: 1,
                    voted_for: None,
                })
                .await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;
                let got = crate::RaftStorage::read_vote(&mut s8).await?;
                assert_eq!(Some(crate::Vote::new(1, 0)), got);
            }

            Ok(())
        }

        async fn test_v07_hard_state_voted_for_some(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                or07::RaftStorage::save_hard_state(&s7, &or07::HardState {
                    current_term: 1,
                    voted_for: Some(3),
                })
                .await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;
                let got = crate::RaftStorage::read_vote(&mut s8).await?;
                assert_eq!(Some(crate::Vote::new(1, 3)), got);
            }

            Ok(())
        }

        async fn test_v08_vote(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let mut s8 = self.builder_latest.build(td.path()).await;
                crate::RaftStorage::save_vote(&mut s8, &crate::Vote::new(3, 5)).await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;
                let got = crate::RaftStorage::read_vote(&mut s8).await?;
                assert_eq!(Some(crate::Vote::new(3, 5)), got);
            }

            Ok(())
        }

        async fn test_v07_append_log(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                let entries = vec![
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 5 },
                        payload: or07::EntryPayload::Blank,
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 6 },
                        payload: or07::EntryPayload::Membership(or07::Membership::new_single(btreeset! {1,2})),
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 7 },
                        payload: or07::EntryPayload::Normal(self.builder07.sample_app_data()),
                    },
                ];

                or07::RaftStorage::append_to_log(&s7, entries.iter().collect::<Vec<_>>().as_slice()).await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;
                let got = crate::RaftLogReader::try_get_log_entries(&mut s8, 4..9).await?;
                let want: Vec<crate::Entry<BLatest::C>> = vec![
                    crate::Entry {
                        log_id: crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 5),
                        payload: crate::EntryPayload::Blank,
                    },
                    crate::Entry {
                        log_id: crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 6),
                        payload: crate::EntryPayload::Membership(crate::Membership::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                        )),
                    },
                    crate::Entry {
                        log_id: crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 7),
                        payload: crate::EntryPayload::Normal(self.builder_latest.sample_app_data()),
                    },
                ];

                assert_eq!(3, got.len());
                assert_eq!(want[0].log_id, *got[0].get_log_id());
                assert_eq!(want[1].log_id, *got[1].get_log_id());
                assert_eq!(want[2].log_id, *got[2].get_log_id());

                assert!(got[0].is_blank());
                if let Some(m) = got[1].get_membership() {
                    assert_eq!(
                        &crate::Membership::<u64, crate::EmptyNode>::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                        ),
                        m
                    )
                } else {
                    unreachable!("expect Membership");
                }

                let s = serde_json::to_string(&got[2])?;
                let want = serde_json::to_string(&crate::Entry::<BLatest::C> {
                    log_id: crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 7),
                    payload: crate::EntryPayload::Normal(self.builder_latest.sample_app_data()),
                })?;
                assert_eq!(want, s);
            }

            Ok(())
        }

        async fn test_v07_truncate_purge(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                let entries = vec![
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 5 },
                        payload: or07::EntryPayload::Blank,
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 6 },
                        payload: or07::EntryPayload::Membership(or07::Membership::new_single(btreeset! {1,2})),
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 7 },
                        payload: or07::EntryPayload::Normal(self.builder07.sample_app_data()),
                    },
                ];

                or07::RaftStorage::append_to_log(&s7, entries.iter().collect::<Vec<_>>().as_slice()).await?;
                or07::RaftStorage::delete_conflict_logs_since(&s7, or07::LogId { term: 1, index: 7 }).await?;
                or07::RaftStorage::purge_logs_upto(&s7, or07::LogId { term: 1, index: 5 }).await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;

                let got = crate::RaftLogReader::try_get_log_entries(&mut s8, 4..9).await?;
                let want: Vec<crate::Entry<BLatest::C>> = vec![crate::Entry {
                    log_id: crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 6),
                    payload: crate::EntryPayload::Membership(crate::Membership::new(
                        vec![btreeset! {1,2}],
                        btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                    )),
                }];

                assert_eq!(1, got.len());
                assert_eq!(want[0].log_id, *got[0].get_log_id());

                if let Some(m) = got[0].get_membership() {
                    assert_eq!(
                        &crate::Membership::<u64, crate::EmptyNode>::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                        ),
                        m
                    )
                } else {
                    unreachable!("expect Membership");
                }

                // get_log_state
                let got = crate::RaftStorage::get_log_state(&mut s8).await?;
                assert_eq!(
                    crate::LogState {
                        last_purged_log_id: Some(crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 5)),
                        last_log_id: Some(crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 6)),
                    },
                    got
                );
            }

            Ok(())
        }

        async fn test_v07_apply(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                let entries = vec![
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 5 },
                        payload: or07::EntryPayload::Blank,
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 6 },
                        payload: or07::EntryPayload::Membership(or07::Membership::new_single(btreeset! {1,2})),
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 7 },
                        payload: or07::EntryPayload::Normal(self.builder07.sample_app_data()),
                    },
                ];

                or07::RaftStorage::apply_to_state_machine(&s7, entries.iter().collect::<Vec<_>>().as_slice()).await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;

                let (last_log_id, last_membership) = crate::RaftStorage::last_applied_state(&mut s8).await?;
                assert_eq!(
                    Some(crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 7),),
                    last_log_id
                );
                assert_eq!(
                    crate::StoredMembership::new(
                        Some(crate::LogId::new(crate::CommittedLeaderId::new(1, 0), 6),),
                        crate::Membership::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}}
                        )
                    ),
                    last_membership
                );
            }

            Ok(())
        }

        async fn test_v07_snapshot(&self) -> anyhow::Result<()> {
            let td = tmp_dir();
            {
                let s7 = self.builder07.build(td.path()).await;
                let entries = vec![
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 5 },
                        payload: or07::EntryPayload::Blank,
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 6 },
                        payload: or07::EntryPayload::Membership(or07::Membership::new_single(btreeset! {1,2})),
                    },
                    or07::Entry {
                        log_id: or07::LogId { term: 1, index: 7 },
                        payload: or07::EntryPayload::Normal(self.builder07.sample_app_data()),
                    },
                ];

                or07::RaftStorage::apply_to_state_machine(&s7, entries.iter().collect::<Vec<_>>().as_slice()).await?;
                or07::RaftStorage::build_snapshot(&s7).await?;
            }
            {
                let mut s8 = self.builder_latest.build(td.path()).await;

                let snapshot = crate::RaftStorage::get_current_snapshot(&mut s8).await?;
                assert!(
                    snapshot.is_none(),
                    "SnapshotMeta.last_membership is introduced in 0.8, can not use an old snapshot"
                );
            }

            Ok(())
        }
    }

    fn tmp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().expect("couldn't create temp dir")
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use maplit::btreemap;
    use maplit::btreeset;

    use crate::compat::Upgrade;
    use crate::CommittedLeaderId;
    use crate::TokioRuntime;

    #[test]
    fn test_serde_log_id() -> anyhow::Result<()> {
        use super::LogId;

        let v7 = or07::LogId::new(10, 5);
        let want = crate::LogId::new(CommittedLeaderId::new(10, 0), 5);

        let s = serde_json::to_string(&v7)?;
        let c: LogId = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: LogId = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_vote() -> anyhow::Result<()> {
        use super::Vote;

        let v7 = or07::HardState {
            current_term: 5,
            voted_for: Some(3),
        };
        let want = crate::Vote::new(5, 3);

        let s = serde_json::to_string(&v7)?;
        let c: Vote = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: Vote = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_membership() -> anyhow::Result<()> {
        use super::Membership;

        let v7 = or07::Membership::new_single(btreeset! {1,2});
        let want = crate::Membership::new(
            vec![btreeset! {1,2}],
            btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}},
        );

        let s = serde_json::to_string(&v7)?;
        let c: Membership = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: Membership = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_effective_membership() -> anyhow::Result<()> {
        use super::StoredMembership;

        let m7 = || or07::Membership::new_single(btreeset! {1,2});
        let m8 = || {
            crate::Membership::new(
                vec![btreeset! {1,2}],
                btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}},
            )
        };

        let v7 = or07::EffectiveMembership::new(or07::LogId::new(5, 3), m7());
        let want = crate::StoredMembership::new(Some(crate::LogId::new(CommittedLeaderId::new(5, 0), 3)), m8());

        let s = serde_json::to_string(&v7)?;
        let c: StoredMembership = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: StoredMembership = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    crate::declare_raft_types!(
        pub TestingConfig:
        D = u64, R = u64, NodeId = u64, Node = crate::EmptyNode,
        Entry = crate::Entry<TestingConfig>, SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime
    );

    #[test]
    fn test_serde_entry_payload_blank() -> anyhow::Result<()> {
        use super::EntryPayload;

        let v7 = or07::EntryPayload::<u64>::Blank;
        let want = crate::EntryPayload::<TestingConfig>::Blank;

        let s = serde_json::to_string(&v7)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_entry_payload_normal() -> anyhow::Result<()> {
        use super::EntryPayload;

        let v7 = or07::EntryPayload::<u64>::Normal(3);
        let want = crate::EntryPayload::<TestingConfig>::Normal(3);

        let s = serde_json::to_string(&v7)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_entry_payload_membership() -> anyhow::Result<()> {
        use super::EntryPayload;

        let m7 = || or07::Membership::new_single(btreeset! {1,2});
        let m8 = || {
            crate::Membership::new(
                vec![btreeset! {1,2}],
                btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}},
            )
        };

        let v7 = or07::EntryPayload::<u64>::Membership(m7());
        let want = crate::EntryPayload::<TestingConfig>::Membership(m8());

        let s = serde_json::to_string(&v7)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(want, v8);

        let s = serde_json::to_string(&v8)?;
        let c: EntryPayload<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(want, c.upgrade());
        Ok(())
    }

    #[test]
    fn test_serde_entry_blank() -> anyhow::Result<()> {
        use super::Entry;

        let v7 = or07::Entry {
            log_id: or07::LogId::new(10, 5),
            payload: or07::EntryPayload::<u64>::Blank,
        };
        let want = crate::Entry {
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 0), 5),
            payload: crate::EntryPayload::<TestingConfig>::Blank,
        };

        let s = serde_json::to_string(&v7)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&v8)?);

        let s = serde_json::to_string(&v8)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&c.upgrade())?);
        Ok(())
    }

    #[test]
    fn test_serde_entry_normal() -> anyhow::Result<()> {
        use super::Entry;

        let v7 = or07::Entry {
            log_id: or07::LogId::new(10, 5),
            payload: or07::EntryPayload::<u64>::Normal(3),
        };
        let want = crate::Entry {
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 0), 5),
            payload: crate::EntryPayload::<TestingConfig>::Normal(3),
        };

        let s = serde_json::to_string(&v7)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&v8)?);

        let s = serde_json::to_string(&v8)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&c.upgrade())?);
        Ok(())
    }

    #[test]
    fn test_serde_entry_membership() -> anyhow::Result<()> {
        use super::Entry;

        let m7 = || or07::Membership::new_single(btreeset! {1,2});
        let m8 = || {
            crate::Membership::new(
                vec![btreeset! {1,2}],
                btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}},
            )
        };
        let v7 = or07::Entry {
            log_id: or07::LogId::new(10, 5),
            payload: or07::EntryPayload::<u64>::Membership(m7()),
        };
        let want = crate::Entry {
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 0), 5),
            payload: crate::EntryPayload::<TestingConfig>::Membership(m8()),
        };

        let s = serde_json::to_string(&v7)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        let v8 = c.upgrade();
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&v8)?);

        let s = serde_json::to_string(&v8)?;
        let c: Entry<TestingConfig> = serde_json::from_str(&s)?;
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&c.upgrade())?);
        Ok(())
    }

    #[test]
    fn test_serde_snapshot_meta() -> anyhow::Result<()> {
        use super::SnapshotMeta;

        let m8 = || {
            crate::Membership::new(
                vec![btreeset! {1,2}],
                btreemap! {1=>crate::EmptyNode{}, 2=>crate::EmptyNode{}},
            )
        };
        let v7 = or07::SnapshotMeta {
            last_log_id: Some(or07::LogId::new(10, 5)),
            snapshot_id: "a".to_string(),
        };
        let want = crate::SnapshotMeta {
            last_log_id: Some(crate::LogId::new(crate::CommittedLeaderId::new(10, 0), 5)),
            last_membership: crate::StoredMembership::new(
                Some(crate::LogId::new(crate::CommittedLeaderId::new(10, 0), 5)),
                m8(),
            ),
            snapshot_id: "a".to_string(),
        };

        let s = serde_json::to_string(&v7)?;
        let c: SnapshotMeta = serde_json::from_str(&s)?;
        assert!(
            c.try_upgrade().is_err(),
            "snapshot_meta can not be upgrade because it has no membership"
        );

        let s = serde_json::to_string(&want)?;
        let c: SnapshotMeta = serde_json::from_str(&s)?;
        let c = c.try_upgrade().unwrap();
        assert_eq!(serde_json::to_string(&want)?, serde_json::to_string(&c)?);

        Ok(())
    }
}
