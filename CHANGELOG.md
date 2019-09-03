changelog
=========

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
