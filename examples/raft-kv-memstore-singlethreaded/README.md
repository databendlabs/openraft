# Example single threaded key-value store

Example key-value store with `openraft`, single threaded, i.e., Openraft does not require `Send` for data types,
by enabling feature flag `singlethreaded`

In this example, `NodeId` and application request `Request` are not `Send`, by enabling feature flag `singlethreaded`:
`openraft = { path = "../../openraft", features = ["singlethreaded"] }`,
Openraft works happily with non-`Send` data types:

```rust
pub struct NodeId {
    pub id: u64,
    // Make it !Send
    _p: PhantomData<*const ()>,
}
pub enum Request {
    Set {
        key: String,
        value: String,
        // Make it !Send
        _p: PhantomData<*const ()>,
    }
}
```

## Run it

Run it with `cargo test -- --nocaputre`.