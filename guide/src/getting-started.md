Getting Started
===============
Raft is a distributed consensus protocol designed to manage a replicated log containing state machine commands from clients. Why use Raft? Among other things, it provides data storage systems with fault-tolerance, strong consistency and linearizability.

A visual depiction of how Raft works (taken from the spec) can be seen below.

<p>
    <img style="max-width:600px;" src="./images/raft-overview.png"/>
</p>

Raft is intended to run within some parent application, which traditionally will be some sort of data storage system (SQL, NoSQL, KV store, AMQP, Streaming, Graph, whatever). You can do whatever you want with your application, Raft will provide you with the consensus module.

## first steps
In order to start using Raft, you will need to declare the data types you will use for client requests and client responses. Let's do that now. Throughout this guide, we will be using the `memstore` crate, which is an in-memory implementation of the `RaftStorage` trait for demo and testing purposes (part of the same repo). This will give us a concrete set of examples to work with, which also happen to be used for all of the integration tests of `async-raft` itself.

### `async_raft::AppData`
This marker trait is used to declare an application's data type. It has the following constraints: `Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static`. Your data type represents the requests which will be sent to your application to create, update and delete data. Requests to read data should not be sent through Raft, only mutating requests. More on linearizable reads, and how to avoid stale reads, is discussed in the [Raft API chapter](TODO:).

The intention of this trait is that applications which are using this crate will be able to use their own concrete data types throughout their application without having to serialize and deserialize their data as it goes through Raft. Instead, applications can present their data models as-is to Raft, Raft will present it to the application's `RaftStorage` impl when ready, and the application may then deal with the data directly in the storage engine without having to do a preliminary deserialization.

##### impl
Finishing up this step is easy, just `impl AppData for YourData {}` ... and in most cases, that's it. You'll need to be sure that the aforementioned constraints are satisfied on `YourData`. The following derivation should do the trick `#[derive(Clone, Debug, Serialize, Deserialize)]`.

In the `memstore` crate, here is a snippet of what the code looks like:

```rust
/// The application data request type which the `MemStore` works with.
///
/// Conceptually, for demo purposes, this represents an update to a client's status info,
/// returning the previously recorded status.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientRequest {
    /* fields omitted */
}

impl AppData for ClientRequest {}
```

### `async_raft::AppDataResponse`
This marker trait is used to declare an application's response data. It has the following constraints: `Clone + Debug + Send + Sync + Serialize + DeserializeOwned + 'static`.

The intention of this trait is that applications which are using this crate will be able to use their own concrete data types for returning response data from the storage layer when an entry is applied to the state machine as part of a client request (this is not used during replication). This allows applications to seamlessly return application specific data from their storage layer, up through Raft, and back into their application for returning data to clients.

This type must encapsulate both success and error responses, as application specific logic related to the success or failure of a client request — application specific validation logic, enforcing of data constraints, and anything of that nature — are expressly out of the realm of the Raft consensus protocol.

##### impl
Finishing up this step is also easy: `impl AppDataResponse for YourDataResponse {}`. The aforementioned derivation applies here as well.

In the `memstore` crate, here is a snippet of what the code looks like:

```rust

/// The application data response type which the `MemStore` works with.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientResponse(Result<Option<String>, ClientError>);

impl AppDataResponse for ClientResponse {}
```

---

Woot woot! Onward to the networking layer.
