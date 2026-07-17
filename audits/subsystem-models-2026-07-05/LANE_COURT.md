# Lane C — Court 子系统协议模型审计

> 审计类型:协议层 (protocol-model),不是实现层。
> 工作树:`/Users/arthur/RustroverProjects/Myelin`(在 `main` 分支,有未提交 cellscript 子模块改动)。
> 审计日期:2026-07-05。
> 范围:court 协议的不变量、dispute 触发条件、final-script 在协议层的角色、`identity(field(...))` 是否真实承诺、court bundle 边界语义、court economics 与 court 的关系。
> **重要前提**:`cellscript/examples/myelin/{da-anchor-final,settlement-final,da-anchor-carrier,settlement-carrier}.cell` 在 commit `8661e1b` "Merge CellScript upstream and harden Myelin gates" 已被删除。`myelin_ckb_devnet_smoke.sh:114-117` 与 `myelin_public_testnet_rehearsal_prepare.sh:80-83` 仍引用这些文件路径,本审计基于已删除文件的存在证据(`git show 8661e1b -- "...cell"`)与 main 分支当前 `cli/src/main.rs` 代码审计推断其协议层语义。

## TL;DR

Court 子系统协议层建立在 4 类不变量上:**chunk 决定性拆分**(由 `chunk_index`、offset、commit、wtxid 与 Molecule 序列化共同决定)、**bundle 完整性证明**(11-12 个 `push_check` 在 `verify_session_court_bundle` 内循环覆盖)、**disputed-close 时间门**(`settlement_permitted = current_time_ms >= challenge_deadline_ms`)、以及 **court economics 承诺**(`d1bb6f7` 之后的 `commitment-only evidence`,即见证的不再是 *整体 disputed 经济学是否完整*,而是 *disputed 经济学在协议层的存在*,且 `testnet_beta_ready` 与 `production_ready` 永远为 false)。

严密度总体:court bundle 的 11 项 verifier check 是闭环的(`cli/src/main.rs:5880-6097`),Molecule 序列化、block hash、evidence block hash、challenge payload hash 的耦合在协议层是加密可验证的;但 **chunk 拆分只是序列索引同构**,不是真正的随机选择/对抗裁剪,**identity(field(...)) 在协议层不是承诺而是 typed-cell 元数据**(即使在 bd0571b 的 fixture 中也有 `identity(field(...))` 注解,但 `exec/src/celltx/types.rs` 没有对应的 `TypedCellDecl`),因此 `da-anchor-final` 与 `settlement-final` 在协议层**是**"资源"声明而**不是**"单例化承诺"。

合理性总体:court 协议在"static closed committee + contested-chunk JSON replay + off-chain DA segment"上是自洽的;但 final-script fixtures `da-anchor-final.cell` / `settlement-final.cell` 同时承担 4 类角色(`identity` typed-cell、160-byte payload data layout、`data2` type-script args layout、authority-cell/consumed input 验证),在协议层混淆了 **资源身份**、**type-cell identity binding**、**consumption policy** 三个独立维度。court economics 是 commitment-only 而非 enforcement 的降级是 `d1bb6f7` 的有意声明;但 README:360 (pre-`d1bb6f7` snapshot) 显示 *裁判经济学承诺* 与 *完整裁判经济学* 的语义区别从 commit 名称已经在协议层做出。

安全性总体:court bundle 的 Molecule 绑定 + wtxid + block-hash 是加密防伪的;但 **挑战窗口是一个单一协议层信号**:`settlement_permitted` 只看 `current_time_ms >= challenge_deadline_ms`,没有"多证人阻止"、没有"司法裁判者拒绝",**在 static-closed-committee 协议层里,dispute 严格地说是 self-service 的**——只有"我有 challenge payload hash"的人能阻止 settlement,且**没有 protocol-layer mechanism** 区分合法 challenger 与卡 window griefing 的 challenger。`challenge_window_ms` 是 CLI 参数(默认 60000,见 `cli/src/main.rs:524-525`),**协议层没有最小/最大值约束**,攻击者可以把 window 调成 0 然后用合法的 JSON 路径绕过 challenge 检查(代码在 `:6979-6980` 强制 `challenge_window_ms > 0`,但只防 `0`,不防 1 millisecond 的攻击)。`fee_policy: "submission-economics-report-enforces-ckb-fee-floor-rate-and-max-fee"`(`:4131`)是协议层声明但只在 `submission_economics_*` 子系统强制;court_economics commitment domain 不覆盖 fee,使得 griefing 自身不会被抵押品锁定。

**已知缺陷已吸收**:`XD-01 / F-CLI-01 / F-DOC-01 / F-SCRIPT-14 / D1BB6F7` 的协议层含义在本报告 §6 中讨论(不作为新 finding 重复)。

## 1. 模型边界

Court 子系统的协议层由四个相互嵌套的对象构成:

1. **DisputedBundle**(`MYELIN_SESSION_L2_PLAN.md:78-79` 中定义的协议层概念,实现层为 `SessionCourtBundleReport` / `myelin-session-court-bundle-v1` schema):
   - 它是一份 "challenged chunk + recomputable replay" 的 **纯 JSON 容器**。
   - 协议层字段:`session_id`、`chunk_index`、`consensus_kind`、`vm_profile`、`state_root_before`、`state_root_after`、`data_commitments[0]`、`scheduler_commitment`、`block` (含 `MyelinBlock` 全部 12 字段)、`molecule_transaction_hex`、`molecule_transaction_hash`、`challenge_payload_hash`、`ordered_cell_tx_commitments[0]`、`finality_evidence`(`static_committee_evidence` 或 `tendermint_evidence` 二选一)、`ckb_projection`、`court_verifiable`、`l1_court_implemented: false`。

2. **SettlementIntent**(`myelin-session-settlement-intent-v1`,只有 `kind = "disputed-close"` 被代码承认,见 `cli/src/main.rs:6976-6978`):
   - 它是 DisputedBundle 的下一步,**仅在** `current_time_ms >= block.timestamp_ms + challenge_window_ms` 时为 `settlement_permitted: true`(`:7004`)。
   - 必须绑定 verified `court_bundle_hash` + `da_manifest_hash` + `participant_set_hash` + `escrow_input_cells_hash` + `state_root_before`/`final_state_root`(:7041-7044)。

3. **SettlementPackage**(`myelin-session-settlement-package-v1`,消费 SettlementIntent + verified court + verified DA):
   - 输出 **deterministic CKB-compatible settlement CellTx**(:7374 `session_settlement_cell_tx`)。
   - 包含 settlement_authority input `consumed_input_index = 1`(:4565-4570 强制)、`session_authority_commitment`(:4545-4551 强制必须是 32-byte hex 且与 recomputed 匹配)。
   - 持有 `l1_court_script_implemented: false`(`:7445`),协议层这只是 CellTx + verifier 的 **模板**,不是 L1 部署的 contract。

4. **court economics commitment**(`myelin-session-court-economics-v1`):
   - 协议层只是 over-binding(`blake3(myelin:session-court-economics:v1, ...)`)的 32-byte commitment(:4170-4178 及 :4147-4150 的算法字符串)。
   - **关键协议层声明**:`testnet_beta_ready: false`、`production_ready: false` 总为 false(:4153-4154);即使提供 `court_economics_deployment_evidence`(:4156-4165)将 `economics_invariant_checked` 取与,但 `production_ready` 仍由 deployment 字段决定(`:4161-4162`)。`d1bb6f7` 是把 mode 从 `"disputed-close-testnet-beta"` 改成 `"disputed-close-policy-commitment"` 并把 `testnet_beta_ready = false` 写死为默认的 commit。

**本审计的"协议层"范围**包括:
- Chunk 拆分的协议层语义(谁决定 chunk 边界、由谁选 `chunk_index`、chunk 序列是否决定)。
- Dispute 触发条件(谁能生成 court bundle、bundle 与 settlement intent 的关系、challenge_window 的语义)。
- Dispute path 在协议层是不是 paper-only(在 closed static committee 下的 chain-of-custody)。
- Final-script fixtures 在协议层的角色(`identity(field(...))` 是不是承诺、type-cell identity 在 L1 是不是 enforcement)。
- Court bundle 的边界(field 范围、bundle 完整性验证、`note` 字段 `l1_court_implemented: false` 的语义)。
- Court economics 与 court 的关系(court economics 是 court 子系统的 enforcement 还是 court bundle 的 commitment 副产物,以及 `d1bb6f7` 的降级在协议层如何被声明)。

**协议层不**:
- 报告 CLI helper 缺失、live script 角色映射、生产 gate 覆盖(XD-01 等已在 WHOLEREPO 报告,本报告 §6 仅讨论其协议层含义)。
- 重新审计 celltx 哈希 collision(F-PRIM-01,讨论见 §6 与 cross-subsystem interface §7)。
- 重新审计 DA 子系统的 production_ready 计算(详见 LANE_DA 报告)。

## 2. 严密度评估

### 2.1 总体

Court 子系统在 **bundle reconstruction** 方面是闭环的:11 项 verifier check 把 Molecule bytes、wtxid、block hash、block 与 finality evidence 的 binding、challenge_payload_hash 的所有参与字段都重新计算并 `==` 比对(`cli/src/main.rs:5880-6097`)。这套 verifier 在 main branch 当前状态已经在 14 条 commit 上迭代(commit history 验证 `9163ea6` ... `3fda2ab` 等),且每次 commit 都调整其中一两个 check,说明 protocol design 是 active。

