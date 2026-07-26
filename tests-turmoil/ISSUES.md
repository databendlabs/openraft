# tests-turmoil: Open Work

本文只记录当前仍需要处理的事项。已完成并移除的条目包括:确定性修复
(`HashSet` -> `BTreeSet`、tokio watch `rng_seed`、futures-util `reseed`)、
workload throughput 修复、旧 violation seed 复验、`CommittedOnQuorum` 边界修复、
log-history invariants 审计、membership retry-with-cooldown 与
attempts/applied/failed 计数、client oracle 与 liveness phase、RPC soft_ttl
超时、joint-config 收敛期 finalize。

所有命令默认在 `tests-turmoil/` 目录下执行(必须:`.cargo/config.toml`
提供 `--cfg tokio_unstable`,从仓库根目录构建会触发 compile_error)。

---

## 1. Mutation coverage gap: #1828 fully-purged probe dead-end

**Severity:** Low. 该 bug 已在 openraft 修复(afd6508b);本条目只关于
fuzzer 的检出能力。

Revert afd6508b 的语义突变(probing regime 下 `start == end` 时返回
`Inflight::None` 而不是发 snapshot)在 ~800 个随机迭代内未被检出。
插桩确认突变分支从未命中:触发需要
`searching_end == purge_upto_next == last_next` 的等式角,即
"新建 progress entry 之后、full purge 之前零 append,然后静默直到观察"。
相邻情形(`searching_end < purge_upto_next`)由更早就存在的
snapshot condition 1 兜底,不经过突变分支。

已为提高概率加入的机制:`Compact` trigger(snapshot+purge-to-tip,偏向
leader)、`pre_liveness_quiet_ticks`(安全阶段尾部写入静默)。仍未命中。

**Recommended approach:** 写一个 targeted deterministic scenario test
(非随机):3 节点,hold 住节点 X 的链路 → add_learner(X) → 无写入 →
leader Compact → release X → 要求收敛。该测试在突变代码上应失败、
在修复代码上应通过。

## 2. Mutation coverage gap: #1805 stale transfer-leader election

**Severity:** Low. 同上,bug 已修复(d000ee20),只关于检出能力。

只 revert `Candidate::grant_by()` 的 panic(fix 的另一半 —— transfer
target 的 log-flush 检查 —— 保留)在 250 个迭代内未检出:flush 检查挡住了
promoted-learner 窗口这条主要路径。完整 revert 涉及 9 个文件的 API 变更
(`TransferLeaderRequest` 携带 `last_log_id` 等),reverse-apply 冲突大,
未尝试。

**Recommended approach:** 若要覆盖,考虑 targeted scenario:
promote learner X 且 hold 其 promotion log 的复制 → transfer_leader(X)。
或接受该角落由 openraft 自身的 regression test 覆盖。

### Verification

```sh
cargo fmt
cargo test --lib
cargo run --release --bin fuzz -- --seed 500 --iterations 20 --max-steps 50000
```
