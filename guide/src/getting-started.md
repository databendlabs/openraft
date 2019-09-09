Getting Started
===============
This crate's `Raft` type is an Actix actor which is intended to run within some parent application, which traditionally will be some sort of data storage system (SQL, NoSQL, KV store, AMQP, Streaming, whatever). Inasmuch as the `Raft` instance is an actor, it is expected that the parent application is also built upon the Actix actor framework, though that is not technically required.

To use this crate, applications must also implement the `RaftStorage` & `RaftNetwork` traits. See the [storage](https://railgun-rs.github.io/actix-raft/storage.html) & [network](https://railgun-rs.github.io/actix-raft/network.html) chapters for details on what these traits represent and how to implement them. In brief, the implementing types must be actors which can handle specific message types which correspond to everything needed for Raft storage and networking.

### deep dive
To get started, applications can define a type alias which declares the types which are going to be used for the application's data, errors, `RaftNetwork` impl & `RaftStorage` impl.

First, let's define the new application's main data struct. This is the data which will be inside of Raft's normal log entries.

```rust
use actix_raft::AppData;
use serde::{Serialize, Deserialize};

/// The application's data struct.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Data {
    // Your data fields go here.
}

/// This also has a `'static` lifetime constraint, so no `&` references
/// at this time. The new futures & async/await should help out with this
/// quite a lot, so hopefully this constraint will be removed in actix as well.
impl AppData for Data {}
```

Now we'll define the application's error type.

```rust
use actix_raft::AppError;
use serde::{Serialize, Deserialize};

/// The application's error struct. This could be an enum as well.
///
/// NOTE: the below impls for Display & Error can be
/// derived using crates like `Failure` &c.
#[derive(Debug, Serialize, Deserialize)]
pub struct Error;

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // ... snip ...
    }
}

impl std::error::Error for Error {}

// Mark this type for use as an `actix_raft::AppError`.
impl AppError for Error {}
```

Now for the two big parts. `RaftNetwork` & `RaftStorage`. Here, we will only look at the skeleton for these types. See the [network](https://railgun-rs.github.io/actix-raft/network.html) & [storage](https://railgun-rs.github.io/actix-raft/storage.html) chapters for more details on how to actually implement these types. First, let's cover the network impl.

```rust
use actix::{Actor, Context, ResponseActFuture};
use actix_raft::{RaftNetwork, messages};

/// Your application's network interface actor.
struct AppNetwork {/* ... snip ... */}

impl Actor for AppNetwork {
    type Context = Context<Self>;

    // ... snip ... other actix methods can be implemented here as needed.
}

// Ensure you impl this over your application's data type. Here, it is `Data`.
impl RaftNetwork<Data> for AppNetwork {}

// Then you just implement the various message handlers.
// See the network chapter for details.
impl Handler<messages::AppendEntriesRequest<Data>> for AppNetwork {
    type Result = ResponseActFuture<Self, messages::AppendEntriesResponse, ()>;

    fn handle(&mut self, _msg: messages::AppendEntriesRequest<Data>, _ctx: &mut Self::Context) -> Self::Result {
        // ... snip ...
        # actix::fut::err(())
    }
}

// Impl handlers on `AppNetwork` for the other `actix_raft::messages` message types.
```

Now for the storage impl. The storage impl will assume an `actix::Context` by default (which is async). Overwrite the third type parameter to `RaftStorage` as `actix::SyncContext<Self>` if your storage impl needs to be synchronous.

```rust
use actix::{Actor, Context, ResponseActFuture};
use actix_raft::{NodeId, RaftStorage, storage};

/// Your application's storage interface actor.
struct AppStorage {/* ... snip ... */}

// Ensure you impl this over your application's data & error types.
// For sync storage, write as: `RaftStorage<Data, Error, >`
impl RaftStorage<Data, Error, SyncContext<Self>> for AppStorage {}

impl Actor for AppStorage {
    type Context = Context<Self>; // Use `SyncContext<Self>` if storage is sync.

    // ... snip ... other actix methods can be implemented here as needed.
}

// Then you just implement the various message handlers.
// See the storage chapter for details.
impl Handler<storage::GetInitialState<Error>> for AppStorage {
    type Result = ResponseActFuture<Self, storage::InitialState, Error>;

    fn handle(&mut self, _msg: storage::GetInitialState<Error>, _ctx: &mut Self::Context) -> Self::Result {
        // ... snip ...
        # actix::fut::err(())
    }
}

// Impl handlers on `AppStorage` for the other `actix_raft::storage` message types.
```

In order for Raft to expose metrics on how it is doing, we will need a type which can receive `RaftMetrics` messages. Application's can do whatever they want with this info. Expose integrations with Prometheus & Influx, trigger events, whatever is needed. Here we will keep it simple.

```rust
use actix::{Actor, Context};
use actix_raft::RaftMetrics;

/// Your application's metrics interface actor.
struct AppMetrics {/* ... snip ... */}

impl Actor for AppMetrics {
    type Context = Context<Self>;

    // ... snip ... other actix methods can be implemented here as needed.
}

impl Handler<RaftMetrics> for AppMetrics {
    type Result = ();

    fn handle(&mut self, _msg: RaftMetrics, _ctx: &mut Context<Self>) -> Self::Result {
        // ... snip ...
    }
}
```

And finally, a simple type alias which ties everything together. This type alias can then be used throughout the application's code base without the need to specify the various types being used for data, errors, network & storage.

```rust
use actix_raft::Raft;

/// A type alias used to define an application's concrete Raft type.
type AppRaft = Raft<Data, Error, AppNetwork, AppStorage>;
```

### booting up the system
Now that the various needed types are in place, the actix system will need to be started, the various actor types we've defined above will need to be started, and then we're off to the races.

```rust
use actix;
use actix_raft::{Config, ConfigBuilder, SnapshotPolicy};

fn main() {
    // Build the actix system.
    let sys = actix::System::new("my-awesome-app");

    // Build the needed runtime config for Raft specifying where
    // snapshots will be stored. See the storage chapter for more details.
    let config = Config::build(String::from("/app/snapshots")).validate().unwrap();

    // Start off with just a single node in the cluster. Applications
    // should implement their own discovery system. See the cluster
    // formation chapter for more details.
    let members = vec![1];

    // Start the various actor types and hold on to their addrs.
    let network = AppNetwork.start();
    let storage = AppStorage.start();
    let metrics = AppMetrics.start();
    let app_raft = AppRaft::new(1, config, network, storage, metrics).start();

    // Run the actix system. Unix signals for termination &
    // graceful shutdown are automatically handled.
    let _ = sys.run();
}
```

----

You've already ascended to the next level of AWESOME! There is a lot more to cover, we're just getting started. Next, let's take a look at the `Raft` type in more detail.
