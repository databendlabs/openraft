raft essentials
===============
This document presents the essential details of the Raft protocol as an aid for protocol implementors. As such, this document assumes at least moderate familiarity with the [Raft spec](https://raft.github.io/raft.pdf) and includes references to pertinent sections of the spec.

## definitions
- `server`: a process running an implementation of the Raft protocol.
- `cluster`: a group of servers working together to implement the requirements of the Raft spec.
- `committed` (log): a log entry is committed once the leader that created the entry has replicated it on a majority of the servers.
- `applied` (log): a log entry is applied after it has been committed and then the leader applies it to its own state machine. Successive AppendEntries RPCs will include the index of the last applied log and all followers will apply logs to their state machine up through that index.
- `election timeout`: a randomized period of time which is reset with a new randomized value after every AppendEntries and RequestVote RPC is received (unless they are rejected). If the time elapses, the server will start a new election.
- `election timeout minimum`: the minimum period of time which a server must wait before starting an election. For compliance, all servers should be configured with the same minimum value. The randomized election timeout must always be greater than this value.

## basics
- a distinguished leader is elected for the cluster, and is then given complete responsibility for managing the replicated log. §5.0
- the leader accepts log entries from clients, replicates them on other servers, and tells servers when it is safe to apply log entries to their state machines. §5.0
- data always flows from leader to followers, never any other direction. §5.0
- when a leader goes offline, a new leader is elected. §5.0
- at any given time each server is in one of four states: leader (§5.0), candidate (§5.0), follower (§5.0), or learner (§6).

## terms
- time is divided into terms of arbitrary length. §5.1
- each server stores a current term number, which increases monotonically over time. 5.1
- each term begins with an election, in which one or more candidates attempt to become leader. §5.1
- if a candidate wins the election, then it serves as leader for the rest of the term. §5.1
- in some situations an election will result in a split vote. In this case the term will end with no leader; a new term (with a new election) will begin shortly. Raft ensures that there is at most one leader in a given term. §5.1

## server roles
- when servers start up, they begin as followers; and remain as followers as long as they receive valid RPCs from a leader or candidate. §5.2
- if a server receives a request with a stale term number, it rejects the request. §5.1
- current terms are exchanged whenever servers communicate; if one server's current term is smaller than the other's, then it updates its current term to the larger value. §5.1
- to avoid issues where removed servers may disrupt the cluster by starting new elections, servers disregard RequestVote RPCs when they believe a current leader exists; specifically, if a server receives a RequestVote RPC within the election timeout minimum of hearing from a current leader, it does not update its term or grant its vote. §6
- if a leader or candidate discovers that its term is stale (via an RPC from a peer), it immediately reverts to follower state. §5.1
- a voter must deny its vote if its own log is more up-to-date than that of the candidate; if the logs have last entries with different terms, then the log with the later term is more up-to-date; if the logs end with the same term, then whichever log is longer is more up-to-date. §5.4.1

### follower
Follower servers simply respond to requests from leaders and candidates. §5.1

- a new randomized election timeout is started after each valid RPC received from the leader or candidates. §5.2
- if a follower receives no communication during the election timeout window, then it assumes there is no viable leader, transitions to the candidate state, and begins campaigning to become the new leader. §5.2

### learner
learners are the same as followers, except are not considered for votes, and do not have election timeouts. §6

### candidate
Candidate servers campaign to become the cluster leader using the RequestVote RPC. §5.1

- the first action a candidate takes is to increment the current term. §5.2
- it then starts a new randomized election timeout within the configured range. §5.2
- it then votes for itself and issues RequestVote RPCs in parallel to each of the other servers in the cluster. §5.2
- a candidate continues in this state until one of three things happens: (a) it wins the election, (b) another server establishes itself as leader, or (c) a period of time goes by with no winner. §5.2

#### winning elections
- a candidate wins an election if it receives votes from a majority of the servers in the cluster for the same term. §5.2
- each server will vote for at most one candidate in a given term, on a first-come-first-served basis. §5.2
- once a candidate wins an election, it becomes leader. §5.2

#### loosing elections
- while waiting for votes, a candidate may receive an AppendEntries RPC from another server claiming to be leader. §5.2
- if the leader's term (included in its RPC) is at least as large as the candidate's current term, then the candidate recognizes the leader as legitimate and returns to follower state. §5.2
- if the term in the RPC is smaller than the candidate's current term, then the candidate rejects the RPC and continues in candidate state. §5.2

#### stalemate elections (split vote)
- if the candidate times out before receiving enough votes, and before another leader comes to power, start a new election by incrementing the term and initiating another round of RequestVote RPCs.

#### receiver implementation: RequestVote RPC
1. Reply false if term < currentTerm (§5.1) or if a heartbeat was received from the leader within the election timeout minimum window (§6).
2. If votedFor is null or candidateId, and candidate's log is at least as up-to-date as receiver's log, grant vote (§5.2, §5.4).

### leader
The cluster leader handles all client requests and client requests must be redirected to the leader if received by another node (§5.1).

- a leader commits a blank no-op entry into the log at the start of its term. §8
- must send periodic heartbeats (empty AppendEntries RPCs) to all followers in order to maintain their authority and prevent new elections. §5.2
- must service client requests. Each client request contains a command to be executed by the replicated state machines; the leader appends the command to its log as a new entry, then issues AppendEntries RPCs in parallel to each of the other servers to replicate the entry. §5.3
- once an entry has been committed, the leader applies the entry to its state machine and returns the result of that execution to the client. §5.3
- if followers crash or run slowly, or if network packets are lost, the leader retries AppendEntries RPCs indefinitely (even after it has responded to the client) until all followers eventually store all log entries (§5.3); AppendEntries RPCs should be issued in parallel for best performance; AppendEntries RPCs are idempotent (§5.1).
- the leader keeps track of the highest index it knows to be committed, and it includes that index in future AppendEntries RPCs (including heartbeats) so that the other servers eventually find out. §5.3
- once a follower learns that a log entry is committed, it applies the entry to its local state machine (in log order). §5.3
- a leader never overwrites or deletes entries in its own log. §5.3

#### replication
- a leader creates at most one entry with a given log index in a given term, and log entries never change their position in the log. §5.3
- when sending an AppendEntries RPC, the leader includes the index and term of the entry in its log that immediately precedes the new entries. If the follower does not find an entry in its log with the same index and term, then it refuses the new entries. §5.3
- the leader handles inconsistencies by forcing the followers' logs to duplicate its own. This means that conflicting entries in follower logs will be overwritten with entries from the leader's log (which will only ever happen to uncommitted log entries). §5.3
- to bring a follower's log into consistency with its own, the leader must find the latest log entry where the two logs agree, delete any entries in the follower's log after that point, and send the follower all of the leader's entries after that point. §5.3
- all of these actions happen in response to the consistency check performed by AppendEntries RPCs. The leader maintains a nextIndex for each follower, which is the index of the next log entry the leader will send to that follower. §5.3
- when a leader first comes to power, it initializes all nextIndex values for all cluster peers to the index just after the last one in its log. §5.3
- if a follower's log is inconsistent with the leader's, the AppendEntries consistency check will fail in the next AppendEntries RPC to that follower. After a rejection, the leader decrements nextIndex and retries the AppendEntries RPC. Eventually nextIndex will reach a point where the leader and follower logs match. When this happens, AppendEntries will succeed, which removes any conflicting entries in the follower's log and appends entries from the leader's log (if any). Once AppendEntries succeeds, the follower's log is consistent with the leader's, and it will remain that way for the rest of the term. §5.3

#### log consistency check optimization
The follower can include the term of the conflicting entry and the first index it stores for that term. With this information, the leader can decrement nextIndex to bypass all of the conflicting entries in that term; one AppendEntries RPC will be required for each term with conflicting entries, rather than one RPC per entry. §5.3

##### receiver implementation: AppendEntries RPC
1. Reply false if term < currentTerm (§5.1).
2. Reply false if log doesn't contain an entry at prevLogIndex whose term matches prevLogTerm (§5.3) Use the log consistency check optimization when possible.
3. If an existing entry conflicts with a new one (same index but different terms), delete the existing entry and all that follow it (§5.3).
4. Append any new entries not already in the log.
5. If leaderCommit > commitIndex, set commitIndex to min(leaderCommit, index of last new entry).

## configuration changes
In order to ensure safety, configuration changes must use a two-phase approach, called `joint consensus`. §6

- once a server adds a new configuration entry to its log, it uses that configuration for all future decisions (a server always uses the latest configuration in its log, regardless of whether the entry is committed). §6
- when joint consensus is entered, log entries are replicated to all servers in both configurations. §6
- any server from either configuration may serve as leader. §6
- agreement (for elections and entry commitment) requires separate majorities from both the old and new configurations. §6
- in order to avoid availability gaps, Raft introduces an additional phase before the configuration change, in which the new servers join the cluster as non-voting members (the leader replicates log entries to them, but they are not considered for majorities). Once the new servers have caught up with the rest of the cluster, the reconfiguration can proceed as described above. §6
- in the case where the cluster leader is not part of the new configuration, the leader steps down (returns to follower state) once it has committed the log entry of the new configuration. This means that there will be a period of time (while it is committing the new configuration) when the leader is managing a cluster that does not include itself; it replicates log entries but does not count itself in majorities. The leader transition occurs when the new configuration is committed because this is the first point when the new configuration can operate independently (it will always be possible to choose a leader from the new configuration). Before this point, it may be the case that only a server from the old configuration can be elected leader. §6

## log compaction | snapshotting
In log compaction, the entire system state is written to a snapshot on stable storage, then the entire log up to that point is discarded.

- each server takes snapshots independently, covering just the committed entries in its log. §7
- most of the work consists of the state machine writing its current state to the snapshot. §7
- a small amount of metadata is included in the snapshot: the last included index is the index of the last entry in the log that the snapshot replaces (the last entry the state machine had applied), and the last included term is the term of this entry. §7
- to enable cluster membership changes (§6), the snapshot also includes the latest configuration in the log as of last included index. §7
- once a server completes writing a snapshot, it may delete all log entries up through the last included index, as well as any prior snapshot. §7
- the last index and term included in the snapshot must still be identifiable in the log to support the log consistency check. Practically, this may just be a special type of log entry which points to the snapshot and holds the pertinent metadata of the snapshot including the index and term. §7

### leader to follower snapshots
Although servers normally take snapshots independently, the leader must occasionally send snapshots to followers that lag behind.

- the cluster leader uses the InstallSnapshot RPC for sending snapshots to followers. §7
- this happens when the leader has already discarded the next log entry that it needs to send to a follower (due to its own compaction process). §7
- a follower that has kept up with the leader would already have this entry; however, an exceptionally slow follower or a new server joining the cluster (§6) would not. The way to bring such a follower up-to-date is for the leader to send it a snapshot over the network. §7
- when a follower receives a snapshot with this RPC, it must decide what to do with its existing log entries. Usually the snapshot will contain new information not already in the recipient's log. In this case, the follower discards its entire log; it is all superseded by the snapshot and may possibly have uncommitted entries that conflict with the snapshot. §7
- if instead the follower receives a snapshot that describes a prefix of its log (due to retransmission or by mistake), then log entries covered by the snapshot are deleted but entries following the snapshot are still valid and must be retained. §7

### receiver implementation: InstallSnapshot RPC
1. Reply immediately if term < currentTerm.
2. Create new snapshot file if first chunk (offset is 0).
3. Write data into snapshot file at given offset.
4. Reply and wait for more data chunks if done is false.
5. Save snapshot file, discard any existing or partial snapshot with a smaller index.
6. If existing log entry has same index and term as snapshot's last included entry, retain log entries following it and reply.
7. Discard the entire log.
8. Reset state machine using snapshot contents (and load snapshot's cluster configuration).

## client interaction
Clients of Raft send all of their requests to the leader.

- when a client first starts up, it connects to a randomly-chosen server. §8
- if the client's first choice is not the leader, that server will reject the client's request and supply information about the most recent leader it has heard from. §8
- if the leader crashes, client requests will timeout; clients then try again with randomly-chosen servers. §8

### request forwarding
A non-spec pattern may be used to satisfy the requirements of the spec: request forwarding. Non-leader servers may forward client requests to the leader instead of rejecting the request. If the leader is not currently known, due to being in an election, the request may be buffered for some period of time to wait for a new leader to be elected. Worst case scenario, the request times out, and the client needs to send the request again.

### linearizability
#### writes
Our goal for Raft is to implement linearizable semantics. If the leader crashes after committing a log entry but before responding to the client, the client may retry the command with a new leader, causing it to be executed a second time.

The solution is for clients to assign unique serial numbers to every command. Then, the state machine tracks the latest serial number processed for each client, along with the associated response. If it receives a command whose serial number has already been executed, it responds immediately without reexecuting the request. §8

Note well: this is an application specific requirement, and must be implemented on the application-level.

#### reads
Linearizable reads must not return stale data.

- a leader commits a blank no-op entry into the log at the start of its term. §8
- a leader must check whether it has been deposed before processing a read-only request; raft handles this by having the leader exchange heartbeat messages with a majority of the cluster before responding to read-only requests. §8

---

## faq
### can the log index reset per term?
The Raft spec states explicitly that the index is a monotonically increasing value, and makes no mention of a potential reset per term. Moreover, resetting per term would cause problems with other requirements of the spec. Namely that the leader would now need to keep track of both the next term and the next index to be replicated to followers.

Sticking with a uniform, monotonically increasing index value, which does not reset per term, is the recommended approach.

### does the definition of commitment include the leader itself?
The leader is included in the calculation of majorities, except in the case of joint consensus for the specific case where the leader is not part of the new configuration.
