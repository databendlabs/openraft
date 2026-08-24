//! Test the `declare_raft_types` macro with default values

#![allow(dead_code)]

use std::fmt;

use openraft_rt_tokio::TokioRuntime;

use crate::EntryPayload;
use crate::Membership;
use crate::RaftTypeConfig;
use crate::declare_raft_types;
use crate::entry::RaftPayload;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
struct CustomPayload(EntryPayload<u64, u64, ()>);

impl fmt::Display for CustomPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl RaftPayload for CustomPayload {
    type D = u64;
    type NodeId = u64;
    type Node = ();

    fn blank() -> Self {
        Self(EntryPayload::blank())
    }

    fn with_normal(self, data: u64) -> Self {
        Self(self.0.with_normal(data))
    }

    fn with_membership(self, membership: Membership<u64, ()>) -> Self {
        Self(self.0.with_membership(membership))
    }

    fn get_membership(&self) -> Option<Membership<u64, ()>> {
        self.0.get_membership()
    }
}

declare_raft_types!(
    All:
        NodeId = u64,
        Node = (),

        /// This is AppData
        D = u64,
        #[allow(dead_code)]
        #[allow(dead_code)]
        R = (),
        Term = u64,
        LeaderId = crate::impls::leader_id_std::LeaderId<u64, u64>,
        Entry = crate::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::D, Self::NodeId, Self::Node>,
        Vote = crate::impls::Vote<Self::LeaderId>,
        AsyncRuntime = TokioRuntime,
        // Responder<T> is not supported by  declare_raft_types
        // Responder<T> = crate::impls::OneshotResponder<Self, T> where T: OptionalSend + 'static,
);

declare_raft_types!(
    WithoutD:
        R = (),
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::D, Self::NodeId, Self::Node>,
        AsyncRuntime = TokioRuntime,
);

declare_raft_types!(
    WithoutR:
        D = u64,
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<<Self::LeaderId as crate::vote::RaftLeaderId>::Committed, Self::D, Self::NodeId, Self::Node>,
        AsyncRuntime = TokioRuntime,
);

declare_raft_types!(EmptyWithColon:);

declare_raft_types!(Empty);

declare_raft_types!(
    WithCustomPayload:
        D = u64,
        R = (),
        Node = (),
        Payload = CustomPayload,
        AsyncRuntime = TokioRuntime,
);

#[test]
fn test_payload_type() {
    fn assert_payload<C, P>()
    where
        C: RaftTypeConfig<Payload = P>,
        P: RaftPayload,
    {
    }

    assert_payload::<WithCustomPayload, CustomPayload>();
    assert_payload::<Empty, EntryPayload<String, u64, crate::impls::BasicNode>>();
}
