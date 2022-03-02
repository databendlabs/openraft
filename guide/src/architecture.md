# Architecture

```bob

        .~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~.
        !          User                   !
        !          o                      !
        !          |                      !
        !          |                      !
        !          | "client_write(impl AppData) -> impl AppDataResponse"
        !          | "is_leader()"        !
        !          | "add_learner()"      !
        !          | "change_membership()"!
        !          v                      !
        !          Raft                   !             .-------------> Raft -----.
        '~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~'             |                         |
                   |                                    |                         |
                   | enum RaftMes                       |                         |
                   |                                    |                         |
           .~~~~~~~|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~.    |                         |
           !       v                               !    | RPC:                    v
.----------------o RaftCore                        !    |   "vote()"              RaftCore
|          !          o                            !    |   "append_entries()"
|          !       .--+--------.                   !    |   "install_snapshot()"
|          !       v           v                   !    |
|          ! ReplicationState  ReplicationState    !    |
|          '~~|~~~~~~~~~~~~~~~~|~~~~~~~~~~~~~~~~~~~'    |
|             |                |                        |
|             |                |                        |
|  .~~~~~~~~~~|~~~~~~~~~~.  .~~|~~~~~~~~~~~~~~~~~~~.    |
|  !          v          !  !  v                   !    |
|  ! ReplicationStream   !  !  ReplicationStream   !    |
|  ! o                   !  !  o  o                !    |
|  '~|~~~~~~~~~~~~~~~~~~~'  !  |  |                !    |
|    |                      !  |  v                !    |
|    |                      !  |  Arc<RaftNetwork> -----'
|    |                      '~~|~~~~~~~~~~~~~~~~~~~'
|    |                         |
|    `-------------------------+
|                              | "get_log()"
|                              | "..."
|                              v
`----------------------> Arc<RaftStorage>
   "append_log()"              o
   "..."                       |
                               v
                           local-disk


 -----------------------------------------------              -----------------------------------------------
 Node 1                                                       Node 2

Legends:

.~~~~~~~~~~~~~~.
! "tokio task" !
'~~~~~~~~~~~~~~'

o--> function call
---> async communication: through channel or RPC
```
