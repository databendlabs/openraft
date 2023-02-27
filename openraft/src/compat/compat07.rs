//! This mod provides types that can be deserialized from data written by either v0.7 or
//! the latest openraft.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Debug;

use crate::compat::Compat;
use crate::compat::Upgrade;
use crate::EmptyNode;

impl Upgrade<crate::LogId<u64>> for or07::LogId {
    fn upgrade(self) -> crate::LogId<u64> {
        let committed_leader_id = crate::CommittedLeaderId::new(self.term, 0);
        crate::LogId::new(committed_leader_id, self.index)
    }
}

impl Upgrade<crate::Membership<u64, crate::EmptyNode>> for or07::Membership {
    fn upgrade(self) -> crate::Membership<u64, crate::EmptyNode> {
        let configs = self.get_configs().clone();
        let nodes = self.all_nodes().iter().map(|nid| (*nid, crate::EmptyNode::new())).collect::<BTreeMap<_, _>>();
        crate::Membership::new(configs, nodes)
    }
}

impl Upgrade<crate::StoredMembership<u64, crate::EmptyNode>> for or07::EffectiveMembership {
    fn upgrade(self) -> crate::StoredMembership<u64, crate::EmptyNode> {
        let membership = self.membership.upgrade();
        let log_id = self.log_id.upgrade();

        crate::StoredMembership::new(Some(log_id), membership)
    }
}

impl<C> Upgrade<crate::EntryPayload<C>> for or07::EntryPayload<C::D>
where
    C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>,
    <C as crate::RaftTypeConfig>::D: or07::AppData + Debug,
{
    fn upgrade(self) -> crate::EntryPayload<C> {
        match self {
            Self::Blank => crate::EntryPayload::Blank,
            Self::Membership(m) => crate::EntryPayload::Membership(m.upgrade()),
            Self::Normal(d) => crate::EntryPayload::Normal(d),
        }
    }
}

impl<C> Upgrade<crate::Entry<C>> for or07::Entry<C::D>
where
    C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>,
    <C as crate::RaftTypeConfig>::D: or07::AppData + Debug,
{
    fn upgrade(self) -> crate::Entry<C> {
        let log_id = self.log_id.upgrade();
        let payload = self.payload.upgrade();
        crate::Entry { log_id, payload }
    }
}

impl Upgrade<crate::Vote<u64>> for or07::HardState {
    fn upgrade(self) -> crate::Vote<u64> {
        // When it has not yet voted for any node, let it vote for any node won't break the consensus.
        crate::Vote::new(self.current_term, self.voted_for.unwrap_or_default())
    }
}

impl Upgrade<crate::SnapshotMeta<u64, crate::EmptyNode>> for or07::SnapshotMeta {
    fn upgrade(self) -> crate::SnapshotMeta<u64, crate::EmptyNode> {
        unimplemented!("can not upgrade SnapshotMeta")
    }
}

pub type LogId = Compat<or07::LogId, crate::LogId<u64>>;
pub type Vote = Compat<or07::HardState, crate::Vote<u64>>;
pub type SnapshotMeta = Compat<or07::SnapshotMeta, crate::SnapshotMeta<u64, crate::EmptyNode>>;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Membership {
    pub configs: Vec<BTreeSet<u64>>,
    pub nodes: Option<BTreeMap<u64, crate::EmptyNode>>,
    pub all_nodes: Option<BTreeSet<u64>>,
}

impl Upgrade<crate::Membership<u64, crate::EmptyNode>> for Membership {
    fn upgrade(self) -> crate::Membership<u64, EmptyNode> {
        if let Some(ns) = self.nodes {
            crate::Membership::new(self.configs, ns)
        } else {
            crate::Membership::new(self.configs, self.all_nodes.unwrap())
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct StoredMembership {
    pub log_id: Option<LogId>,
    pub membership: Membership,
    #[serde(skip)]
    pub quorum_set: Option<()>,
    #[serde(skip)]
    pub voter_ids: Option<()>,
}

impl Upgrade<crate::StoredMembership<u64, crate::EmptyNode>> for StoredMembership {
    fn upgrade(self) -> crate::StoredMembership<u64, EmptyNode> {
        crate::StoredMembership::new(self.log_id.map(|lid| lid.upgrade()), self.membership.upgrade())
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum EntryPayload<C: crate::RaftTypeConfig> {
    Blank,
    Normal(C::D),
    Membership(Membership),
}

impl<C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>> Upgrade<crate::EntryPayload<C>>
    for EntryPayload<C>
{
    fn upgrade(self) -> crate::EntryPayload<C> {
        match self {
            EntryPayload::Blank => crate::EntryPayload::Blank,
            EntryPayload::Normal(d) => crate::EntryPayload::Normal(d),
            EntryPayload::Membership(m) => crate::EntryPayload::Membership(m.upgrade()),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(bound = "")]
pub struct Entry<C: crate::RaftTypeConfig> {
    pub log_id: LogId,
    pub payload: EntryPayload<C>,
}

impl<C: crate::RaftTypeConfig<NodeId = u64, Node = crate::EmptyNode>> Upgrade<crate::Entry<C>> for Entry<C> {
    fn upgrade(self) -> crate::Entry<C> {
        crate::Entry {
            log_id: self.log_id.upgrade(),
            payload: self.payload.upgrade(),
        }
    }
}

pub mod testing {
    use std::path::Path;

    use maplit::btreemap;
    use maplit::btreeset;

    use crate::compat;
    use crate::compat::compat07;

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
    pub struct Suite07<B07, BLatest>
    where
        B07: compat07::testing::StoreBuilder07,
        BLatest: compat::testing::StoreBuilder,
    {
        pub builder07: B07,
        pub builder_latest: BLatest,
    }

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
                assert_eq!(want[0].log_id, got[0].log_id);
                assert_eq!(want[1].log_id, got[1].log_id);
                assert_eq!(want[2].log_id, got[2].log_id);

                assert!(matches!(got[0].payload, crate::EntryPayload::Blank));
                if let crate::EntryPayload::Membership(m) = &got[1].payload {
                    assert_eq!(
                        &crate::Membership::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                        ),
                        m
                    )
                } else {
                    unreachable!("expect Membership");
                }

                let s = serde_json::to_string(&got[2].payload)?;
                let want = serde_json::to_string(&crate::EntryPayload::<BLatest::C>::Normal(
                    self.builder_latest.sample_app_data(),
                ))?;
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
                assert_eq!(want[0].log_id, got[0].log_id);

                if let crate::EntryPayload::Membership(m) = &got[0].payload {
                    assert_eq!(
                        &crate::Membership::new(
                            vec![btreeset! {1,2}],
                            btreemap! {1=> crate::EmptyNode{}, 2=>crate::EmptyNode{}},
                        ),
                        m
                    )
                } else {
                    unreachable!("expect Membership");
                }

                // get_log_state
                let got = crate::RaftLogReader::get_log_state(&mut s8).await?;
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

    fn tmp_dir() -> tempdir::TempDir {
        tempdir::TempDir::new("RocksCompatibility").expect("couldn't create temp dir")
    }
}

#[cfg(test)]
mod tests {
    use maplit::btreemap;
    use maplit::btreeset;

    use crate::compat::Upgrade;
    use crate::CommittedLeaderId;

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
        pub TestingConfig: D = u64, R = u64, NodeId = u64, Node = crate::EmptyNode
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
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 3), 5),
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
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 3), 5),
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
            log_id: crate::LogId::new(crate::CommittedLeaderId::new(10, 3), 5),
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
}
