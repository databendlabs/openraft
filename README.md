<div align="center">
    <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/logo/openraft-banner-dark.svg">
        <img src="assets/logo/openraft-banner.svg" alt="OpenRaft">
    </picture>
    <h4>
        Advanced <a href="https://raft.github.io/">Raft</a> in 🦀 Rust on any async runtime. Please ⭐ on <a href="https://github.com/databendlabs/openraft">GitHub</a>!
    </h4>


[![Crates.io](https://img.shields.io/crates/v/openraft.svg)](https://crates.io/crates/openraft)
[![docs.rs](https://docs.rs/openraft/badge.svg)](https://docs.rs/openraft)
[![DeepWiki](https://img.shields.io/badge/DeepWiki-openraft-blue.svg?logo=data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACwAAAAyCAYAAAAnWDnqAAAAAXNSR0IArs4c6QAAA05JREFUaEPtmUtyEzEQhtWTQyQLHNak2AB7ZnyXZMEjXMGeK/AIi+QuHrMnbChYY7MIh8g01fJoopFb0uhhEqqcbWTp06/uv1saEDv4O3n3dV60RfP947Mm9/SQc0ICFQgzfc4CYZoTPAswgSJCCUJUnAAoRHOAUOcATwbmVLWdGoH//PB8mnKqScAhsD0kYP3j/Yt5LPQe2KvcXmGvRHcDnpxfL2zOYJ1mFwrryWTz0advv1Ut4CJgf5uhDuDj5eUcAUoahrdY/56ebRWeraTjMt/00Sh3UDtjgHtQNHwcRGOC98BJEAEymycmYcWwOprTgcB6VZ5JK5TAJ+fXGLBm3FDAmn6oPPjR4rKCAoJCal2eAiQp2x0vxTPB3ALO2CRkwmDy5WohzBDwSEFKRwPbknEggCPB/imwrycgxX2NzoMCHhPkDwqYMr9tRcP5qNrMZHkVnOjRMWwLCcr8ohBVb1OMjxLwGCvjTikrsBOiA6fNyCrm8V1rP93iVPpwaE+gO0SsWmPiXB+jikdf6SizrT5qKasx5j8ABbHpFTx+vFXp9EnYQmLx02h1QTTrl6eDqxLnGjporxl3NL3agEvXdT0WmEost648sQOYAeJS9Q7bfUVoMGnjo4AZdUMQku50McDcMWcBPvr0SzbTAFDfvJqwLzgxwATnCgnp4wDl6Aa+Ax283gghmj+vj7feE2KBBRMW3FzOpLOADl0Isb5587h/U4gGvkt5v60Z1VLG8BhYjbzRwyQZemwAd6cCR5/XFWLYZRIMpX39AR0tjaGGiGzLVyhse5C9RKC6ai42ppWPKiBagOvaYk8lO7DajerabOZP46Lby5wKjw1HCRx7p9sVMOWGzb/vA1hwiWc6jm3MvQDTogQkiqIhJV0nBQBTU+3okKCFDy9WwferkHjtxib7t3xIUQtHxnIwtx4mpg26/HfwVNVDb4oI9RHmx5WGelRVlrtiw43zboCLaxv46AZeB3IlTkwouebTr1y2NjSpHz68WNFjHvupy3q8TFn3Hos2IAk4Ju5dCo8B3wP7VPr/FGaKiG+T+v+TQqIrOqMTL1VdWV1DdmcbO8KXBz6esmYWYKPwDL5b5FA1a0hwapHiom0r/cKaoqr+27/XcrS5UwSMbQAAAABJRU5ErkJggg==)](https://deepwiki.com/databendlabs/openraft)
[![guides](https://img.shields.io/badge/guide-%E2%86%97-brightgreen)](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/index.html)
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


- 🚀 **Get started**: [Guide](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/getting_started/index.html) · [Docs](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/index.html) · [FAQ](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/faq/index.html) · [DeepWiki](https://deepwiki.com/databendlabs/openraft).
- 💡 **Examples**: [0.10.0-alpha.37](https://github.com/databendlabs/openraft/tree/release-0.10/examples) · [0.9](https://github.com/databendlabs/openraft/tree/release-0.9/examples).
- 🙌 **Questions**? Ask on [Discord](https://discord.gg/ZKw3WG7FQ9), in a [GitHub discussion](https://github.com/databendlabs/openraft/discussions/new), in our [Feishu group](https://applink.feishu.cn/client/chat/chatter/add_by_link?link_token=d20l9084-6d36-4470-bac5-4bad7378d003), or on WeChat: `drmingdrmer`.


# Status

- **[Features](#features)**: OpenRaft implements nearly all features needed to build applications with Raft.
- **Performance**: OpenRaft supports 33,000 writes/sec with a single writer and 5,615,000 writes/sec with batch writes. See the [benchmarks](#performance).
- **Coverage**: Unit test coverage is 92%.
- **Chaos testing**: A [deterministic simulation fuzzer](./tests-turmoil) runs in every CI build.
  The [Jepsen suite](./jepsen) runs 8 nemesis scenarios on every push to `main`:
  network partition, process pause, process kill, slow packets, flaky packets, clock skew,
  membership churn, and all seven combined.


## API status

**OpenRaft's API continues to evolve.** Upgrades before `1.0.0` may include incompatible changes.
See the [changelog](https://github.com/databendlabs/openraft/blob/main/change-log.md). Commit messages start with a keyword indicating the type of change:

- `data-change:` changes on-disk data types and may require a manual upgrade.
- `change:` introduces incompatible changes.
- `feat:` adds compatible, non-breaking features.
- `fix:` contains only bug fixes.

## Versions

| Branch | Status | Release | Upgrade guide |
| --- | --- | --- | --- |
| `main` / [release-0.10](https://github.com/databendlabs/openraft/tree/release-0.10) | Alpha; active development | [0.10.0-alpha.37](https://crates.io/crates/openraft/0.10.0-alpha.37) | [0.9 → 0.10](https://docs.rs/openraft/0.10.0-alpha.37/openraft/docs/upgrade_guide/upgrade_09_10/index.html) |
| [release-0.9](https://github.com/databendlabs/openraft/tree/release-0.9) | Bug fixes only | [0.9.25](https://github.com/databendlabs/openraft/tree/v0.9.25) ([changelog](https://github.com/databendlabs/openraft/blob/release-0.9/change-log.md)) | [0.8 → 0.9](https://docs.rs/openraft/0.9.0/openraft/docs/upgrade_guide/upgrade_08_09/index.html) |

<details>
<summary>Unsupported releases</summary>

[release-0.8](https://github.com/databendlabs/openraft/tree/release-0.8),
[release-0.7](https://github.com/databendlabs/openraft/tree/release-0.7), and
[release-0.6](https://github.com/databendlabs/openraft/tree/release-0.6) are closed;
their [upgrade guides](https://docs.rs/openraft/0.8.9/openraft/docs/upgrade_guide/index.html) remain available.

</details>

# Performance

This benchmark measures the OpenRaft framework with minimal storage and networking; see [benchmark details](./benchmarks/minimal).

| Clients | Single writes (put/s) | Batch writes (4 entries/batch, put/s) |
| --: | --: | --: |
| 4096 | 3,548,000 | 5,615,000 |
| 1024 | 3,006,000 | — |
| 256 | 1,808,000 | — |
| 64 | 912,000 | — |
| 1 | 33,000 | — |

# Features

- ✅ **Generalized membership changes**: [change any set of nodes](./openraft/src/docs/features.md#membership) in one operation, using one log entry when safe or joint consensus otherwise.
- ✅ **Flexible quorums**: choose [threshold, weighted, hierarchical, grid, explicit, or custom rules](./membership/src/docs.md), with separate election and commit quorums.
- ✅ **High throughput**: [streaming replication](./openraft/src/docs/features.md#replication) and [batch writes](./openraft/src/docs/features.md#reads-and-writes), backed by an async, event-driven core.
- ✅ **Linearizable reads**: [reads confirmed by a quorum, lease reads, and follower reads](./openraft/src/docs/features.md#reads-and-writes).
- ✅ **Fully pluggable**: customize [storage, networking, snapshot transport, and core Raft types](./openraft/src/docs/features.md#runtime).
- ✅ **Runtime choice**: [Tokio, compio, or monoio](./openraft/src/docs/features.md#runtime), with support for `single-threaded` applications.
- ✅ **Tested with Turmoil and Jepsen**: [deterministic simulation and fault testing](./openraft/src/docs/features.md#testing-and-monitoring) check Raft invariants, linearizability, and recovery.

See the [full feature list](./openraft/src/docs/features.md).


# Who uses it

- [Databend](https://github.com/databendlabs/databend) - Cloud analytics.
- [Walrus](https://github.com/nubskr/walrus) - Distributed message streaming.
- [Hiqlite](https://github.com/sebadob/hiqlite) - Embedded SQLite with caching.

[See all users](./users.md).

# Contributing

Check out the [CONTRIBUTING.md](https://github.com/databendlabs/openraft/blob/main/CONTRIBUTING.md)
guide for more details on getting started with contributing to this project.

## Contributors

Thanks to everyone who contributes to OpenRaft:

<!-- Regenerate: scripts/contributors/list_contributors.py (writes
     assets/contributors/{contributors.txt,contributors.md}), then
     scripts/contributors/generate_contributor_svgs.py (writes assets/contributors/svg/) -->
[<img src="assets/contributors/svg/aadil-srivastava01.svg" alt="@aadil-srivastava01"/>](https://github.com/aadil-srivastava01)
[<img src="assets/contributors/svg/ariesdevil.svg" alt="@ariesdevil"/>](https://github.com/ariesdevil)
[<img src="assets/contributors/svg/AustenSchunk.svg" alt="@AustenSchunk"/>](https://github.com/AustenSchunk)
[<img src="assets/contributors/svg/bohutang.svg" alt="@bohutang"/>](https://github.com/bohutang)
[<img src="assets/contributors/svg/byronwasti.svg" alt="@byronwasti"/>](https://github.com/byronwasti)
[<img src="assets/contributors/svg/caicancai.svg" alt="@caicancai"/>](https://github.com/caicancai)
[<img src="assets/contributors/svg/CentRa-Linux.svg" alt="@CentRa-Linux"/>](https://github.com/CentRa-Linux)
[<img src="assets/contributors/svg/chahat-101.svg" alt="@chahat-101"/>](https://github.com/chahat-101)
[<img src="assets/contributors/svg/claude.svg" alt="@claude"/>](https://github.com/claude)
[<img src="assets/contributors/svg/cliff0412.svg" alt="@cliff0412"/>](https://github.com/cliff0412)
[<img src="assets/contributors/svg/ClSlaid.svg" alt="@ClSlaid"/>](https://github.com/ClSlaid)
[<img src="assets/contributors/svg/danthegoodman1.svg" alt="@danthegoodman1"/>](https://github.com/danthegoodman1)
[<img src="assets/contributors/svg/devillove084.svg" alt="@devillove084"/>](https://github.com/devillove084)
[<img src="assets/contributors/svg/Diggsey.svg" alt="@Diggsey"/>](https://github.com/Diggsey)
[<img src="assets/contributors/svg/diptanu.svg" alt="@diptanu"/>](https://github.com/diptanu)
[<img src="assets/contributors/svg/DRMacIver.svg" alt="@DRMacIver"/>](https://github.com/DRMacIver)
[<img src="assets/contributors/svg/drmingdrmer.svg" alt="@drmingdrmer"/>](https://github.com/drmingdrmer)
[<img src="assets/contributors/svg/eliasyaoyc.svg" alt="@eliasyaoyc"/>](https://github.com/eliasyaoyc)
[<img src="assets/contributors/svg/emmanuel-ferdman.svg" alt="@emmanuel-ferdman"/>](https://github.com/emmanuel-ferdman)
[<img src="assets/contributors/svg/emon100.svg" alt="@emon100"/>](https://github.com/emon100)
[<img src="assets/contributors/svg/ethe.svg" alt="@ethe"/>](https://github.com/ethe)
[<img src="assets/contributors/svg/evanj.svg" alt="@evanj"/>](https://github.com/evanj)
[<img src="assets/contributors/svg/gardenias.svg" alt="@gardenias"/>](https://github.com/gardenias)
[<img src="assets/contributors/svg/George-Miao.svg" alt="@George-Miao"/>](https://github.com/George-Miao)
[<img src="assets/contributors/svg/getong.svg" alt="@getong"/>](https://github.com/getong)
[<img src="assets/contributors/svg/glennswest.svg" alt="@glennswest"/>](https://github.com/glennswest)
[<img src="assets/contributors/svg/GrapeBaBa.svg" alt="@GrapeBaBa"/>](https://github.com/GrapeBaBa)
[<img src="assets/contributors/svg/gtema.svg" alt="@gtema"/>](https://github.com/gtema)
[<img src="assets/contributors/svg/guojidan.svg" alt="@guojidan"/>](https://github.com/guojidan)
[<img src="assets/contributors/svg/hadronzoo.svg" alt="@hadronzoo"/>](https://github.com/hadronzoo)
[<img src="assets/contributors/svg/HaHa421.svg" alt="@HaHa421"/>](https://github.com/HaHa421)
[<img src="assets/contributors/svg/homa31.svg" alt="@homa31"/>](https://github.com/homa31)
[<img src="assets/contributors/svg/iamazy.svg" alt="@iamazy"/>](https://github.com/iamazy)
[<img src="assets/contributors/svg/iMashtak.svg" alt="@iMashtak"/>](https://github.com/iMashtak)
[<img src="assets/contributors/svg/jamals86.svg" alt="@jamals86"/>](https://github.com/jamals86)
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
[<img src="assets/contributors/svg/lsytj0413.svg" alt="@lsytj0413"/>](https://github.com/lsytj0413)
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
[<img src="assets/contributors/svg/nremond.svg" alt="@nremond"/>](https://github.com/nremond)
[<img src="assets/contributors/svg/othmanekn.svg" alt="@othmanekn"/>](https://github.com/othmanekn)
[<img src="assets/contributors/svg/P47Phoenix.svg" alt="@P47Phoenix"/>](https://github.com/P47Phoenix)
[<img src="assets/contributors/svg/pchrysostomou.svg" alt="@pchrysostomou"/>](https://github.com/pchrysostomou)
[<img src="assets/contributors/svg/PeterKnego.svg" alt="@PeterKnego"/>](https://github.com/PeterKnego)
[<img src="assets/contributors/svg/pfwang80s.svg" alt="@pfwang80s"/>](https://github.com/pfwang80s)
[<img src="assets/contributors/svg/Phoenix500526.svg" alt="@Phoenix500526"/>](https://github.com/Phoenix500526)
[<img src="assets/contributors/svg/pinylin.svg" alt="@pinylin"/>](https://github.com/pinylin)
[<img src="assets/contributors/svg/ppamorim.svg" alt="@ppamorim"/>](https://github.com/ppamorim)
[<img src="assets/contributors/svg/PsiACE.svg" alt="@PsiACE"/>](https://github.com/PsiACE)
[<img src="assets/contributors/svg/Qian-Cheng-nju.svg" alt="@Qian-Cheng-nju"/>](https://github.com/Qian-Cheng-nju)
[<img src="assets/contributors/svg/rvielma.svg" alt="@rvielma"/>](https://github.com/rvielma)
[<img src="assets/contributors/svg/sainad2222.svg" alt="@sainad2222"/>](https://github.com/sainad2222)
[<img src="assets/contributors/svg/SAY-5.svg" alt="@SAY-5"/>](https://github.com/SAY-5)
[<img src="assets/contributors/svg/schreter.svg" alt="@schreter"/>](https://github.com/schreter)
[<img src="assets/contributors/svg/SebastianThiebaud.svg" alt="@SebastianThiebaud"/>](https://github.com/SebastianThiebaud)
[<img src="assets/contributors/svg/shuoli84.svg" alt="@shuoli84"/>](https://github.com/shuoli84)
[<img src="assets/contributors/svg/socutes.svg" alt="@socutes"/>](https://github.com/socutes)
[<img src="assets/contributors/svg/sollhui.svg" alt="@sollhui"/>](https://github.com/sollhui)
[<img src="assets/contributors/svg/SteveLauC.svg" alt="@SteveLauC"/>](https://github.com/SteveLauC)
[<img src="assets/contributors/svg/sunli829.svg" alt="@sunli829"/>](https://github.com/sunli829)
[<img src="assets/contributors/svg/sunsided.svg" alt="@sunsided"/>](https://github.com/sunsided)
[<img src="assets/contributors/svg/svix-jbrown.svg" alt="@svix-jbrown"/>](https://github.com/svix-jbrown)
[<img src="assets/contributors/svg/svix-jplatte.svg" alt="@svix-jplatte"/>](https://github.com/svix-jplatte)
[<img src="assets/contributors/svg/tangwz.svg" alt="@tangwz"/>](https://github.com/tangwz)
[<img src="assets/contributors/svg/thedodd.svg" alt="@thedodd"/>](https://github.com/thedodd)
[<img src="assets/contributors/svg/tisonkun.svg" alt="@tisonkun"/>](https://github.com/tisonkun)
[<img src="assets/contributors/svg/tobodner.svg" alt="@tobodner"/>](https://github.com/tobodner)
[<img src="assets/contributors/svg/trungda.svg" alt="@trungda"/>](https://github.com/trungda)
[<img src="assets/contributors/svg/tvsfx.svg" alt="@tvsfx"/>](https://github.com/tvsfx)
[<img src="assets/contributors/svg/undecidedapollo.svg" alt="@undecidedapollo"/>](https://github.com/undecidedapollo)
[<img src="assets/contributors/svg/Veeupup.svg" alt="@Veeupup"/>](https://github.com/Veeupup)
[<img src="assets/contributors/svg/VerifiedOrganic.svg" alt="@VerifiedOrganic"/>](https://github.com/VerifiedOrganic)
[<img src="assets/contributors/svg/vigith.svg" alt="@vigith"/>](https://github.com/vigith)
[<img src="assets/contributors/svg/wvwwvwwv.svg" alt="@wvwwvwwv"/>](https://github.com/wvwwvwwv)
[<img src="assets/contributors/svg/xu-cheng.svg" alt="@xu-cheng"/>](https://github.com/xu-cheng)
[<img src="assets/contributors/svg/Xuanwo.svg" alt="@Xuanwo"/>](https://github.com/Xuanwo)
[<img src="assets/contributors/svg/YangKian.svg" alt="@YangKian"/>](https://github.com/YangKian)
[<img src="assets/contributors/svg/ygf11.svg" alt="@ygf11"/>](https://github.com/ygf11)
[<img src="assets/contributors/svg/yichenshen.svg" alt="@yichenshen"/>](https://github.com/yichenshen)
[<img src="assets/contributors/svg/youichi-uda.svg" alt="@youichi-uda"/>](https://github.com/youichi-uda)
[<img src="assets/contributors/svg/zach-schoenberger.svg" alt="@zach-schoenberger"/>](https://github.com/zach-schoenberger)
[<img src="assets/contributors/svg/ZhangZihao270.svg" alt="@ZhangZihao270"/>](https://github.com/ZhangZihao270)
[<img src="assets/contributors/svg/zhixinwen.svg" alt="@zhixinwen"/>](https://github.com/zhixinwen)

# License

OpenRaft is licensed under the terms of the [MIT License](https://en.wikipedia.org/wiki/MIT_License#License_terms)
or the [Apache License 2.0](http://www.apache.org/licenses/LICENSE-2.0), at your choosing.

OpenRaft originated from [async-raft](https://docs.rs/crate/async-raft/latest).
