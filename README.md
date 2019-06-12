actix-raft
==========
An implementation of the Raft consensus protocol using the actix framework.

This differs from other Raft implementations in that it is fully reactive and embraces the async ecosystem.

- It is driven by actual Raft related events taking place in the system as opposed to being driven by a `tick` operation.
- Integrates seamlessly with other actors by exposing a clean set of message types which must be used to interact with the Raft actor from this crate, thus providing a more simple and clear API.
- This Raft implementation provides the core Raft consensus protocol logic, but provides clean integration points for efficiently implementing the networking and storage layer.
- Uses actix traits (actors & handlers) as integration points for users to implement the networking and storage layers. This allows for the storage interface to be synchronous or asynchronous based on the storage engine used, and allows for easily integrating with the actix ecosystem's networking components for efficient async networking.
- Does not impose buffering of outbound messages, but still allows for it if needed.
- Uses [Prost](https://github.com/danburkert/prost) for simple and clean protocol buffers data modelling.

This implementation strictly adheres to the [Raft spec](https://raft.github.io/raft.pdf) (*pdf warning*), and all data models use the same nomenclature found in the spec for better understandability.
