//! Test the `declare_raft_types` macro with default values

use std::io::Cursor;

use crate::declare_raft_types;
use crate::TokioRuntime;

declare_raft_types!(
    All:
        D = (),
        R = (),
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<Self>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime,
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
        D = (),
        NodeId = u64,
        Node = (),
        Entry = crate::Entry<Self>,
        SnapshotData = Cursor<Vec<u8>>,
        AsyncRuntime = TokioRuntime,
);

// This raise an compile error:
// > error: Type not in its expected position : NodeId = u64, D = (), types must present
// > in this order : D, R, NodeId, Node, Entry, SnapshotData, AsyncRuntime
// declare_raft_types!(
//     Foo:
//         Node = (),
//         NodeId = u64,
//         D = (),
// );

declare_raft_types!(EmptyWithColon:);

declare_raft_types!(Empty);
