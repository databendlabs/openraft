//! Test the `declare_raft_types` macro with default values

#![allow(dead_code)]

use std::io::Cursor;

use crate::declare_raft_types;
use crate::impls::TokioRuntime;

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
        LeaderId = crate::impls::leader_id_std::LeaderId<Self>,
        Entry = crate::Entry<Self>,
        Vote = crate::impls::Vote<Self>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime,
        // Responder<T> is not supported by  declare_raft_types
        // Responder<T> = crate::impls::OneshotResponder<Self, T> where T: OptionalSend + 'static,
);

declare_raft_types!(
    WithoutD:
        R = (),
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<Self>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime,
);

declare_raft_types!(
    WithoutR:
        D = u64,
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<Self>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime,
);

declare_raft_types!(EmptyWithColon:);

declare_raft_types!(Empty);