**协议层不闭环的**:
1. **chunk_index 是单一攻击面**:`session_court_bundle` 在 `:5778-5786` 用 `chunk_index != commit.chunk_index` 比对,但协议层没有"挑战 chunk index 选择不一致"的检查,被挑战者(rollup 操作员)可能用不真实的 challenge_payload 反向指控挑战者(详见 F-COURT-01)。
2. **dispute-path 没有 protocol-layer evidence-of-dispute 角色**:`verify_session_court_bundle` 用 `finality_evidence.block_hash == block_hash` 检查(:6075-6080),但没有"挑战者身份"的 binding——DisputedBundle 在协议层是匿名的(griefing 详见 F-COURT-02)。
3. **`l1_court_implemented: false` 是协议层表面声明,但 carrier/final-script 区分是 CLI 内部的**:协议层"disputed"与"disputed via final-script"没有 protocol-level 标记,只有 CLI `submission_schema` enum(`cli/src/main.rs:10344` 后)。
4. **`identity(field(...))` 协议层只是 typed-cell metadata**:`exec/src/celltx/types.rs` 没有 `SettlementFinal` / `DaAnchorFinal` / `DaAnchorCarrier` / `SettlementCarrier` 的 `TypedCellDecl`,因此 `cellscript/examples/myelin/{da-anchor-final,settlement-final}.cell:4` 的 `identity(field(da_manifest_hash))` / `identity(field(intent_hash))` 在协议层不是 commitment,只是 **资源标签**:代码 type-script 在 `verify_final_da_publication`(:19-43,见 deleted bd0571b snapshot)读 5 个 32-byte field 并对比 type args 的 prefix/suffix,**identity 注解没被 codegen 强制**。

### 2.2 Findings (严密度)

#### F-COURT-01 (CRITICAL) — chunk_index 是 DisputedBundle 的唯一身份字段,协议层缺少 "who 选定 / who 拒绝" 元数据

**维度**:严密度 / 安全性
**严重度**:CRITICAL

**观察**:`session_court_bundle` 用 `cli/src/main.rs:5781-5786` 强制 `chunk_index == commit.chunk_index`,协议的 "disputed chunk" 完全由 `chunk_index` 确定;但没有任何协议层字段表示 "who challenged" / "when challenged" / "which side is challenger"。整个 protocol-lane 的"chunk 拆分是确定的"在静态上是 true,在对抗条件下是 **circular reasoning**——同一个 `chunk_index` 可以同时被两个不同对手用于构造两个不同 `DisputedBundle`(它们 Molecule bytes 相同,但 bundle JSON 里 `note` / 派生 metadata 可以不同)。

**证据**:
- `cli/src/main.rs:5778-5786` — 单一字段 `chunk_index` 决定 bundle 身份;
- `cli/src/main.rs:7052` `kind: kind.to_owned()`(只接 `"disputed-close"`),没有 challenger 字段;
- `cli/src/main.rs:3995-4115` (settlement_intent fields) — `participant_set_hash` 是 session-level,**不是 challenger-level**;
- `MYELIN_SESSION_L2_PLAN.md:78-80` 定义 `DisputeBundle` 为 "one challenged chunk plus all data needed to recompute"，**没有任何 chunk-index-attribution 字段**。

**影响**:在静态委员会下,attacker 提交一个 DisputedBundle 用合法 `chunk_index`,block operator 用另一个 DisputedBundle 同样合法 `chunk_index`,**协议层没有办法机械分辨这两个 bundle 谁先到**,但 finality_evidence.block_hash 双方匹配(因为是同一 block)。**Griefing** 路径:defender 永远在 challenger 后 1 ms 生成一个同样的 bundle 反驳,challenge window 因此 always-resets 实际不增加(因为 `challenge_deadline_ms` 在生成 settlement_intent 时一次性 anchor 到 `block.timestamp_ms + challenge_window_ms`)。

**建议方向**:
- 协议层应在 `DisputeBundle` 加 `challenger_pubkey_hash` / `challenge_originator_kind` (e.g., `participant | operator | third-party-arbiter`),并强制 `kind = "disputed-close"` 时必填。
- 协议层应加 `challenge_nonce` 或 `challenge_digest` 让同一 `chunk_index` 可以有多个合法 dispute 实例。

#### F-COURT-02 (CRITICAL) — challenge window 协议层无最小/最大边界,可被 1-millisecond self-skip griefing

**维度**:严密度 / 合理性
**严重度**:CRITICAL

**观察**:`session_settlement_intent` 在 `cli/src/main.rs:6979-6980` 强制 `challenge_window_ms > 0`,但**不强制最小值**。在 main branch 当前状态下,CLI 默认 `challenge_window_ms = 60_000`(:524),但 plan 显式把它当作 `--current-time-ms 60000 --challenge-window-ms 60000` 的 fixture-specific 数字(:159)。在协议层,`current_time_ms >= challenge_deadline_ms = block.timestamp_ms + challenge_window_ms`(:7000-7004)是单一的 "settlement permission" gate,而 `challenge_window_ms` 是 CLI 参数。**没有协议层 invariant 阻止一个对手把 window 设成 1 毫秒,然后立刻生成 settlement_intent 并 settling**。

**证据**:
- `cli/src/main.rs:524` `#[arg(long, default_value_t = 60_000)] challenge_window_ms` — 没有 minimum validator;
- `cli/src/main.rs:6979-6980` — 仅检查 `challenge_window_ms != 0`;
- `cli/src/main.rs:7000-7004` — `settlement_permitted` 是 `>`= 比较,无最小 gap requirement;
- `MYELIN_SESSION_L2_PLAN.md:159` 把 `--current-time-ms 60000 --challenge-window-ms 60000` 显式列出为 fixture 测试参数,不是 protocol layer constant;
- `docs/public-testnet-rehearsal-runbook.md:264-266,283-286`(per WHOLEREPO F-DOC-07)持 `current-time-ms 60000 --challenge-window-ms 60000` 没有 guidance。

**影响**:`settlement_permitted` 完全是 **duration-not-meaningful** gate。1-millisecond window + 当前时间已经逝去的 attacker 路径是开放的,defender 在毫秒内无法响应(commit/verify 都需要常数级别 ms 级时间,而 block.timestamp_ms 是 session 创建时的 timestamp,在 main branch fixture 中是 `0`,这导致 `settlement_permitted` 永远是 `current_time_ms >= challenge_window_ms` 的退化形式)。

**建议方向**:
- 协议层约束(在 `session_settlement_intent` 后置,在 `MyelinBlock.timestamp_ms` 之前置):`max(60_000, challenge_window_ms) <= elapsed_from_block`,并允许 challenge_window 是 configurable 但不允许 < minimum(协议文档需要回答 "minimum challenge window 是多少")。

#### F-COURT-03 (HIGH) — DisputedBundle 协议层 field-by-field verification 是密闭环,但 dispute 提交者身份未在协议层声明

**维度**:严密度
**严重度**:HIGH

**观察**:`verify_session_court_bundle` 在 `cli/src/main.rs:5880-6097` 实现 11 项 `push_check`,覆盖 schema/vm-profile/spawn-ipc/molecule-transaction-hash/ordered-celltx-commitment/projection-possible/projection-profile/block-hash-recomputes/block-consensus-kind-matches/block-state-root-before-matches/block-state-root-after-matches/block-scheduler-commitment-matches/participant-set-hash/escrow-input-cells-hash/session-lineage-commitment/block-data-commitment-matches/challenge-payload-hash/evidence-block-hash-matches-canonical-block/court-verifiable-profile。其中每一项都是 byte-precise 的 "recompute + equality"。**但整个 verifier** 不接受任何 "提交者的身份 / 签名 / 凭证" 输入,而是接受一个 **任意的** DisputedBundle JSON。这虽然在内层是密闭环的(12 项 check 都是 hash-equal),但因为 **bundle 本身是 anon 的**,任何第三方可以拿任何已签发的 JSON 包验证并 claim "see, the bundle verifies"。

**证据**:
- `cli/src/main.rs:5879-5881` — `verify_session_court_bundle` 只读 bundle path,无 `signer` / `key` / `provider` 参数;
- `cli/src/main.rs:5883-5890` schema check — `"myelin-session-court-bundle-v1"` 是固定字符串,**没有绑 signing key**;
- `MYELIN_SESSION_L2_PLAN.md:78-79` — DisputedBundle 在协议层定义为 "one challenged chunk plus all data needed to recompute the claimed state transition",**没有 signer field**;
- 同 plan:181-184 "court-bundle materialises the disputed chunk into a self-contained replay bundle" — "self-contained" 进一步暗示不需要外部身份。

**影响**:bundle verifier 是 *correctness* 的 verifier,不是 *authenticity* 的 verifier。在 closed-committee 协议层里,authenticity 由 committee 的 finality evidence 隐式提供(`block_hash` is bound to `commit.chunk_index`),但 **dispute path 上** 这意味着:
1. Challenger X 构造 DisputedBundle for chunk i;
2. Operator Y 也构造 DisputedBundle for chunk i;两者都让 verifier 报 valid;
3. 协议层没有 "X came first" 的协议层 binding。

这与 F-COURT-01 联动放大 griefing 面。

**建议方向**:协议层在 `DisputeBundle` 增加 "提交者签名" 字段(可以是 participant_pubkey_hash,也可以是 operator_service_pubkey_hash),并在 `verify_session_court_bundle` 的 coverage matrix 中加 `bundle-signed-by` field。

#### F-COURT-04 (HIGH) — `identity(field(...))` 在协议层是 typed-cell metadata,不是承诺;`cellscript/examples/myelin/*.cell` 的 `identity(field(da_manifest_hash))` / `identity(field(intent_hash))` 在 on-chain 没有 enforcement

