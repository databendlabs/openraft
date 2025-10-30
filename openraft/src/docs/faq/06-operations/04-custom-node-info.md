### How do I store additional information about nodes in Openraft?

By default, Openraft provide a [`BasicNode`] as the node type in a cluster.
To store more information about each node in Openraft, define a custom struct
with the desired fields and use it in place of `BasicNode`. Here's a brief
guide:

1. Define your custom node struct:

```rust,ignore
#[derive(...)]
struct MyNode {
    ipv4: String,
    ipv6: String,
    port: u16,
    // Add additional fields as needed
}
```

2. Register the custom node type with `declare_raft_types!` macro:

```rust,ignore
openraft::declare_raft_types!(
   pub MyRaftConfig:
       // ...
       NodeId = u64,        // Use the appropriate type for NodeId
       Node = MyNode,       // Replace BasicNode with your custom node type
       // ... other associated types
);
```

Use `MyRaftConfig` in your Raft setup to utilize the custom node structure.

[`BasicNode`]: `crate::node::BasicNode`
