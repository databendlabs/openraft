# Architecture


```

        ...................................
        .          User                   .
        .          ‖                      .
        .          ‖                      .
        .          ‖                      .
        .          ‖ client_write(impl AppData) -> impl AppDataResponse
        .          ‖ client_read()        .
        .          ‖ add_non_voter()      .
        .          ‖ change_member()      .
        .          v                      .
        .          Raft                   .             .-------------> Raft -----.
        ...................................             |                         |
                   |                                    |                         |
                   | enum RaftMes                       |                         |
                   |                                    |                         |
           ........|................................    |                         |
           .       v                               .    | RPC:                    v
.================= RaftCore                        .    |   vote()                RaftCore
‖          .       .==+========.                   .    |   append_entries()      
‖          .       v           v                   .    |   install_snapshot()
‖          . ReplicationState  ReplicationState    .    |   
‖          ...|................|....................    |
‖             |                |                        |
‖             |                |                        |
‖  ...........|...........  ...|....................    |
‖  .          v          .  .  v                   .    |
‖  . ReplicationStream   .  .  ReplicationStream   .    |
‖  . ‖                   .  .  ‖  ‖                .    |
‖  ..‖....................  .  ‖  ‖                .    |
‖    ‖                      .  ‖  v                .    |
‖    ‖                      .  ‖  Arc<RaftNetwork> -----'
‖    ‖                      ...‖....................
‖    ‖                         ‖
‖    `=========================+ 
‖                              ‖ get_log()
‖                              ‖ ...
‖                              v
`======================> Arc<RaftStorage>
   append_log()                ‖
   ...                         ‖
                               v
                           local-disk
                                
 
 -----------------------------------------------              ----------------------------------------------- 
 Node 1                                                       Node 2                                          

```

Legends:

```
..............
. tokio task .
..............

=> function call
-> async communication: through channel or RPC
```