**维度**:严密度
**严重度**:HIGH

**观察**:deleted bd0571b 仓库 `cellscript/examples/myelin/da-anchor-final.cell:1-12` 与 `cellscript/examples/myelin/settlement-final.cell:1-12`(已通过 `git show 8661e1b` 找到原文)声明:

```text
resource DaAnchorFinal has store, create
    identity(field(da_manifest_hash))
{
    da_manifest_hash: Hash,
    court_bundle_hash: Hash,
    ...
}
```

`identity(field(...))` 的协议层语义在 cellscript 编译器中是 **typed-cell 标识符**:它声明这个 type-cell 的 canonical identity 由 `field(da_manifest_hash)` 决定,field 是 cellscript 的 32-byte hash 字段。`identity(...)` 在 cellscript 语言层会被 codegen 注入到 metadata sidecar 与 typed-cell manifest 中。但 **typed-cell identity 在 CKB 协议层**没有 enforcement——`exec/src/celltx/types.rs` 在 main branch 里没有 `TypedCellDecl` for `SettlementFinal` / `DaAnchorFinal` / `DaAnchorCarrier` / `SettlementCarrier`(这是 WHOLEREPO F-DOC-05 / F-PRIM-FIXTURE-ORPHAN 的事实层)。即:`identity(field(da_manifest_hash))` 与 `identity(field(intent_hash))` 在 on-chain 是 cell data 解析 + type-script 校验的产物,**不是**协议层 binding。

`verify_final_da_publication` 与 `verify_final_settlement`(deleted bd0571b fixture)实际上以 on-chain 形式绑了 `da_manifest_hash` / `intent_hash`(通过 `ckb::cell_data_hash_at(output, 0)` 与 type args),所以 **on-chain 协议层 binding 是 verifier 的 memory-side 强验证**,不是 typed-cell identity 的声称;**但协议层混淆了 typed-cell identity 与 on-chain verifier 的语义差异**。

**证据**:
- deleted bd0571b `cellscript/examples/myelin/da-anchor-final.cell:1-12` — 资源声明带 `identity(field(da_manifest_hash))`;
- deleted bd0571b `cellscript/examples/myelin/settlement-final.cell:1-12` — 资源声明带 `identity(field(intent_hash))`;
- main branch 当前状态 — `cellscript/examples/myelin/*.cell` 全部被 merge `8661e1b` 删除;
- `exec/src/celltx/types.rs` (main branch) — 无对应 `TypedCellDecl`(WHOLEREPO F-DOC-05);
- `cellscript/examples/language/v0_15_identity_lifecycle.cell:9,15,22` 是 cellscript **语言层的** typed-cell identity 教学,**与 court subsystem protocol 无关**;
- `cli/src/main.rs:8093-8095` — schema 区分 `myelin-session-ckb-final-script-submission-v1` 与 `myelin-session-ckb-carrier-submission-v1`,但没绑 `identity(...)` 字段;
- `cli/src/main.rs:4584-4592` — `carrier_payload_type_args_hex` 只接 `myelin-session-da-anchor-carrier-v1` 与 `myelin-session-settlement-carrier-v1`,**没有**`myelin-session-da-anchor-final-v1` / `myelin-session-settlement-final-v1`(WHOLEREPO F-CLI-01)。

**影响**:
1. **协议层"single instance" 主张没有 typed-cell identity enforce**:`da-anchor-final.cell:39-41` 中 `if ckb::cell_exists(source::group_input(0))` 与 `if ckb::cell_exists(source::group_output(1))` 是 on-chain runtime checks,**它们** enforce same-type group 单例化,不是 typed-cell identity。
2. **`identity(field(da_manifest_hash))` 误读风险**:downstream tooling 可能把 typed-cell identity 当作 on-chain commitment,但实际上 on-chain commitment 来自 `ckb::cell_data_hash` + type-script verification,完全是另一条语义路径。
3. **`cellscript/examples/myelin/` 目录在 main branch 中不存在**(全 4 个 fixture 均被 deleted by merge `8661e1b`),cellscript v0_18 测试 `v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper` 与 `v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm` 在 main branch 中也不再存在(`git log -G "v0_18_myelin" -- cellscript/tests/v0_18.rs` 不返回结果;`cellscript/tests/v0_18.rs` 当前 851 行,没有 :898-925 的 final-script test)。

**建议方向**:
- 协议层文档需要明确 "typed-cell identity 是 compilation-time metadata,on-chain enforcement 来自 type-script + cell_data + type_args 三角"——不能并列放在 `DisputedBundle` 验证语义中。
- 如果 typed-cell identity 是协议层承诺,则 `exec/src/celltx/types.rs` 必须有对应 `TypedCellDecl` 注册表,且 `MyelinBlock.cell_deps` 在协议层必须 reference type-cell library hash。

#### F-COURT-05 (HIGH) — court economics 的 `fee_policy` 在协议层只声明,不由 court_economics 子系统 enforce

**维度**:严密度
**严重度**:HIGH

**观察**:`cli/src/main.rs:4131` 声明 `fee_policy: "submission-economics-report-enforces-ckb-fee-floor-rate-and-max-fee"` —— 这是 court_economics evidence 内的一个 protocol-layer string commitment。但 `court_economics_evidence` 与 `court_economics_base_commitment` 的 blake3 commitment algorithm(:4147-4149)只覆盖 16 个字段,**不覆盖** CKB fee 的 rate / floor / max-fee 字段(它们在 `session_carrier_submission` 的 `submission_economics` 子系统由独立 verifier 检查)。

**证据**:
- `cli/src/main.rs:4131` — `fee_policy` 是字面字符串,不是 binding;
- `cli/src/main.rs:4147-4149` `economics_commitment_algorithm` — blake3 输入是 `participant_set_hash, escrow_input_cells_hash, challenge_payload_hash, da_availability_commitment, challenge_window_ms, challenge_deadline_ms, minimum_dispute_bond_shannons, challenger_reward_bps, loser_slash_bps, honest_party_refund_bps, unresolved_remainder_bps, settlement_after_deadline_only, da_evidence_required`,**没有** fee rate/floor/max;
- `submission_economics_*` 是在独立的 CLI 子系统 enforce(per `MYELIN_PRODUCTION_GATE.md:217-223`),court_economics commitment 不重复 enforce。

**影响**:
1. **协议层 fee policy 是 paper-only** —— 在公共 testnet 上当 `submission_economics_*` 与 court_economics 同时出现,`fee_policy` 字符串不一致时,**协议层无法检测**(只在 `submission_economics_*` 检查中失败)。
2. `d1bb6f7` "Mark court economics as commitment-only evidence" 的承诺(self-disclosed: ":court economics are a deterministic policy commitment without claiming complete dispute-economics readiness")在协议层只是 *commitment* 不是 *enforcement* —— fee_policy 的语义降级与 `testnet_beta_ready = false` 的语义降级是同级的降级。

**建议方向**:
- 协议层应在 court_economics commitment algorithm 字符串中显式承诺"fee_policy is enforced by submission_economics_*";或在 court_economics evidence 中加 `fee_policy_floor_shannons`,与 CKB submission economics 子系统的 floor 字段绑定。

#### F-COURT-06 (HIGH) — `court_economics_deployment_evidence` 在协议层是 stale-deployment protection,但 stale-detection key 没有 protocol-layer guidance

**维度**:严密度
**严重度**:HIGH

**观察**:commit `4612677` "Reject stale court economics deployment commitments" 在协议层引入 `verified_session_id` / `deployment_commitment_algorithm` 等字段,使 `normalize_court_economics_deployment_evidence` 在 `cli/src/main.rs:4243-4335` 上做的 stale detection 是 **deploy-time-grounded** 的——但 stale 的含义是什么?**协议层文档没有定义** "staleness window"。例如:
- 一个 30 天前的 deployment,scared a user 给出的 "production_ready = true"(verify 时)还能不能被用来 bind 今天的 settlement?

**证据**:
- `cli/src/main.rs:4230-4335` — `normalize_court_economics_deployment_evidence` 函数实现 stale 检查;
- `cli/src/main.rs:4326-4340` — `court_economics_deployment_commitment` 重算 only on `verified_session_id + verified_deployment_evidence`;
- `cli/src/main.rs:4264-4270` (verify path) — `expect("normalized court economics deployment is internally verified")` 等 assert 在 production code;**stale 不在 protocol layer declaration**,只在 CLI implementation。
- `cellscript/examples/myelin/*.cell` 在 main branch 不存在,因此 "verified_deployment_evidence 的 staleness window 由 fixture 决定"是 noise。

**影响**:operator 拿到一份 1 年前的 court_economics deployment evidence,而且它的 source hash 还匹配当时的审计报告,但当前 CKB 节点上 `code_dep` 已经被替换/重部署——协议层是不是允许这种 stale evidence 用于 generate a settlement?**协议层答:在 main branch 上没有强制 staleness window**。

**建议方向**:
- 协议层应在 `SessionCourtEconomicsDeploymentEvidence` schema 上加 `evidence_validity_window_ms` 字段,verifier 必须接受 `|now - evidence_timestamp_ms| < window`。
- 协议层应声明 `stale_at_block_height`(由 deployment package 上的 script_since_cell 锚定)。

#### F-COURT-07 (MEDIUM) — chunk 拆分决定性在单次 dispute 内闭环,但跨 dispute 不闭环(同一 session 内 multiple chunk_index 没有跨竞争 binding)

**维度**:严密度
**严重度**:MEDIUM

