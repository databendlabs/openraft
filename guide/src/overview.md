{{#include ../../README.md}}

### overview
This crate's `Raft` type is an Actix actor which is intended to run within some parent application, which traditionally will be some sort of data storage system (SQL, NoSQL, KV store, AMQP, Streaming, whatever). Inasmuch as the `Raft` instance is an actor, it is expected that the parent application is also built upon the Actix actor framework, though that is not technically required.

To use this this crate, applications must also implement the `RaftStorage` & `RaftNetwork` traits. See the [storage](https://railgun-rs.github.io/actix-raft/storage.html) & [network](https://railgun-rs.github.io/actix-raft/network.html) chapters for details on what these traits represent and how to implement them. In brief, the implementing types must be actors which can handle specific message types which correspond to everything needed for Raft storage and networking.

The following diagram shows how client requests are presented to Raft from within an application, how the data is stored, replicated and ultimately applied to the application's state machine.

<p>
    <img src="./images/raft-workflow-client-requests.png"/>
</p>

The numbered elements represent segments of the workflow.
1. The parent application has received a client request, and presents the payload to `Raft` using the `ClientPayload` type.
2. `Raft` will present the payload to the `RaftStorage` impl via the `AppendLogEntry` type. This is the one location where the `RaftStorage` impl may return an application specific error. This could be for validation logic, enforcing unique indices, data/schema validation; whatever application level rules the application enforces, this is where they should be enforced. Close to the data, just before it hits the `Raft` log.
3. The `RaftStorage` impl responds to the `Raft` actor. If it is successful, go to step 4, else the error response will be sent back to the caller immediately. The error response is a statically known type defined by the parent application.
4. `Raft` uses the `RaftNetwork` impl to communicate with the peers of the cluster.
5. `Raft` uses the `RaftNetwork` impl to replicate the entry to all other nodes in the cluster.
6. Follower nodes in the cluster respond upon successful replication.
7. Once the entry has been replicated to a majority of nodes in the cluster — known as a "committed" entry in the Raft spec — it is ready to be applied to the application's state machine.
8. `Raft` will apply the entry to the application's state machine via the `ApplyToStateMachine` type.
9. The `RaftStorage` impl responds to the `Raft` actor.
10. The success response is returned to the caller.

**NOTE:** this implementation of Raft offers the option for client requests to receive a response once its entry has been committed, and before it is applied to the state machine. This is controlled by the `ClientPayload.response_type` field, which is an instance of the `ResponseMode` enum which may be either `Committed` or `Applied`. Application's may use either depending on their needs.

**NOTE:** the appearance of the "section" symbols `§` throughout this project are references to specific sections of the Raft spec.
