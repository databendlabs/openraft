# Openraft Architecture

The Openraft architecture is designed to provide a modular and extensible
framework for implementing the Raft consensus algorithm. Each component serves a
specific purpose and communicates with other components through channels or
APIs. Users can customize the network, log storage, and state machine layers to
fit their specific needs and requirements.

The major components inside Openraft include:

- **[`Raft`]**: This is the control handle for the application and user. It
  provides APIs to send `RaftMsg` to `RaftCore` to manipulate Openraft.

-   **`RaftCore`**: This is the main component running in a `tokio::task`,
    which handles all client requests (such as [`client_write`]) and Raft
    protocol requests (such as [`append_entries`]). It is primarily an event
    loop that receives messages from the `RaftMsg` channel and the internal
    notification channel `Notify`.

    In `v0.8`, [`RaftLogStorage`] runs in the same task as `RaftCore`, while
    [`RaftStateMachine`] runs in a standalone task.

-   **`ReplicationHandle`**: This is the control handle for replication tasks,
    e.g., sending replication commands to `ReplicationCore`.

-   **`ReplicationCore`**: This is another event loop running in separate tasks
    to replicate logs or snapshots. It communicates with `RaftCore` through
    channels.

-   **`HeartbeatWorkersHandle`**: This control handle manages all heartbeat
    tasks: `HeartbeatWorker`.

-   **`HeartbeatWorker`**: This specialized task is solely responsible for
    sending heartbeat messages (in the form of an empty
    [`AppendEntriesRequest`]) to a specific target node.  This task, separate
    from the `ReplicationCore`, is responsible for sending heartbeats. This
    design prevents intensive log entry replication from blocking heartbeat
    acknowledgments, which could otherwise lead to the Leader's lease expiration
    and subsequent step down.

-   **[`RaftNetwork`]**: This is a user-provided component that implements the
    network transport layer, e.g., sending logs to a remote node or sending a
    [`VoteRequest`] to a remote node.

-   **[`RaftLogStorage`]**: This is a user-provided component that implements the
    log storage layer, e.g., appending a log to the log storage or reading a log
    from the log storage.

-   **[`RaftStateMachine`]**: This is a user-provided component that implements
    the state machine and snapshot, e.g., applying a log to the state machine or
    building a snapshot from the state machine.



[`Raft`]:                 `crate::raft::Raft`
[`client_write`]:         `crate::raft::Raft::client_write`
[`RaftLogStorage`]:       `crate::storage::RaftLogStorage`
[`RaftStateMachine`]:     `crate::storage::RaftStateMachine`
[`RaftNetwork`]:          `crate::network::RaftNetwork`
[`append_entries`]:       `crate::network::RaftNetwork::append_entries`
[`VoteRequest`]:          `crate::raft::VoteRequest`
[`AppendEntriesRequest`]: `crate::raft::message::AppendEntriesRequest`

[//]: # (private)
[//]: # ([`RaftMsg`]:           `crate::raft::RaftMsg`)
[//]: # ([`RaftCore`]:          `crate::core::RaftCore`)
[//]: # ([`Notify`]:            `crate::core::notify::Notify`)
[//]: # ([`ReplicationHandle`]: `crate::replication::ReplicationHandle`)
[//]: # ([`ReplicationCore`]:   `crate::replication::ReplicationCore`)



```bob

        .~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~.
        !          User                   !
        !          o                      !
        !          |                      !
        !          | "client_write(impl AppData)"
        !          | "ensure_linearizable()"         
        !          | "change_membership()"!
        !          v                      !
        !          Raft                   !                  .-----> Raft ----.
        '~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~'                  |                |       
                   |                                         |                |       
                   | enum RaftMes                            |                |       
                   |                                         |                |       
           .~~~~~~~|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~.         |         .~~~~~~|~~~~~~~~~~~~.
"append()" !       v                               !         |         !      v            !
      .------o RaftCore -------------------------------.     |         !      RaftCore     !
      |    !   ReplicationHandle                   !   |     |         !                   !
      |    !   |  ReplicationHandle                !   |     |         '~~~~~~~~~~~~~~~~~~~'
      |    !   |  |  HeartbeatWorkersHandle        !   |     |    
      |    !   |  |       |                        !   |     |                                                       
      |    '~~~|~~|~~~~~~~|~~~~~~~~~~~~~~~~~~~~~~~~'   |     |                                                       
      |        |  |       |                            |     |    
      |        |  |       |     .~~~~~~~~~~~~~~~~~.    |     |                       
      |        |  |       +------>HeartbeatWorker !    |     |                       
      |        |  |       |     !  RaftNetwork---------------+                       
      |        |  |       |     '~~~~~~~~~~~~~~~~~'    |     |                       
      |        |  |       |     .~~~~~~~~~~~~~~~~~.    |     | RPC:                      
      |        |  |       `------>HeartbeatWorker !    |     |   "vote()"                
      |        |  |             !  RaftNetwork    !    |     |   "append_entries()"      
      |        |  |             '~~~~~~~~~~~~~~~~~'    |     |   "snapshot()"    
      |        |  |                                    |     |   "transfer_leader()"
      |        |  |     .~~~~~~~~~~~~~~~~~.            |     |    
      |        |  `------>ReplicationCore !            |     |    
      |   .----|--------!-o  RaftNetwork---------------------+    
      |   |    |        '~~~~~~~~~~~~~~~~~'            |     |    
      |   |    |        .~~~~~~~~~~~~~~~~~.            |     |    
      |   |    `--------->ReplicationCore !            |     |    
      |   +-------------!-o  RaftNetwork---------------------'    
      |   |             '~~~~~~~~~~~~~~~~~'            |          
      |   |                                            | "apply()"
      |   |                                            | "build_snapshot()"
      |   | "try_get_log_entries()"        .-----------' "install_snapshot()"
      |   |                                |                   
      |   |                     .~~~~~~~~~~|~~~~~~~~~~. 
      |   |                     !          v          ! 
      |   |                     !   RaftStateMachine  ! 
      |   |                     !          o          ! 
      |   |                     '~~~~~~~~~~|~~~~~~~~~~' 
      v   v                                |            
     RaftLogStorage                        |            
           o                               |            
           |                               |            
           |                               |            
           v                               v            
       local-disk                     local-disk        
 -----------------------------------------------                     -----------------------------------------------
 Node 1                                                              Node 2

Legends:

.~~~~~~~~~~~~~~.
! "tokio task" !
'~~~~~~~~~~~~~~'

o--> function call
---> async communication: via channel or RPC
```