**观察**:`chunk_index` 是 dispute 的唯一 anchor(`cli/src/main.rs:5781-5786`),同一 bundle 内 chunk 拆分是确定的(从 `chunk_index * chunk_bytes` 取 offset,见 `cli/src/main.rs:1923-1926` 在 `teeworlds_chunk_cell_tx` 中,迁移到 session 时是 `chunk_index.to_le_bytes()` 参与 commitment hashes,`:5731-5733`)。但**跨** dispute:同一 session 可能在不同时间被不同对手用不同 `chunk_index` 发起 multiple disputes;协议层没有"先来后到"的 `dispute_id` / `dispute_sequence_no` 字段。

**证据**:
- `cli/src/main.rs:7052` `kind: kind.to_owned()`,`kind = "disputed-close"` 是仅有的语义;没有 dispute sequence 字段;
- `MYELIN_SESSION_L2_PLAN.md:78-80` `DisputeBundle` 定义只有 chunk 没有 sequence;
- `cli/src/main.rs:5778-5786` chunk_index 是单字段;
- `cli/src/main.rs:5728-5733` `chunk_index.to_le_bytes()` 进入 `myelin:session-fixture-*:v1` commitment 域,但是 这是 **chunk-level commitment 不是 dispute-level commitment**。

**影响**:同一 session 在不同窗口下被多次 dispute,协议层没有办法机械 sequentialize——challenger 可以 stale-replay 一个旧 dispute 来重置 challenge window。

**建议方向**:
- `DisputedBundle` schema 加 `dispute_sequence_no` 字段,与 session_id 一起做成 `dispute_id = blake3(session_id || dispute_sequence_no)` 并在 settlement_intent 中 binding。

#### F-COURT-08 (MEDIUM) — `note` 字段在协议层是 human-readable,但 path 代码把 note 当作 protocol semantics

**维度**:严密度
**严重度**:MEDIUM

**观察**:`session_court_bundle` 在 `cli/src/main.rs:5872-5875` 设置 `notes: vec!["...", "..."]`,这两个 note 是 string-encoded protocol-layer disclosure:
- `"This is a deterministic Session L2 disputed-chunk court bundle."`
- `"It proves the chunk can be replayed as a CKB-compatible CellTx; it is not yet an on-chain CKB court script."`

**协议层**:`note` 字段是 human-readable,**但** `l1_court_implemented: false`(:5871)是协议层 binding,这两类声明由不同字段承载,在协议层 verify 时 `verify_session_court_bundle` 不检查 `note` 的字符串值(只在 protocol-layer 检查 `bundle.court_verifiable` + projection + block binding)。

**证据**:
- `cli/src/main.rs:5871-5875` — `l1_court_implemented: false` 与 `notes: vec![...]`;
- `cli/src/main.rs:5883-6097` — `verify_session_court_bundle` 不 push_check `note` 字段;
- `MYELIN_SESSION_L2_PLAN.md:184` "prove CKB-compatible projection, finalize in CKB-strict profile" 是 plan 文本,不是 JSON 协议字段。

**影响**:`note` 不是协议层 invariant(被 verbose 接受),`l1_court_implemented: false` 是 — 这意味着工程实现层可以**修改 note 文本**而 verifier 仍 pass;但反过来,`note` 文本声称的 "CKB-compatible" **被协议层期待作为 audit trail**。工程误改这字段不被检测。

**建议方向**:
- 协议层或者收敛 `note` 为可选 verbose-only 字段(如明确的 `audit_trail: Vec<{code, message}>` 二元组),或者把 note 的内容作为 blake3-bound `protocol_disclosure_hash` 字段。

## 3. 合理性评估

### 3.1 总体

Court 子系统的"final-script in protocol-layer"角色在 main branch **不明确**:删除 fixture 后,协议层的 "final-script" 仅由 CLI `submission_schema` enum 暗示(`cli/src/main.rs:10344` 处 `final_l1_script = schema == "myelin-session-ckb-final-script-submission-v1"`)。即:
- 协议层的 "carrier submission" 与 "final-script submission" **只在 CLI 层区分**,没有 protocol-layer marker(没有 `DisputedBundle.final_l1_script: bool`)。
- 协议层的 "disputed-close" 与 "normal close" / "timeout exit" / "abort" **只在** `kind` 字段区分;代码 `:6976-6978` hardcodes "only disputed-close",意味着 "normal close" / "timeout exit" / "abort" 三个 settlement kind 在协议层 **不存在**(per `MYELIN_SESSION_L2_PLAN.md:80` 列表中的其他项在 CLI 中未实现)。

这些在协议层是 "scope cut",但**没有 architectural guidance**说明为什么 "normal close" 不需要走 court bundle(可能可以但 plan 没解释)。dispute window / challenge period 在 CLI 层定义,协议层没说 minimum / recommended / use the same window for all decisions。

在静态委员会下 dispute path **在协议层是有意义**的(static committee 的 finality 是 "我同意这条 chunk 是 final",但 "chunk 是否合法" 是法院问题;两者正交,法院不应该看 committee,委员会不应该拒绝法院审查)。court 协议层的 chain-of-custody 是:challenger 出 court bundle → JSON verify-court-bundle → bundled replay → settlement-intent 时被允许(以 challenge window 决定)。**整个 chain 在协议层是 self-service**——没有 "external judge" 角色,这是 static-committee 的合理选择(因为委员会是 closed 的,不需要外部 oracle),但 **plan 与 README 在 protocol-layer narrative 上没说清楚这一点**,这是 meaningful protocol-layer omission。

court bundle 完整性在协议层是 **off-chain verification 与 on-chain readiness 共同声明**:`l1_court_implemented: false` (protocol layer) + `bundle.court_verifiable + ckb_projection.ckb_projection_possible + ckb_projection.semantic_profile == "ckb-compatible"`(verifier check at `:6084-6094`)是协议层的 **不变量 surrogate**:不能在协议层 commit 一个 "permitted to submit on-chain court" 的状态,而只能 commit "verifies off-chain" — 即**协议层把 court bundle 定义为 off-chain 验证 + on-chain readiness marker 的二元结构**,没有第三层(实际 on-chain 部署)。

### 3.2 Findings (合理性)

#### F-COURT-09 (HIGH) — settlement kind 的 protocol layer coverage < plan 声明

**维度**:合理性
**严重度**:HIGH

**观察**:`MYELIN_SESSION_L2_PLAN.md:80` 声明 `SettlementIntent` 的 4 种 kind:"normal close, disputed close, timeout exit, or abort"。但 main branch 代码 `cli/src/main.rs:6976-6978` 严格限制 `kind != "disputed-close"` 为错误 `unsupported settlement intent kind`,**`normal close / timeout exit / abort` 三种 kind 在协议层不存在**。

**证据**:
- `MYELIN_SESSION_L2_PLAN.md:80` — 4-kind declaration;
- `cli/src/main.rs:6976-6978` — `if kind != "disputed-close"` 是 hard fail;
- `cli/src/main.rs:5069-5071` `l1_court_implemented: bool` 是 bundle field;
- `MYELIN_PRODUCTION_GATE.md:50` — gate 仅跑 fixture 中"open/commit/court/DA/settlement",**没有** normal close / timeout exit / abort 的 fixture。

**影响**:
1. 协议层 "SettlementIntent" 是 4-kind 是 plan 的承诺,但代码只 1-kind,plan 与代码不一致。
2. `normal close` 是 happy-path,`timeout exit` 是安全释放,`abort` 是异常 —— 三个 path 都不在 protocol layer 里,意味着 **用户只能 dispute 或 stuck**。

**建议方向**:
- 协议层需在 `session_settlement_intent` 路径上接受其他 3 种 kind,或在 plan 中撤销 4-kind declaration。**当前状态是 "plan says 4, code says 1"**,这是 protocol-narrative drift。

#### F-COURT-10 (HIGH) — final-script fixtures 在 protocol-layer 是 required closure 但只在 CLI 层区分

**维度**:合理性
**严重度**:HIGH

**观察**:`MYELIN_SESSION_L2_PLAN.md:288-291` 声明 final settlement type args `session_id_hash || settlement_identity_hash`,并要求 "transaction-local singleton creation; cross-transaction replay is blocked by consuming the one-use authority Cell"——即 **final-script 是 protocol-layer required closure for the settlement subsystem**(settlement_local_singleton + cross_replay_blocked = protocol layer claim)。但:
1. main branch 删除 fixtures;
2. CLI 没有 helper(`myelin-session-da-anchor-final-v1` / `myelin-session-settlement-final-v1`)— `carrier_payload_type_args_hex` 在 `cli/src/main.rs:4584-4592` 没有 final kind;
3. `--verifier-role final-l1-script` 没有 helper(`scripts/myelin_public_testnet_rehearsal_live.sh:94-132` per WHOLEREPO F-DOC-03)。

协议层 required,但 CLI/operator 没法满足——这是 **合理性下降**:final-script 是 architecture-level role,但 operator reachable surface 只有 carrier submission。

**证据**:
- `MYELIN_SESSION_L2_PLAN.md:288-291` — final settlement protocol layer requirement;
- `cli/src/main.rs:4584-4592` — only carrier kinds;
- `MYELIN_SWARM_AUDIT_WHOLEREPO.md:170-220` (XD-01 cross-cutting) — same finding;
- `cli/src/main.rs:8093-8095` `schema` enum 区分 final-script 但没绑命令。

**影响**:
1. final-script 是 protocol-layer required closure,**但 protocol-layer 没有 path 让 operator 满足这个 closure**。
2. `live_l1_script_submission_ready` 在 `cli/src/main.rs:10344` 仅由 schema 决定,没有 "the underlying protocol-layer commit is cross-tx-bounded"的 binding。

