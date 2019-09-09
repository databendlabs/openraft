changelog
=========

### 0.3
#### 0.3.0
Another backwards incompatible change to the `RaftStorage` trait. It is now using associated types to better express the needed trait constraints. These changes were the final bit of work needed to get the entire actix-raft system to work with a Synchronous `RaftStorage` impl. Async impls continue to work as they have, the `RaftStorage` impl block will need to be updated to use the associated types though. The recommend pattern is as follows:

```rust
impl RaftStorage<..., ...> for MyStorage {
    type Actor = Self;
    type Context = Context<Self>; // Or SyncContext<Self>;
}
```

My hope is that this will be the last backwards incompatible change needed before a 1.0 release. This crate is still young though, so we will see.

### 0.2
#### 0.2.0
- Made a few backwards incompatible changes to the `RaftStorage` trait. Overwrite its third type parameter with `actix::SyncContext<Self>` to enable sync storage.
- Also removed the `RaftStorage::new` constructor, as it is a bit restrictive. Just added some docs instead describing what is needed.

### 0.1
#### 0.1.3
- Added a few addition top-level exports for convenience.

#### 0.1.2
- Changes to the README for docs.rs.

#### 0.1.1
- Changes to the README for docs.rs.

#### 0.1.0
- Initial release!
