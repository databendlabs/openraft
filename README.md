<div align="center">
    <h1>OpenRaft</h1>
    <h4>
        Advanced <a href="https://raft.github.io/">Raft</a> in 🦀 Rust using <a href="https://tokio.rs/">Tokio</a>. Please ⭐ on <a href="https://github.com/databendlabs/openraft">GitHub</a>!
    </h4>


[![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft)
[![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft)
[![DeepWiki](https://img.shields.io/badge/DeepWiki-openraft-blue.svg?logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACwAAAAyCAYAAAAnWDnqAAAAAXNSR0IArs4c6QAAA05JREFUaEPtmUtyEzEQhtWTQyQLHNak2AB7ZnyXZMEjXMGeK/AIi+QuHrMnbChYY7MIh8g01fJoopFb0uhhEqqcbWTp06/uv1saEDv4O3n3dV60RfP947Mm9/SQc0ICFQgzfc4CYZoTPAswgSJCCUJUnAAoRHOAUOcATwbmVLWdGoH//PB8mnKqScAhsD0kYP3j/Yt5LPQe2KvcXmGvRHcDnpxfL2zOYJ1mFwrryWTz0advv1Ut4CJgf5uhDuDj5eUcAUoahrdY/56ebRWeraTjMt/00Sh3UDtjgHtQNHwcRGOC98BJEAEymycmYcWwOprTgcB6VZ5JK5TAJ+fXGLBm3FDAmn6oPPjR4rKCAoJCal2eAiQp2x0vxTPB3ALO2CRkwmDy5WohzBDwSEFKRwPbknEggCPB/imwrycgxX2NzoMCHhPkDwqYMr9tRcP5qNrMZHkVnOjRMWwLCcr8ohBVb1OMjxLwGCvjTikrsBOiA6fNyCrm8V1rP93iVPpwaE+gO0SsWmPiXB+jikdf6SizrT5qKasx5j8ABbHpFTx+vFXp9EnYQmLx02h1QTTrl6eDqxLnGjporxl3NL3agEvXdT0WmEost648sQOYAeJS9Q7bfUVoMGnjo4AZdUMQku50McDcMWcBPvr0SzbTAFDfvJqwLzgxwATnCgnp4wDl6Aa+Ax283gghmj+vj7feE2KBBRMW3FzOpLOADl0Isb5587h/U4gGvkt5v60Z1VLG8BhYjbzRwyQZemwAd6cCR5/XFWLYZRIMpX39AR0tjaGGiGzLVyhse5C9RKC6ai42ppWPKiBagOvaYk8lO7DajerabOZP46Lby5wKjw1HCRx7p9sVMOWGzb/vA1hwiWc6jm3MvQDTogQkiqIhJV0nBQBTU+3okKCFDy9WwferkHjtxib7t3xIUQtHxnIwtx4mpg26/HfwVNVDb4oI9RHmx5WGelRVlrtiw43zboCLaxv46AZeB3IlTkwouebTr1y2NjSpHz68WNFjHvupy3q8TFn3Hos2IAk4Ju5dCo8B3wP7VPr/FGaKiG+T+v+TQqIrOqMTL1VdWV1DdmcbO8KXBz6esmYWYKPwDL5b5FA1a0hwapHiom0r/cKaoqr+27/XcrS5UwSMbQAAAABJRU5ErkJggg==)](https://deepwiki.com/databendlabs/openraft)
[![guides](https://img.shields.io/badge/guide-%E2%86%97-brightgreen)](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/index.html)
[![Discord Chat](https://img.shields.io/discord/1015845055434588200?logo=discord)](https://discord.gg/ZKw3WG7FQ9)
<br/>
[![CI](https://github.com/databendlabs/openraft/actions/workflows/ci.yaml/badge.svg)](https://github.com/databendlabs/openraft/actions/workflows/ci.yaml)
[![Coverage Status](https://coveralls.io/repos/github/databendlabs/openraft/badge.svg?branch=main)](https://coveralls.io/github/databendlabs/openraft?branch=main)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)
![Crates.io](https://img.shields.io/crates/d/openraft.svg)
![Crates.io](https://img.shields.io/crates/dv/openraft.svg)

</div>

This project intends to improve raft as the next-generation consensus protocol for distributed data storage systems (SQL, NoSQL, KV, Streaming, Graph ... or maybe something more exotic).

Currently, openraft is the consensus engine of meta-service cluster in [databend](https://github.com/databendlabs/databend).


- 🚀 **Get started**:
    - [OpenRaft guide](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/getting_started/index.html) is the best place to get started,
    - [OpenRaft docs](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/index.html) for more in-depth details,
    - [OpenRaft FAQ](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/faq/index.html) explains some common questions.
    - [OpenRaft on DeepWiki](https://deepwiki.com/databendlabs/openraft) provides detailed architectural documentation to help understand OpenRaft internals.

- 💡 **Example Applications**:

    - [Examples with OpenRaft 0.10](https://github.com/databendlabs/openraft/tree/release-0.10/examples) require OpenRaft `0.10.0-alpha.29` (alpha) on [crates.io/openraft](https://crates.io/crates/openraft);
    - [Examples with OpenRaft 0.9](https://github.com/databendlabs/openraft/tree/release-0.9/examples) require OpenRaft 0.9 on [crates.io/openraft](https://crates.io/crates/openraft).

- 🙌 **Questions**?
    - Why not take a peek at our [FAQ](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/faq/index.html)? You might find just what you need.
    - Wanna chat? Come hang out with us on [Discord](https://discord.gg/ZKw3WG7FQ9)!
    - Or start a new discussion over on [GitHub](https://github.com/databendlabs/openraft/discussions/new).
    - Or join our [Feishu group](https://applink.feishu.cn/client/chat/chatter/add_by_link?link_token=d20l9084-6d36-4470-bac5-4bad7378d003).
    - And hey, if you're on WeChat, add us: `drmingdrmer`. Let's get the conversation started!

Whatever your style, we're here to support you. 🚀 Let's make something awesome together!

- OpenRaft is derived from [async-raft](https://docs.rs/crate/async-raft/latest) with several bugs fixed: [Fixed bugs](https://github.com/databendlabs/openraft/blob/main/derived-from-async-raft.md).


# Status

- The features are almost complete for building an application.
- Performance: Supports 33,000 writes/sec for a single writer and 5,615,000 writes/sec with batch writes. See: [Performance](#performance)
- Unit test coverage stands at 92%.
- The chaos test has not yet been completed, and further testing is needed to ensure the application's robustness and reliability.


## API status

- **OpenRaft API is not stable yet**. Before `1.0.0`, an upgrade may contain incompatible changes.
  Check our [change-log](https://github.com/databendlabs/openraft/blob/main/change-log.md). A commit message starts with a keyword to indicate the modification type of the commit:

  - `DataChange:` on-disk data types changes, which may require manual upgrade.
  - `Change:` if it introduces incompatible changes.
  - `Feature:` if it introduces compatible non-breaking new features.
  - `Fix:` if it just fixes a bug.

## Versions

- **Branch main** has been under active development.
    The main branch is for the [release-0.10](https://github.com/databendlabs/openraft/tree/release-0.10).
    Latest alpha on crates.io: [v0.10.0-alpha.29](https://crates.io/crates/openraft/0.10.0-alpha.29).
    The `0.10` line is in **alpha** — the API may still change before the final `0.10.0` release.

- **Branch [release-0.9](https://github.com/databendlabs/openraft/tree/release-0.9)**:
  Latest: ( [v0.9.0](https://github.com/databendlabs/openraft/tree/v0.9.0) | [Change log](https://github.com/databendlabs/openraft/blob/release-0.9/change-log.md#v090) );
  Upgrade guide: ⬆️  [0.8 to 0.9](https://docs.rs/openraft/0.9.0/openraft/docs/upgrade_guide/upgrade_08_09/index.html);
  `release-0.9` **Won't** accept new features but only bug fixes.

- **Branch [release-0.8](https://github.com/databendlabs/openraft/tree/release-0.8)**:
  Latest: ( [v0.8.8](https://github.com/databendlabs/openraft/tree/v0.8.8) | [Change log](https://github.com/databendlabs/openraft/blob/release-0.8/change-log.md#v088) );
  Upgrade guide: ⬆️  [0.7 to 0.8](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_07_08/index.html), ⬆️  [0.8.3 to 0.8.4](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_083_084/index.html);
  `release-0.8` **Won't** accept new features but only bug fixes.

- **Branch [release-0.7](https://github.com/databendlabs/openraft/tree/release-0.7)**:
  Latest: ( [v0.7.6](https://github.com/databendlabs/openraft/tree/v0.7.6) | [Change log](https://github.com/databendlabs/openraft/blob/release-0.7/change-log.md#v076) );
  Upgrade guide: ⬆️  [0.6 to 0.7](https://docs.rs/openraft/0.8.4/openraft/docs/upgrade_guide/upgrade_06_07/index.html);
  `release-0.7` **Won't** accept new features but only bug fixes.

- **Branch [release-0.6](https://github.com/databendlabs/openraft/tree/release-0.6)**:
  Latest: ( [v0.6.8](https://github.com/databendlabs/openraft/tree/v0.6.8) | [Change log](https://github.com/databendlabs/openraft/blob/release-0.6/change-log.md) );
  `release-0.6` **won't** accept new features but only bug fixes.

# Roadmap

- [x] **2022-10-31** [Extended joint membership](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/data/extended_membership/index.html)
- [x] **2023-02-14** Minimize confliction rate when electing;
  See: [OpenRaft Vote design](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/data/vote/index.html);
  Or use [standard Raft leader-ID mode](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/data/leader_id/index.html).
- [x] **2023-04-26** Goal performance is 1,000,000 put/sec.
- [ ] Reduce the complexity of vote and pre-vote: [get rid of pre-vote RPC](https://github.com/databendlabs/openraft/discussions/15);
- [ ] Support flexible quorum, e.g.: [Hierarchical Quorums](https://zookeeper.apache.org/doc/r3.5.9/zookeeperHierarchicalQuorums.html)
- [ ] Consider introducing read-quorum and write-quorum,
  improve efficiency with a cluster with an even number of nodes.


<!--
   - - [ ] Consider separating log storage and log order storage.
   -   Leader only determines and replicates the index of log entries, not log
   -   payload.
      -->

# Performance

The benchmark is focused on the OpenRaft framework itself and is run on a
minimized store and network. This is **NOT a real world** application benchmark!!!

Single writes:

| clients | put/s     |
| --:     | --:       |
| 4096    | 3,548,000 |
| 1024    | 3,006,000 |
| 256     | 1,808,000 |
| 64      |   912,000 |
| 1       |    33,000 |

Batch writes (4 entries per batch):

| clients | put/s     |
| --:     | --:       |
| 4096    | 5,615,000 |

For benchmark detail, go to the [./benchmarks/minimal](./benchmarks/minimal) folder.

# Features

- **Async and Event-Driven**: Operates based on Raft events without reliance on periodic ticks, optimizing message batching for high throughput.
- **Extensible Storage and Networking**: Customizable via `RaftLogStorage`, `RaftStateMachine` and `RaftNetworkV2` traits, allowing flexibility in choosing storage and network solutions.
- **Unified Raft API**: Offers a single `Raft` type for creating and interacting with Raft tasks, with a straightforward API.
- **Cluster Formation**: Provides strategies for initial cluster setup as detailed in the [cluster formation guide](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/cluster_control/cluster_formation/index.html).
- **Built-In Tracing Instrumentation**: The codebase integrates [tracing](https://docs.rs/tracing/) for logging and distributed tracing, with the option to [set verbosity levels at compile time](https://docs.rs/tracing/latest/tracing/level_filters/index.html).

## Functionality:

- ✅ **Leader election**: by policy or manually ([`Trigger::elect()`][]).
- ✅ **Leader transfer**: [`Trigger::transfer_leader()`][].
- ✅ **Pre-vote**: avoid unnecessary term increments by enabling [`Config::enable_pre_vote`][].
- ✅ **Non-voter(learner) Role**: refer to [`add_learner()`][].
- ✅ **Log Compaction**(snapshot of state machine): by policy or manually ([`Trigger::snapshot()`][]).
- ✅ **Snapshot replication**.
- ✅ **Dynamic Membership**: using joint membership config change. Refer to [dynamic membership](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/cluster_control/dynamic_membership/index.html)
- ✅ **Linearizable read**: [`ensure_linearizable()`][].
- ✅ **Metrics**: [`Raft::metrics()`][], [`Raft::data_metrics()`][], and [`Raft::server_metrics()`][].
- ⛔️ **Won't support**: Single-step config change. Single-step membership change is a restricted subset of joint consensus that only allows changing one node at a time. Openraft uses the more general [joint consensus](https://docs.rs/openraft/0.10.0-alpha.29/openraft/docs/cluster_control/dynamic_membership/index.html) approach which supports arbitrary membership changes in a single operation.
- ✅ Toggle heartbeat / election: [`RuntimeConfigHandle::heartbeat()`][] / [`RuntimeConfigHandle::elect()`][].
- ✅ Trigger snapshot / election manually: [`Trigger::snapshot()`][] / [`Trigger::elect()`][].
- ✅ Purge log by policy or manually: [`Trigger::purge_log()`][].


# Who uses it

<!-- Star counts as of 2026-07; ordered by stars, Databend first. Comments are invisible on rendered GitHub. -->
- [Databend](https://github.com/databendlabs/databend) - The Next-Gen Cloud [Data+AI] Analytics <!-- ★9378 -->
- [Walrus](https://github.com/nubskr/walrus) - Distributed message streaming engine with Raft-based metadata coordination. <!-- ★1900 -->
- [CnosDB](https://github.com/cnosdb/cnosdb) - A cloud-native open source distributed time series database. <!-- ★1752 -->
- [RobustMQ](https://github.com/robustmq/robustmq) - Next generation cloud-native converged message queue. <!-- ★1636 -->
- [RocketMQ-rust](https://github.com/mxsm/rocketmq-rust) - Apache RocketMQ reimplemented in Rust. <!-- ★1495 -->
- [Hiqlite](https://github.com/sebadob/hiqlite) - Highly-available, embeddable, Raft-based SQLite with caching. <!-- ★460 -->
- [Octopii](https://github.com/octopii-rs/octopii) - A distributed systems kernel for building replicated, fault-tolerant services. <!-- ★446 -->
- [Renegade](https://github.com/renegade-fi/renegade) - On-chain dark pool for anonymous trading. <!-- ★252 -->
- [Helyim](https://github.com/helyim/helyim) - [SeaweedFS](https://github.com/seaweedfs/seaweedfs) implemented in pure Rust. <!-- ★209 -->
- [Ahnlich](https://github.com/deven96/ahnlich) - In-memory vector database with an AI proxy. <!-- ★204 -->
- [raymondshe/matchengine-raft](https://github.com/raymondshe/matchengine-raft) - A example to demonstrate how openraft persists snapshots/logs to disk. <!-- ★56 -->
- [tsoracle](https://github.com/prisma-risk/tsoracle) - Distributed timestamp and sequence oracle serving strictly monotonic, gapless IDs. <!-- ★55 -->
- [yuyang0/rrqlite](https://github.com/yuyang0/rrqlite) - A rust implementation of [rqlite](https://github.com/rqlite/rqlite). <!-- ★21 -->

📣 Using openraft in your project? We'd love to feature it here — [open an issue](https://github.com/databendlabs/openraft/issues/new) to let us know, or skip the wait and [submit a pull request directly](https://github.com/databendlabs/openraft/edit/main/README.md).

# Contributing

Check out the [CONTRIBUTING.md](https://github.com/databendlabs/openraft/blob/main/CONTRIBUTING.md)
guide for more details on getting started with contributing to this project.

## Contributors

Thanks to everyone who contributes to OpenRaft:

<!-- Regenerate: scripts/contributors/list_contributors.py (writes
     assets/contributors/{contributors.txt,contributors.md}), then
     scripts/contributors/generate_contributor_svgs.py (writes assets/contributors/svg/) -->
[<img src="assets/contributors/svg/ariesdevil.svg" alt="@ariesdevil"/>](https://github.com/ariesdevil)
[<img src="assets/contributors/svg/AustenSchunk.svg" alt="@AustenSchunk"/>](https://github.com/AustenSchunk)
[<img src="assets/contributors/svg/bohutang.svg" alt="@bohutang"/>](https://github.com/bohutang)
[<img src="assets/contributors/svg/byronwasti.svg" alt="@byronwasti"/>](https://github.com/byronwasti)
[<img src="assets/contributors/svg/caicancai.svg" alt="@caicancai"/>](https://github.com/caicancai)
[<img src="assets/contributors/svg/CentRa-Linux.svg" alt="@CentRa-Linux"/>](https://github.com/CentRa-Linux)
[<img src="assets/contributors/svg/claude.svg" alt="@claude"/>](https://github.com/claude)
[<img src="assets/contributors/svg/cliff0412.svg" alt="@cliff0412"/>](https://github.com/cliff0412)
[<img src="assets/contributors/svg/ClSlaid.svg" alt="@ClSlaid"/>](https://github.com/ClSlaid)
[<img src="assets/contributors/svg/danthegoodman1.svg" alt="@danthegoodman1"/>](https://github.com/danthegoodman1)
[<img src="assets/contributors/svg/devillove084.svg" alt="@devillove084"/>](https://github.com/devillove084)
[<img src="assets/contributors/svg/Diggsey.svg" alt="@Diggsey"/>](https://github.com/Diggsey)
[<img src="assets/contributors/svg/diptanu.svg" alt="@diptanu"/>](https://github.com/diptanu)
[<img src="assets/contributors/svg/drmingdrmer.svg" alt="@drmingdrmer"/>](https://github.com/drmingdrmer)
[<img src="assets/contributors/svg/eliasyaoyc.svg" alt="@eliasyaoyc"/>](https://github.com/eliasyaoyc)
[<img src="assets/contributors/svg/emmanuel-ferdman.svg" alt="@emmanuel-ferdman"/>](https://github.com/emmanuel-ferdman)
[<img src="assets/contributors/svg/emon100.svg" alt="@emon100"/>](https://github.com/emon100)
[<img src="assets/contributors/svg/ethe.svg" alt="@ethe"/>](https://github.com/ethe)
[<img src="assets/contributors/svg/evanj.svg" alt="@evanj"/>](https://github.com/evanj)
[<img src="assets/contributors/svg/George-Miao.svg" alt="@George-Miao"/>](https://github.com/George-Miao)
[<img src="assets/contributors/svg/getong.svg" alt="@getong"/>](https://github.com/getong)
[<img src="assets/contributors/svg/GrapeBaBa.svg" alt="@GrapeBaBa"/>](https://github.com/GrapeBaBa)
[<img src="assets/contributors/svg/gtema.svg" alt="@gtema"/>](https://github.com/gtema)
[<img src="assets/contributors/svg/guojidan.svg" alt="@guojidan"/>](https://github.com/guojidan)
[<img src="assets/contributors/svg/hadronzoo.svg" alt="@hadronzoo"/>](https://github.com/hadronzoo)
[<img src="assets/contributors/svg/HaHa421.svg" alt="@HaHa421"/>](https://github.com/HaHa421)
[<img src="assets/contributors/svg/homa31.svg" alt="@homa31"/>](https://github.com/homa31)
[<img src="assets/contributors/svg/iamazy.svg" alt="@iamazy"/>](https://github.com/iamazy)
[<img src="assets/contributors/svg/iMashtak.svg" alt="@iMashtak"/>](https://github.com/iMashtak)
[<img src="assets/contributors/svg/Jason5Lee.svg" alt="@Jason5Lee"/>](https://github.com/Jason5Lee)
[<img src="assets/contributors/svg/jdockerty.svg" alt="@jdockerty"/>](https://github.com/jdockerty)
[<img src="assets/contributors/svg/jgrund.svg" alt="@jgrund"/>](https://github.com/jgrund)
[<img src="assets/contributors/svg/jkosh44.svg" alt="@jkosh44"/>](https://github.com/jkosh44)
[<img src="assets/contributors/svg/josedab.svg" alt="@josedab"/>](https://github.com/josedab)
[<img src="assets/contributors/svg/justinlatimer.svg" alt="@justinlatimer"/>](https://github.com/justinlatimer)
[<img src="assets/contributors/svg/kper.svg" alt="@kper"/>](https://github.com/kper)
[<img src="assets/contributors/svg/leiysky.svg" alt="@leiysky"/>](https://github.com/leiysky)
[<img src="assets/contributors/svg/Licenser.svg" alt="@Licenser"/>](https://github.com/Licenser)
[<img src="assets/contributors/svg/lichuang.svg" alt="@lichuang"/>](https://github.com/lichuang)
[<img src="assets/contributors/svg/linsinn.svg" alt="@linsinn"/>](https://github.com/linsinn)
[<img src="assets/contributors/svg/Lymah123.svg" alt="@Lymah123"/>](https://github.com/Lymah123)
[<img src="assets/contributors/svg/maackle.svg" alt="@maackle"/>](https://github.com/maackle)
[<img src="assets/contributors/svg/maciejkow.svg" alt="@maciejkow"/>](https://github.com/maciejkow)
[<img src="assets/contributors/svg/manuelarte.svg" alt="@manuelarte"/>](https://github.com/manuelarte)
[<img src="assets/contributors/svg/manuelgdlvh.svg" alt="@manuelgdlvh"/>](https://github.com/manuelgdlvh)
[<img src="assets/contributors/svg/MarinPostma.svg" alt="@MarinPostma"/>](https://github.com/MarinPostma)
[<img src="assets/contributors/svg/markus-behrens.svg" alt="@markus-behrens"/>](https://github.com/markus-behrens)
[<img src="assets/contributors/svg/marvin-j97.svg" alt="@marvin-j97"/>](https://github.com/marvin-j97)
[<img src="assets/contributors/svg/mengxuroblox.svg" alt="@mengxuroblox"/>](https://github.com/mengxuroblox)
[<img src="assets/contributors/svg/mfelsche.svg" alt="@mfelsche"/>](https://github.com/mfelsche)
[<img src="assets/contributors/svg/Miaxos.svg" alt="@Miaxos"/>](https://github.com/Miaxos)
[<img src="assets/contributors/svg/monoid.svg" alt="@monoid"/>](https://github.com/monoid)
[<img src="assets/contributors/svg/MrCroxx.svg" alt="@MrCroxx"/>](https://github.com/MrCroxx)
[<img src="assets/contributors/svg/nasuiyile.svg" alt="@nasuiyile"/>](https://github.com/nasuiyile)
[<img src="assets/contributors/svg/Nicholas-Ball.svg" alt="@Nicholas-Ball"/>](https://github.com/Nicholas-Ball)
[<img src="assets/contributors/svg/penghs520.svg" alt="@penghs520"/>](https://github.com/penghs520)
[<img src="assets/contributors/svg/PeterKnego.svg" alt="@PeterKnego"/>](https://github.com/PeterKnego)
[<img src="assets/contributors/svg/pfwang80s.svg" alt="@pfwang80s"/>](https://github.com/pfwang80s)
[<img src="assets/contributors/svg/Phoenix500526.svg" alt="@Phoenix500526"/>](https://github.com/Phoenix500526)
[<img src="assets/contributors/svg/pinylin.svg" alt="@pinylin"/>](https://github.com/pinylin)
[<img src="assets/contributors/svg/ppamorim.svg" alt="@ppamorim"/>](https://github.com/ppamorim)
[<img src="assets/contributors/svg/PsiACE.svg" alt="@PsiACE"/>](https://github.com/PsiACE)
[<img src="assets/contributors/svg/Qian-Cheng-nju.svg" alt="@Qian-Cheng-nju"/>](https://github.com/Qian-Cheng-nju)
[<img src="assets/contributors/svg/rvielma.svg" alt="@rvielma"/>](https://github.com/rvielma)
[<img src="assets/contributors/svg/sainad2222.svg" alt="@sainad2222"/>](https://github.com/sainad2222)
[<img src="assets/contributors/svg/schreter.svg" alt="@schreter"/>](https://github.com/schreter)
[<img src="assets/contributors/svg/SebastianThiebaud.svg" alt="@SebastianThiebaud"/>](https://github.com/SebastianThiebaud)
[<img src="assets/contributors/svg/shuoli84.svg" alt="@shuoli84"/>](https://github.com/shuoli84)
[<img src="assets/contributors/svg/socutes.svg" alt="@socutes"/>](https://github.com/socutes)
[<img src="assets/contributors/svg/sollhui.svg" alt="@sollhui"/>](https://github.com/sollhui)
[<img src="assets/contributors/svg/SteveLauC.svg" alt="@SteveLauC"/>](https://github.com/SteveLauC)
[<img src="assets/contributors/svg/sunli829.svg" alt="@sunli829"/>](https://github.com/sunli829)
[<img src="assets/contributors/svg/sunsided.svg" alt="@sunsided"/>](https://github.com/sunsided)
[<img src="assets/contributors/svg/svix-jplatte.svg" alt="@svix-jplatte"/>](https://github.com/svix-jplatte)
[<img src="assets/contributors/svg/tangwz.svg" alt="@tangwz"/>](https://github.com/tangwz)
[<img src="assets/contributors/svg/thedodd.svg" alt="@thedodd"/>](https://github.com/thedodd)
[<img src="assets/contributors/svg/tisonkun.svg" alt="@tisonkun"/>](https://github.com/tisonkun)
[<img src="assets/contributors/svg/tobodner.svg" alt="@tobodner"/>](https://github.com/tobodner)
[<img src="assets/contributors/svg/trungda.svg" alt="@trungda"/>](https://github.com/trungda)
[<img src="assets/contributors/svg/tvsfx.svg" alt="@tvsfx"/>](https://github.com/tvsfx)
[<img src="assets/contributors/svg/undecidedapollo.svg" alt="@undecidedapollo"/>](https://github.com/undecidedapollo)
[<img src="assets/contributors/svg/Veeupup.svg" alt="@Veeupup"/>](https://github.com/Veeupup)
[<img src="assets/contributors/svg/vigith.svg" alt="@vigith"/>](https://github.com/vigith)
[<img src="assets/contributors/svg/wvwwvwwv.svg" alt="@wvwwvwwv"/>](https://github.com/wvwwvwwv)
[<img src="assets/contributors/svg/xu-cheng.svg" alt="@xu-cheng"/>](https://github.com/xu-cheng)
[<img src="assets/contributors/svg/Xuanwo.svg" alt="@Xuanwo"/>](https://github.com/Xuanwo)
[<img src="assets/contributors/svg/YangKian.svg" alt="@YangKian"/>](https://github.com/YangKian)
[<img src="assets/contributors/svg/ygf11.svg" alt="@ygf11"/>](https://github.com/ygf11)
[<img src="assets/contributors/svg/youichi-uda.svg" alt="@youichi-uda"/>](https://github.com/youichi-uda)
[<img src="assets/contributors/svg/zach-schoenberger.svg" alt="@zach-schoenberger"/>](https://github.com/zach-schoenberger)
[<img src="assets/contributors/svg/zhixinwen.svg" alt="@zhixinwen"/>](https://github.com/zhixinwen)

# License

OpenRaft is licensed under the terms of the [MIT License](https://en.wikipedia.org/wiki/MIT_License#License_terms)
or the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0), at your choosing.


[`change_membership()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.change_membership
[`add_learner()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.add_learner
[`Trigger::purge_log()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/trigger/struct.Trigger.html#method.purge_log

[`RuntimeConfigHandle::heartbeat()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.RuntimeConfigHandle.html#method.heartbeat
[`RuntimeConfigHandle::elect()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.RuntimeConfigHandle.html#method.elect

[`Trigger::elect()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/trigger/struct.Trigger.html#method.elect
[`Trigger::snapshot()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/trigger/struct.Trigger.html#method.snapshot
[`Trigger::transfer_leader()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/trigger/struct.Trigger.html#method.transfer_leader
[`Config::enable_pre_vote`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/struct.Config.html#structfield.enable_pre_vote
[`Raft::metrics()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.metrics
[`Raft::data_metrics()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.data_metrics
[`Raft::server_metrics()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.server_metrics
[`ensure_linearizable()`]: https://docs.rs/openraft/0.10.0-alpha.29/openraft/raft/struct.Raft.html#method.ensure_linearizable