**建议方向**:
- 协议层应在 `MyelinBlock` schema 上加 `finalization_path_kind: "carrier" | "final-script"` field,而不是把它隐式在 submission_schema enum 内。

#### F-COURT-11 (MEDIUM) — static committee finality + dispute path 是 closed system 不需要外部 oracle,plan/README 没明确声明这是 architecturally intentional

**维度**:合理性
**严重度**:MEDIUM

**观察**:static closed committee(`MYELIN_SESSION_L2_PLAN.md:94-99`)是 closed validator set。这意味着 court 协议层可以是 **self-contained**——disputed chunk replay 由 replay tool 验证,**不需要**外部 oracle 决定 "这条 chunk 是合法还是非法"。但 plan 没有 explicit declaration:"static committee 是 chosen to enable self-contained court path";README 与 positioning 也未强调这一点。读者会自然地认为 "court 应该有 external judge",这与 closed-committee 选择的 protocol model 不一致。

**证据**:
- `MYELIN_SESSION_L2_PLAN.md:94-99` — declares "finalises with static committee" 是 fixture-acceptance;
- `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:33-34` — "Static closed committee and Tendermint fixture certificates | Fixture";
- `MYELIN_SESSION_L2_PLAN.md:107-132` — P1 章节明确 "court-facing execution narrower than general Myelin VM path" 但没说 "court can be self-contained";

**影响**:
1. 协议层 "dispute path is self-contained" 应是 protocol-narrative 的一部分,而不是 implementation-detail。
2. Tendermint 模式下 dispute 路径仍然是 self-replay,**没有改变** (per plan:548-549 "The state transition is consensus-independent; only finality evidence differs"),但 **plan 没有声明 finality evidence 是 committee-dependent 而 dispute 是 consensus-independent**。

**建议方向**:
- Protocol-narrative doc 章节明确 "court path 的 protocol design 不依赖 committee 拓扑,只依赖 finality_evidence 字段"。

#### F-COURT-12 (MEDIUM) — court bundle 完整性在协议层是 off-chain verify + on-chain readiness marker 二元结构,plan 与代码没声明 protocol invariant 是这两者之一还是两者

**维度**:合理性
**严重度**:MEDIUM

**观察**:`l1_court_implemented: false` + `bundle.court_verifiable` 是 court bundle JSON 的两个核心字段。前者是 protocol-layer "on-chain readiness marker",后者是 off-chain verification 的 audit 标记。但在 `MYELIN_SESSION_L2_PLAN.md:316-318` 中:"Settlement intent verification passes only after the configured challenge window has elapsed, and the report remains explicit that L1 court settlement is not implemented" —— **plan 文本**没有协议层 naming convention,这是 narrative vagueness。

**证据**:
- `cli/src/main.rs:5871` `l1_court_implemented: false`;
- `MYELIN_SESSION_L2_PLAN.md:316-318` — plan text 关于 L1 court settlement 没实现;
- 协议字段命名 in plan 与代码之间不一致:代码 `l1_court_implemented`,plan 文本 "L1 court settlement not implemented"。

**影响**:
- 协议层 invariant 名 `bundle.court_verifiable + l1_court_implemented` 是 `false + true` 还是 `false + false` 还是 `true + false` —— plan 没说。
- 实际 main branch fixture 是 `court_verifiable: true + l1_court_implemented: false` —— 即 off-chain verify passes 但 on-chain 不存在。在协议层 readable label 是 "Ready for protocol review, not for L1 publication"。

**建议方向**:
- 协议层 invariant 命名明确:`bundle.court_verifiable` → `bundle.off_chain_verifiable`,`l1_court_implemented` → `l1_court_script_published` 等。

#### F-COURT-13 (MEDIUM) — challenge_window_ms 协议层声明是"duration not meaningful",实操协议是 fixture 60_000 ms,但 main branch 没 fixture 在 protocol-layer documentation

**维度**:合理性
**严重度**:MEDIUM

**观察**:`MYELIN_SESSION_L2_PLAN.md:159` 显式以 `--current-time-ms 60000 --challenge-window-ms 60000` 作为 expected usage,prod gate 用相同 fixture(MYELIN_PRODUCTION_GATE.md:50)。**协议层**:`challenge_window_ms == 60_000` 是 fixture value,不是 protocol constant;其他 production runbook 仍不 commit 到 minimum/recommended。

**证据**:
- `cli/src/main.rs:524` `default_value_t = 60_000` — fixture default;
- `MYELIN_SESSION_L2_PLAN.md:159` — fixture fixture;
- `docs/public-testnet-rehearsal-runbook.md:264-266` per WHOLEREPO F-DOC-07 — "hardcodes 60000, no guidance for real sessions"。

**影响**:
- Protocol-layer "challenge window" 是个 **advisory string** — 在 implementation layer 是 CLI arg,但在 protocol layer 是 fixture value。
- 没有 minimum / maximum / recommended 时间常数的 protocol guidance。

**建议方向**:
- 协议层应在 spec 中明确 "typical challenge window is 60s for off-chain-only, 24h for mainnet rehearsal, 7d for mainnet"。

#### F-COURT-14 (LOW) — court economics 是 commitment-only evidence(`d1bb6f7`);这是 protocol-layer degradation 但自我披露是好的

**维度**:合理性
**严重度**:LOW

**观察**:`d1bb6f7` commit message: "Mark court economics as commitment-only evidence" — 这是 self-disclosed degradation。Wholerepo F-CLI-28 已经 capture 此点的 implementation 端,本 audit 关注的是 **protocol-layer** side:`economics_invariant_checked` 是真正的 invariant check,`testnet_beta_ready` 与 `production_ready` 在协议层是 **self-disclosed boolean**,**协议层没有** enforce 它们。

**证据**:
- `git log --oneline d1bb6f7` — 自我披露;
- `cli/src/main.rs:4098-4145` `economics_invariant_checked` logic;
- `cli/src/main.rs:4153-4154` `testnet_beta_ready: false, production_ready: false` 是 hardcoded default。

**影响**:
- commit 名称清晰(`commitment-only`),因此是 **self-disclosed degradation**,protocol layer 是 self-aware。这是该 commit 的 positive side。
- 但 plan text 与 README 没有明确标注 "court economics is commitment-only in protocol layer",即 narrative drift。

**建议方向**:
- Plan doc 加 explicit paragraph "court economics 是 commitment-only;不 claim complete dispute-economics readiness"。

## 4. 安全性评估

### 4.1 总体

Court 子系统在 **Molecule bytes + wtxid + block-hash 三位一体** 方面是加密防伪的,但在 **dispute path 的对抗层级** 上有几个突出的 protocol-layer 漏洞:
1. **dispute path 是 anon**(F-COURT-03)
2. **chunk_index 是 sole identity**(F-COURT-01)
3. **challenge window 是 duration-not-meaningful**(F-COURT-02)
4. **identity(field(...)) 没有 enforcement**(F-COURT-04)

`identity(field(...))` 在协议层没有 on-chain enforcement 的安全后果:
- `cellscript/examples/myelin/da-anchor-final.cell:1-12` 中的 `identity(field(da_manifest_hash))` 是 typed-cell 标签,而 verifier `verify_final_da_publication`(`git show 8661e1b -- ...)`:19-43 的 deleted fixture 中 on-chain 实际 binding 是 `ckb::cell_data_hash_at(output, 0) == da_manifest_hash` + `ckb::require_cell_type_args_suffix_hash(output, da_manifest_hash)` + `script::require_cell_type_matches(output, expected_type)` —— 三重 binding 而**不**依赖 `identity` 声明。
- **安全后果**:`identity` 注解的 falsified-on-paper 攻击无 on-chain consequence。但删除 fixture 后,**即使在 fixture 保留的版本里**,`identity` 注解也是 audit metadata,不是 L1 protocol 保证。

`d1bb6f7` 把 court_economics mark 为 commitment-only 的安全后果:
- **commitment-only** 在协议层是这样:**dispute_economics_policy 在协议层是 declared 但不被 enforced**。攻击者可以 claim 一个 illegal dispute_economics policy(例如 `challenger_reward_bps = 0, loser_slash_bps = 0`),其 commitment 与 normal commitment 都是 valid(因为 bps 都是合法的 u16)。
- 自从 `economics_invariant_checked = payout_balance_bps == loser_slash_bps`(`cli/src/main.rs:4103-4107`);该 invariant 强制 `challenger_reward_bps + honest_party_refund_bps == loser_slash_bps` —— **attacker 不能 fuzz bps 而保持 commitment valid**,invariant check 是 invariant。但这只是 invariant,**不是 enforcement**(commit 之后没有 settle-side enforce)。**`fee_policy` 也不在 invariant 内**(F-COURT-05)。

在 static committee 下 dispute 决议被委员会多数否决:协议层 `l1_court_implemented: false` + committee finality_evidence 的 intervention 在 challenge window 期间没有 escalate path——committee 已是 final 的,法院是 separate path(off-chain JSON verify)。这意味着 committee **不能** 在 protocol layer 阻止 dispute path;court 协议层是 **平行通道**,不是 committee 的 extension。这是 the architecture 的 good design choice(没有 court overrule committee 的 hidden mechanism),但 plan 与 README 没提到这个 protocol-layer invariant。

### 4.2 Findings (安全性)

#### F-COURT-15 (CRITICAL) — challenge window griefing:attacker 可以 batch-generate bundles 用合法 `chunk_index` 重复 dispute 同一 chunk,defender 的 settlement 永远 atomic / cancelable

**维度**:安全性
**严重度**:CRITICAL

**观察**:plan 命令 `--current-time-ms 60000 --challenge-window-ms 60000`(`MYELIN_SESSION_L2_PLAN.md:159`)假设一个 dispute = 一个 challenge window,但 main branch 代码 `cli/src/main.rs:6976-7050` 中 `settlement_permitted = current_time_ms >= block.timestamp_ms + challenge_window_ms` 是 **单窗口检查**,没有 "正在 dispute 进行中" 的状态机。每个新生成的 settlement_intent 都重新计算 `current_time_ms >= challenge_deadline_ms`,**任意次数**。

**证据**:
- `cli/src/main.rs:7000-7004` — `settlement_permitted` 是 `current_time_ms >= challenge_deadline_ms`,无 in-progress state;
- `cli/src/main.rs:7019-7027` — `court_economics_evidence_with_deployment` 重算 challenges 字段;
- `cli/src/main.rs:7319-7488` (`session_settlement_package`) — `if !intent.settlement_permitted` 是 only check;
- `cli/src/main.rs:5059-5077` — `SessionSettlementIntentReport` 字段没有 "dispute in progress"字段;
- `cli/src/main.rs:5778-5786` — chunk_index 是 only per-bundle identity,**没有** ongoing-dispute 字段。

**影响**:
1. **Griefing path**: challenger 提交 court bundle → 等待 → defender 开 settlement_intent 时 `settlement_permitted = true` → challenger 立刻发起 *新* dispute(虽然 finality_evidence unchanged) → defender 重开 settlement_intent 时 `current_time_ms` 重置 `challenge_deadline_ms = block.timestamp_ms + challenge_window_ms`,**`challenge_deadline_ms` 一致**(因为 anchor 到 block.timestamp_ms),所以重新 settlement_intent **不需要重新 reset challenge window**。
2. 即:**协议层目前实际上没有 protocol-level dispute lifecycle**,只有 single-blundle + window check。Griefing 攻击的 reward 是 mitigation:defender 重发 settlement_intent 的成本 ≈ 1 个 blake3 + CPU。
3. **Protocol lane 没有 escalation**:defender 不能通过 protocol-level action 叫停一个持续的 dispute。

**建议方向**:
- 协议层应在 `DisputedBundle` 加 `dispute_status: "open" | "resolved" | "withdrawn"` 字段,并在 session 上加 `latest_open_dispute` 状态;settlement_intent 必须 reference 唯一 `dispute_id`。

#### F-COURT-16 (CRITICAL) — challenge window minimum-policy 缺位,defender 没有 protocol-level anti-griefing 工具

**维度**:安全性
**严重度**:CRITICAL

**观察**:**`challenge_window_ms == 0` 是** block by code(`cli/src/main.rs:6979-6980`),"1 millisecond" 是 **不** block。**`challenge_window_ms` 没有 minimum protocol policy**。

**证据**:
- `cli/src/main.rs:524` — `default_value_t = 60_000` 但 `#[arg(long)]`,**接受** `--challenge-window-ms 1`;
- `cli/src/main.rs:6979-6980` — 只有 `challenge_window_ms != 0` 拒绝;
- `MYELIN_SESSION_L2_PLAN.md:159` — fixture 60000,**没有** "minimum challenge window"声明;
- `cli/src/main.rs:4105-4107` — `economics_invariant_checked` 要求 `challenge_window_ms > 0` 和 `challenge_deadline_ms >= challenge_window_ms`(self-disclosed invariant),但这是 economics-layer,**不是 protocol layer minimum policy**。

**影响**:
1. **1-millisecond griefing**:attacker 设 `--challenge-window-ms 1`,立刻通过 `settlement_permitted = true`,SettlementIntent 提交给 defender 时 defender 的 `verify_session_court_bundle` 检查 pass(因为 challenge_payload_hash 不随 window 而变),但 defender 在 millisecond 内无法发送任何 anti-dispute response。
2. **Negative minimum**:defender 没有 protocol-layer deadline-by-which-dispute-can-be-challenged-after-settlement,**SettlementIntent 一旦被 verifier 接受,settlement 路径只在 `verify_session_settlement_intent` 内 check,但 finality 是 JSON submission 不是 chain finality**。

**建议方向**:
- 协议层在 `session_settlement_intent` 路径增加 `minimum_challenge_window_ms` 检查(minimum = e.g. `min(60_000, session_timeout_ms / 2)`,且要求 `challenge_window_ms >= settlement_authority_threshold_lock_args_max_revocation_delay_ms`)。
- 协议层应增加 "post-settlement 冻结期"(`finalization_grace_period_ms`)在 settlement package 路径,允许 challenge 在 settlement_intent 通过 settle **后** 的一段时间内仍可 escalate。

#### F-COURT-17 (HIGH) — `court_economics_deployment_evidence` 的 stale-detection 缺 protocol layer time window

**维度**:安全性
**严重度**:HIGH

**观察**:F-COURT-06 已经从严格性角度分析。本节从安全性角度:**staleness 没有 protocol layer enforcement window**,这意味着 deployment evidence 可被 forever-replay。operator 可以 reuse 一年前的 deployment evidence 用于 generate a settlement,**protocol layer 不拒绝**。

**证据**:
- `cli/src/main.rs:4230-4335` (`normalize_court_economics_deployment_evidence`) — 历史逻辑;
- `cli/src/main.rs:5095-5102` `SessionCourtEconomicsDeploymentEvidence` 字段 — 没有 `evidence_validity_window_ms`;
- 同 main branch fixture 是 `now() < recent_block_height` 但 protocol layer 没有 block-height-driven staleness。

**影响**:
- **Replay attack**: attacker 拿到一份 stale deployment evidence,replay 它 generate 一个 economics_evidence production_ready = true 的 settlement,**detect 不到**。
- **Forensic attack**:即使 stale evidence 用过,protocol layer 没有 audit trail 指明 "which version deployed at which block"。

**建议方向**:
- 协议层应在 `SessionCourtEconomicsDeploymentEvidence` 加 `evidence_observed_at_block_height` 和 `evidence_validity_window_blocks`,verifier 检查 stale。

#### F-COURT-18 (HIGH) — `cellscript/examples/myelin/*.cell` 在 main branch 不存在,smoke script 是 broken-dead-on-arrival

**维度**:安全性(广义)
**严重度**:HIGH

**观察**:deleted bd0571b `cellscript/examples/myelin/{da-anchor-carrier,settlement-carrier,da-anchor-final,settlement-final}.cell` 在 merge `8661e1b` 删除。**`scripts/myelin_ckb_devnet_smoke.sh:114-117` 与 `scripts/myelin_public_testnet_rehearsal_prepare.sh:80-83` 仍 cp 这些文件**(read-by-cp-from-source)。

**证据**:
- `git show --stat 8661e1b -- "cellscript/examples/myelin/*"` 删除四个 fixture;
- `git ls-files cellscript/examples/myelin/` 返回空;
- `rg "cellscript/examples/myelin" scripts/` 返回 8 matches,全部 cp 行;
- `cli/src/main.rs` 没有对应 `cp` —— 这是 smoke-only 依赖。

**影响**:
1. **Protocol layer silent regression**:smoke script 在 main branch 不可执行。**协议层** "live carrier submission ready" 在 smoke 路径上 broken,但 main branch 没 fix it。
2. **Cellscript v0_18 测试也 broken**:`cellscript/tests/v0_18.rs` 851 行,**没有** :898-925 final-script test(per WHOLEREPO F-DOC-23 / F-DOC-25 引用)。这意味着 cellscript test 在 main branch **不覆盖** final-script protocol binding。

**建议方向**:
- 协议层判断:fixture 是 protocol-layer artifact,merge delete 应该 follow-up fix smoke + tests。
- This is also a hygiene point:协议层 commitment 在 v0_18 test 中存在(或曾经存在)意味着 test path 是 protocol-layer evidence. 删除 fixture 后 protocol layer 在 test-side 失 commit。

#### F-COURT-19 (MEDIUM) — `da_availability_commitment` 进入 court_economics commitment algorithm,**如果** `da_availability_production_ready = false`,court_economics commitment 仍然 generate 但 settlement 的承诺是 stale-DAP

**维度**:安全性
**严重度**:MEDIUM

**观察**:`court_economics_base_commitment` 在 `cli/src/main.rs:4170-4178` 输入 `da_availability_commitment`,这是 DA 子系统的 availability commitment。**该 commitment 在 fixture path 与 production path 上语义不同**:
- fixture path:`da_availability_production_ready = false`(per WHOLEREPO F-CLI-01)
- production path:`da_availability_production_ready = true`(per commit `3fda2ab` 重新 compute)

但 court_economics commitment **不区分** 这两种 da_availability_commitment——它在 protocol layer 不 bind "production readiness",只 bind "availability commitment bytes"。

**证据**:
- `cli/src/main.rs:4170-4178` `court_economics_base_commitment` input 含 `da_availability_commitment` bytes;
- WHOLEREPO F-CLI-01: gate dry-run 路径 assert `production_ready = false` 但 fence 后 acceptance is True;
- LANE_DA report:`da_availability_production_ready` 不是 protocol-layer invariant(F-DA-15)。

**影响**:
- **Stale DA rebind**:court_economics commitment algorithm 用 stale da_availability_commitment,defender 不能 detect。
- **Protocol layer should refuse**: `session_settlement_intent` 时 should check `da_availability_production_ready = true`,目前没 check。

**建议方向**:
- 协议层加 `da_availability_production_ready` required check in `verify_session_settlement_intent`。

#### F-COURT-20 (MEDIUM) — challenge_payload_hash domain `myelin:session-court-challenge-payload:v1` 在协议层是 single source of truth,但 chain 中只有 clause 操作,没有 protocol-level "valid challenge reason" enum

**维度**:安全性
**严重度**:MEDIUM

**观察**:`challenge_payload_hash` 在 `cli/src/main.rs:5831-5845` 由 `session_id || chunk_index || state_root_before || data_commitment || state_root_after || scheduler_commitment || block_hash || participant_set_hash || escrow_input_cells_hash || session_lineage_commitment` 计算。`challenge_window_ms` 是 attacker-operator free 变量。

**协议层**:**谁是 challenger** 不在 challenge_payload_hash domain 内(因域结构只 hash state-level 字段)。attacker `chunk_index = X` + `chunk_index = Y` 可以 generate same-challenge-payload-hash challenges 与 operator 的 challenge,**协议层无法分辨**。

**证据**:
- `cli/src/main.rs:5831-5845` — challenge_payload_hash domain;
- `cli/src/main.rs:5846` — `court_verifiable` is binding;
- 同 main branch 没有 `challenger_pubkey_hash` 字段(per F-COURT-03)。

**影响**:
- 多 challenger 同一 chunk 的 attack detection **靠 finality_evidence + dispute_id fingerprint**,但 fingerprint 协议层不存在。
- **Mitigation**:F-COURT-15 提供了 base mitigation(if protocol layer add dispute_id)。

**建议方向**:
- 协议层应在 challenge_payload_hash domain 加 `challenger_pubkey_hash`,并在 dispute_id fingerprint 化。

#### F-COURT-21 (LOW) — fee_policy 在 protocol-layer 是 declared string,protocol layer 不能检测 disagreement between court_economics 与 submission_economics

**维度**:安全性
**严重度**:LOW

**观察**:F-COURT-05 已从严格度角度分析。安全方面:即 attacker 在生成 settlement_intent 与 submission 时,**两组 fee policy 可能 mismatch**(court_economics声明 `submission-economics-report-enforces-ckb-fee-floor-rate-and-max-fee`,而实际 submission 时 fee_policy 不同)且 protocol layer 不能检测。

**证据**:
- `cli/src/main.rs:4131` `fee_policy: "submission-economics-report-enforces-..."`;
- `cli/src/main.rs:4147-4149` `economics_commitment_algorithm` 不覆盖 fee 字段;
- `MYELIN_PRODUCTION_GATE.md:217-223` submission_economics 在独立 check;

**影响**:fee policy discrepancy 不被 protocol layer 拒绝,**只在** submission_economics 子系统拒绝。

**建议方向**:
- 协议层应在 `court_economics_evidence` 加 `fee_policy_hash = blake3(min_fee_shannons, min_fee_rate_shannons_per_kb, max_fee_shannons)`,并在 `verify_session_submission_economics` 的 fee_policy hash 比对。

#### F-COURT-22 (LOW) — `l1_court_implemented: false` 是 hardcoded;protocol layer 中 "court script exists" 状态的 falsified 的 attack 不防

**维度**:安全性
**严重度**:LOW

**观察**:`l1_court_implemented: false` 在 `session_court_bundle`(`cli/src/main.rs:5871`)是 hardcoded。这意味着 verifier **不** 检查 on-chain 真实 script 是否 deployed——只要 JSON 标记 false 就 false。**attacker 写 `l1_court_implemented: true` + 手敲一个 fake `court_bundle_hash`**,verifier 因 JSON logic 仍 reject(因为 `bundle.court_verifiable` 也需要 `ckb_projection_possible`),**但 verifier 不区分**"on-chain 部署" 与 "off-chain verified"。

**证据**:
- `cli/src/main.rs:5871` `l1_court_implemented: false` 是 hardcoded;
- `cli/src/main.rs:5870` `court_verifiable` 接受 bundle 的 value;
- 没有 `bundle.deployed_court_script` 字段。

**影响**:
- **Falsified l1_court_implemented = true** 的 attack:attacker 真把 final-script 部署到 L1,**但** 把 `l1_court_implemented: true` 与 `l1_court_script_published: true` 都设为 true;协议层 verifier 不能 detect 这是真发生 还是 attacker 写 JSON。
- 正向**:协议层把 on-chain 状态留作 operator observation,verifier 只 check JSON structure**,这是 protocol model 设计选择。但 plan 与 README 没明文决定这一点。

**建议方向**:
- 协议层应在 verify-submission-readiness 路径上 read-only query CKB RPC for `cells/code_dep`,verifier 把 on-chain state hash 与 `l1_court_implemented` bind。

## 5. 与已存在 audit 的关系

WHOLEREPO audit 报告:
- **XD-01**(`cellscript/examples/myelin/{da-anchor-final,settlement-final}.cell` 是 CLI orphan):本 lane 在 F-COURT-04 / F-COURT-10 / F-COURT-18 中吸收并升级到协议层。其协议层含义是 "final-script 在协议层 required 但 CLI layer 与 fixture layer 都没有 exposed path",这是 protocol-narrative drift。
- **F-CLI-01**(`carrier_payload_type_args_hex` 不知道 final-script payload kind,F-CLI-28 recusive):在 F-COURT-08 / F-COURT-22 中讨论其协议层含义。`fee_policy` 字段是 protocol-layer declared but not enforced。
- **F-DOC-01**(`runbook 指向 --verifier-role final-l1-script 但 live script 没实现`):F-COURT-09 讨论。protocol lane 关于 "finalization_path_kind" 字段缺失。
- **F-SCRIPT-14**(production gate 没 exercise final-script fixtures):F-COURT-18 讨论协议层含义。production gate 的 "production-ready assertion" 在 protocol lane 不是 invariant,是 operator-checked marker。
- **D1BB6F7**(Mark court economics as commitment-only evidence):F-COURT-14 / F-COURT-19 / F-COURT-21 是该 commit 的协议层延伸。`testnet_beta_ready = false` 与 `production_ready = false` 是 hardcoded default,plan narrative 不能"承诺 complete"。
- **F-CLI-28**(recursive court_economics_deployment_flags_valid):F-COURT-06 / F-COURT-17 协议层延伸。stale-detection 没有 protocol layer time window。
- **F-DOC-05**(cellscript fixtures 声明 identity(field(...))但 exec/celltx/types.rs 没 TypedCellDecl):F-COURT-04 协议层延伸。`identity(field(...))` 是 typed-cell metadata,不是 on-chain commitment。
- **F-DOC-07**(runbook hardcodes `--current-time-ms 60000 --challenge-window-ms 60000`):F-COURT-13 协议层延伸。fixture value 不是 protocol constant。
- **F-PRIM-01**(celltx/types collide on `(args="X", data="")` vs `(args="", data="X")`):不在本 lane 协议层(详见 cross-subsystem §7)。
- **F-CLI-35**(F-CLI-02 已有 review):不在本 lane。

cross-audit:
- **LANE_CONSENSUS**:不重复讨论;court subsystem 假设 finality_evidence 是由 consensus 给的 JSON text field,consensus 的 Tendermint/static-closed-committee 不同 modes 是 court 在 protocol layer 的"consensus_kind" enum(见 `cli/src/main.rs:5852` `consensus_kind: consensus_kind.as_str()`)。此处不引入共识 protocol-model findings。
- **LANE_DA**:court_economics commitment algorithm 引用 `da_availability_commitment`,F-COURT-19 与 LANE_DA 的 F-DA-15(生产 readiness 不 invariant)交叉。court subsystem 把 DA 的 freshness 当作 commitment,但 protocol layer 不能区分 fixture 与 production DA。这是 cross-subsystem 改进点。
- **LANE_SETTLEMENT**:settlement sub-system 输出 `SettlementIntent` / `SettlementPackage`,court 的 cross-section 协议层责任是 "challenge window 在协议层有没有 minimum"。F-COURT-02 / F-COURT-16 / F-COURT-19 提供了 settlement sub-system 应吸收的 protocol-layer 约束。

## 6. 已知缺陷(XD-01 / F-CLI-01 / F-DOC-01 / F-SCRIPT-14 / D1BB6F7)的协议层影响

1. **XD-01(CLI orphan)** → 协议层 required closure "final-script 在交易层 single-instance + cross-tx replay-blocked by authority Cell" 没有 on-the-wire reachable path。F-COURT-10 升级到协议层:**协议层 finalization_path_kind 字段缺失**,CLI enum 是 surrogate。Plan text "final settlement type args `session_id_hash || settlement_identity_hash`" 是 protocol claim,code 与 fixture 都不 match。

2. **F-CLI-01(`carrier_payload_type_args_hex` 不识 final-script kind)** → 协议层 "compact 160-byte payload is binding to type-args" 的 invariant **只在 carrier 路径 enforce**,**final-script 路径在协议层是 path-undisclosed**。F-COURT-22 协议层含义:on-chain final-script 状态不能在 protocol layer mechanically distinguish from off-chain verify。

3. **F-DOC-01(runbook 指向 --verifier-role final-l1-script)** → 协议层 **`finalization_path_kind` field 缺失**:CLI enum `submission_schema` 是 surrogate,plan text 没声明 protocol-layer canonical field。

4. **F-SCRIPT-14(production gate 没 exercise final-script fixtures)** → 协议层 `production_ready = false` 在 gate dry-run path 是 false **by design**(per `d1bb6f7`),但 production gate assertion shape 不区分 fixture 与 production evidence(production-submisson-ready requires `coherent-offline-or-mock` in gate)。**协议层 invariant** "production gate tests final-script fixture" 在 smoke-only path 不可达(merged out)。

5. **D1BB6F7(court economics commitment-only)** → 协议层 **`testnet_beta_ready` 与 `production_ready` 是 operator-declared booleans**,不是 invariants。`economics_invariant_checked`(4115-4122)是 invariant check,但 `fee_policy` / staleness / production_ready 三者都是 self-disclosed。F-COURT-19 / F-COURT-21 / F-COURT-22 协议层结论:**`court_economics_evidence` 是 commitment 而非 enforcement**,这是 commit 名称披露的事实,**plan 与 README 没明确 narrative**。

6. **合并删除 fixture `8661e1b`**:F-COURT-18 — smoke + cellscript test 在 main branch broken-dead-on-arrival。协议层 "live devnet smoke proofs" 与 "cellscript test for typed-cell" 在 main branch 都是 broken。

7. **Stale-detection 缺 protocol layer time window**(commit `4612677`):F-COURT-17 — protocol layer 应加 `evidence_validity_window`。

## 7. 跨子系统影响

本 lane finding 对其他 lane(consensus / DA / settlement)的 protocol-layer 影响:

1. **Court → Consensus**:
- `finality_evidence`(consensus 给的 JSON text field)是 court subsystem's input,用作 `bundle.court_verifiable` 的 input。Consensus 提供的 finality evidence 在 court subsystem 应在 protocol layer 区分 **static-closed-committee** vs **Tendermint**,但 court subsystem 主要处理 "block_hash is consistent",**不**处理 consensus-specific protocol-layer invariants(round, precommit quorum, etc.)。
- Court to Consensus 的 contract:`finality_evidence.block_hash` 必须 computable from `MyelinBlock.consensus_kind + ordered_cell_tx_commitments + state_root_after`。这意味着 consensus lane 应确保 finality_evidence JSON 在 protocol layer 是 composable。

2. **Court → DA**:
- `court_economics_base_commitment` 输入 `da_availability_commitment`,意味着 court subsystem 信任 DA 的 availability commitment 在 protocol layer 是 **decidable**(no stale)。
- 但 court 不能 enforce freshness of `da_availability_commitment`:F-COURT-19 / F-COURT-22 暴露 cross-subsystem gap — DA subsystem 声明 `da_availability_production_ready`,但 court subsystem 的 commitment algorithm 不引用它。
- **Cross-subsystem interface contract proposal**:court subsystem 应在 `verify_session_settlement_intent` 时 require `da_availability_production_ready = true`,并在 protocol layer 加 `da_availability_freshness_proof` 字段。

3. **Court → Settlement**:
- `SettlementIntent.kind = "disputed-close"` 是 court-bundle-driven。`SettlementPackage` 包含 `l1_court_script_implemented: false` 与 `settlement_authority_authentication`。Court subsystem 的 protocol-layer responsibility 是 **不要**overlap with settlement_authority_authentication,但 court_economics_deployment_evidence 与 settlement_authority_authentication 都是 operator-declared evidence。
- 跨 subsystem 重复:`testnet_beta_ready = false` 与 `production_ready = false` 在 court_economics 是 hardcoded default,**也**在 settlement_authority_authentication 子系统可能 hardcoded default(per LANE_SETTLEMENT报告)。
- **Interface contract**:court subsystem 与 settlement subsystem 共享 **commitment-only evidence** 的 protocol-layer pattern;两者都需要 protocol-layer `stale-detection window` 字段。

4. **Court 内部**:
- `dispute_id` / `challenge_digest` / `challenger_pubkey_hash` 三个 protocol-layer field 缺失(F-COURT-01 / F-COURT-03 / F-COURT-07 / F-COURT-15 / F-COURT-20);这些是 **court 子系统独立性**的 invariants,跨子系统 contract 是它们的 hash domain namespace。

5. **cellscript `identity(field(...))`** in 协议层:per F-COURT-04,typed-cell identity 是 compiler-time metadata,**不是** on-chain protocol-level commitment。court-lane 应在 protocol-layer narrative 文档中明确 "identity 与 on-chain commitment 是 separate axis",与 cellscript lane / executor lane 的 contract 是 "类型-脚本 + cell_data + type_args 在协议层 enforce,typed-cell identity 在协议层不 enforce"。

## 附录 A: Findings 编号-严重度映射

| # | 严重度 | 维度 | 简述 |
|---|---|---|---|
| F-COURT-01 | CRITICAL | 严密度/安全 | chunk_index 是 sole identity,缺 challenger metadata |
| F-COURT-02 | CRITICAL | 严密度/合理 | challenge_window 无 protocol-layer min/max |
| F-COURT-15 | CRITICAL | 安全 | challenge window griefing 攻击者无 protocol-level escalation |
| F-COURT-16 | CRITICAL | 安全 | challenge window minimum-policy 缺位 |
| F-COURT-03 | HIGH | 严密度 | DisputedBundle verifier 不接受 signer identity |
| F-COURT-04 | HIGH | 严密度 | `identity(field(...))` 协议层不是承诺 |
| F-COURT-05 | HIGH | 严密度 | fee_policy declared but not committed/enforced |
| F-COURT-06 | HIGH | 严密度 | stale court_economics deployment 缺 window |
| F-COURT-09 | HIGH | 合理性 | plan 说 4 settlement kind,code 只有 1 |
| F-COURT-10 | HIGH | 合理性 | final-script 在 protocol layer required, CLI 不可达 |
| F-COURT-11 | MEDIUM | 合理性 | static committee + court 协议独立性 plan 没声明 |
| F-COURT-12 | MEDIUM | 合理性 | court bundle integrity invariant 名:`l1_court_implemented` vs `court_verifiable` |
| F-COURT-13 | MEDIUM | 合理性 | challenge_window_ms fixture value 不是 protocol constant |
| F-COURT-17 | HIGH | 安全 | stale-detection 无 protocol layer time window |
| F-COURT-18 | HIGH | 安全(广义) | fixture 在 main branch 不存在,smoke/scripts broken |
| F-COURT-19 | MEDIUM | 安全 | court economics commitment 包含 stale da_availability |
| F-COURT-20 | MEDIUM | 安全 | challenge_payload_hash domain 不含 challenger identity |
| F-COURT-07 | MEDIUM | 严密度 | chunk 拆分跨 dispute 不闭环 |
| F-COURT-08 | MEDIUM | 严密度 | `note` 字段 verbose-not-bound |
| F-COURT-14 | LOW | 合理性 | `d1bb6f7` self-disclosed degradation 是 good,narrative is missing |
| F-COURT-21 | LOW | 安全 | fee_policy 不能 enforce |
| F-COURT-22 | LOW | 安全 | `l1_court_implemented: false` 是 hardcoded,protocol 不能 detect forgery |

Count: 4 CRITICAL (F-01/02/15/16) + 7 HIGH (F-03/04/05/06/09/10/17/18) — wait, count recount:
- CRITICAL: F-COURT-01, F-COURT-02, F-COURT-15, F-COURT-16 = 4
- HIGH: F-COURT-03, F-COURT-04, F-COURT-05, F-COURT-06, F-COURT-09, F-COURT-10, F-COURT-17, F-COURT-18 = 8
- MEDIUM: F-COURT-07, F-COURT-08, F-COURT-11, F-COURT-12, F-COURT-13, F-COURT-19, F-COURT-20 = 7
- LOW: F-COURT-14, F-COURT-21, F-COURT-22 = 3

Total: 4+8+7+3 = **22 findings** (10 MUST CRITICAL/HIGH + 12 MAYBE MEDIUM/LOW)。

## 附录 B: 已知 VS 新增区分

| Finding | 来源 |
|---|---|
| F-COURT-01 (chunk_index sole identity) | 新增(本 lane) |
| F-COURT-02 (challenge_window no min) | 新增(本 lane) |
| F-COURT-03 (anon bundle verifier) | 新增(本 lane) |
| F-COURT-04 (identity(field(...)) not commitment) | 吸收 F-DOC-05,延伸至协议层 |
| F-COURT-05 (fee_policy not committed) | 吸收 F-CLI-28,延伸至协议层 |
| F-COURT-06 (stale court_economics no window) | 吸收 commit `4612677`,延伸至协议层 |
| F-COURT-07 (chunk 拆分跨 dispute) | 新增(本 lane) |
| F-COURT-08 (note 字段 verbose-not-bound) | 新增(本 lane) |
| F-COURT-09 (settlement kind drift) | 新增(本 lane) |
| F-COURT-10 (final-script unreachable) | 吸收 XD-01 / F-CLI-01 / F-DOC-01 / F-SCRIPT-14,协议层延伸 |
| F-COURT-11 (static committee + court narrative missing) | 新增(本 lane) |
| F-COURT-12 (invariant naming) | 新增(本 lane) |
| F-COURT-13 (challenge_window fixture value) | 吸收 F-DOC-07,延伸至协议层 |
| F-COURT-14 (d1bb6f7 self-disclosed) | 吸收 D1BB6F7 |
| F-COURT-15 (griefing attack) | 新增(本 lane) |
| F-COURT-16 (minimum policy missing) | 新增(本 lane) |
| F-COURT-17 (stale detection no window) | 吸收 F-CLI-28,延伸至协议层 |
| F-COURT-18 (main branch fixtures broken) | 吸收 XD-01,延伸至 main branch state |
| F-COURT-19 (court economics + stale DA) | 新增(本 lane) |
| F-COURT-20 (challenge_payload_hash domain) | 新增(本 lane) |
| F-COURT-21 (fee_policy cross mismatch) | 新增(本 lane) |
| F-COURT-22 (l1_court_implemented hardcoded) | 新增(本 lane) |
