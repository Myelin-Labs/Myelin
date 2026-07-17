# 跨子系统综合 — 协议层一致性与 claim ladder

> 输入:`audits/subsystem-models-2026-07-05/LANE_{CONSENSUS,DA,COURT,SETTLEMENT}.md`
> 外加 `MYELIN_SESSION_L2_PLAN.md`、`README.md`、`docs/security/claim-ladder.md`、
> `exec/src/celltx/types.rs`、`consensus/src/lib.rs`、`cli/src/main.rs`。
> 不重复 lane 报告里讲过的细节;本文档只做 cross-reference、对比、发现
> 单独 lane 看不到的协议层缺口。
>
> 时间:2026-07-05。
>
> 修订 v0.2 (2026-07-05):按 user review 收紧四处过强 / stale 表述 —
> (a) final-script CLI helper 实际已存在(`session_carrier_submission
> --verifier-role final-l1-script`,`cli/src/main.rs:8084-8096`),可 flip
> `l1_da_published` / `l1_court_submitted`(`cli/src/main.rs:8603-8604`),
> synthesis 不能再说"无 helper / 永远 false";(b) F-PRIM-01 的碰撞是
> *每个函数内部* `args` 没有长度前缀,不是"两个函数 hash 输入可相同";
> (c) F-CROSS-07 由 "v1/v2 混用是 bug" 重述为 "缺少统一 domain string
> registry / versioning spec";(d) domain string separator 不一致
> (celltx 用 hyphen,其他用 colon)在 §2.3 显式标注。**严重度结论与 finding
> 列表不变**;以下改动均为措辞精度,不影响修复优先级。

## TL;DR

四个子系统的协议层模型在 *内部一致性* 上都自洽:共识的字节级域分离、
DA 的四层 commitment 嵌套、Court 的 11 项 verifier check、Settlement 的
authority cell message_hash 各自闭环。但 *跨子系统* 一致性有 5 个独立
观察:

1. **三个 claim ladder 版本共存**:README 写 3 层
   (`README.md:97-101`)、`docs/security/claim-ladder.md` 写 4 层
   (插入了 `court bundle` 层,line 15)、`MYELIN_SESSION_L2_PLAN.md`
   的 acceptance criteria 把 Tier 2 等价于 *"a deterministic session fixture
   runs through open, commit, court-bundle, and verification"* 但又同时
   把 *disputed close + DA + settlement* 描述为 tier ≥ 2。这不是 doc 失修
   —— 是三个 doc 各自在协议层画了一条不同的 claim 边界。

2. **`production_ready` / `l1_court_implemented` / `l1_da_published` 三个
   readiness flag 在四个 lane 里各有不同语义**。DA 把 `production_ready`
   描述为 *AND-of-flags 的合取*(LANE_DA F-DA-04),Court 描述为 *commitment
   而非 enforcement*(`d1bb6f7` 自披露,F-COURT-14/19),Settlement 描述为
   *fixture 与 production 两条 readiness 路径不可叠加*(F-SETTLE-14)。同一
   boolean 在四个子系统里被赋予四种 protocol-layer 含义。

3. **静态 closed committee 与 "permissionless L2" 的边界**在 README
   (`README.md:91-104`) 显式承认 — *"closed-validator finality... is not a
   permissionless security claim"*。但四个子系统在 *协议层* 都偷偷越界:
   - 共识:`MyelinBlock.timestamp_ms` 来源未定(F-CONS-04)、quorum_weight
     无下限(F-CONS-03)、`Signature64` 不是密码学签名(F-CONS-01)、
     `finality gap` 未定义(F-CONS-08) — 共识 finality 的 *所有* 关键属性
     都不抗 permissionless 假设。
   - DA:`production_ready` 在 fixture 路径是 byte-deterministic
     (F-DA-04)、`finality gap` 内无 re-anchoring(F-DA-17)、retention
     过期无 fallback(F-DA-14)、provider 串谋检测路径未定义(F-DA-13)。
   - Court:`DisputedBundle` 是 anon 的(F-COURT-03)、`challenge_window`
     无 protocol 层最小值(F-COURT-02)、griefing 攻击无 escalation
     (F-COURT-15/16)、dispute path 是 *self-contained, 无外部 oracle*
     (F-COURT-11)。
   - Settlement:authority rotation 无 epoch(F-SETTLE-03)、`fee_policy`
     declared but not enforced(F-SETTLE-05/21)、custody / runbook 无 chain
     anchor(F-SETTLE-10/20)。

4. **claim ladder 三层在四个子系统的兑现度**:
   - *"no projection report"* → 设计意图:四个子系统都同意(承认是 CKB-style
     Cell-shaped);
   - *"successful projection"* → **真兑现的子集是 projection report 字节级
     存在**;DA manifest / settlement package / court bundle / settlement
     readiness 的 `l1_da_published` / `l1_court_implemented` /
     `l1_court_submitted` / `production_submission_ready` 在 manifest 与
     package 阶段保持 false 是事实(F-DA-04 / F-COURT-14 / F-SETTLE-14)。
     即 *projection* 在协议层存在,*projection+publish* 在协议层 *默认
     不发生*;
   - *"future exercised court"* → 协议层 *工程上不可达*,*不是*协议层
     invariant 上不存在。CLI 层 helper `session_carrier_submission
     --verifier-role final-l1-script` 已存在(`cli/src/main.rs:8084-8096`),
     schema 切到 `myelin-session-ckb-final-script-submission-v1`,可 flip
     `l1_da_published` / `l1_court_submitted`(`cli/src/main.rs:8603-8604`)。
     实际不可达*只*因为:(i) `cellscript/examples/myelin/{da-anchor-carrier,
     settlement-carrier, da-anchor-final, settlement-final}.cell` 在 merge
     `8661e1b` 已删除(见 XD-01);(ii) final-script kind 的 type-args
     canonicalization 仍不完整(F-SETTLE-18 + F-CROSS-04);(iii)
     `identity(field(...))` 在协议层是 typed-cell metadata 不是 on-chain
     commitment(F-COURT-04)。结论:CLI 路径存在,基础 artefact 默认
     false 是事实,但 final-script live submission 路径存在 + 当前工程源
     文件缺失 = "tier 3 在协议层工程上不可达",*不是* "协议层零兑现"。

5. **跨子系统的传染缺陷**(详细见 §4):F-PRIM-01 cellid collision
   (`exec/src/celltx/types.rs:299-307, 316-324`)在四个子系统里以 *不同表现*
   传染:共识的 committee member identity(共享 `CommitteeValidator` struct,
   F-CONS-16)、DA 的 `da_manifest_hash` 引用(DA 用 `molecule_transaction_hash`
   不直接用 type-cell identity,但 final-script 用,F-DA-07)、Court 的
   `chunk_index` sole identity(F-COURT-01)与 `identity(field(...))` 误读
   (F-COURT-04)、Settlement 的 authority cell 与 final settlement carrier
   type args(F-SETTLE-18/21)。

## 1. 四个子系统模型总评

### 1.1 共识(`consensus/src/lib.rs`)

严密度:Layer A(域分离 + 类型边界)真严密;Layer B(单块验证)在 `timestamp_ms`
来源(F-CONS-04)、`quorum_weight` 下限(F-CONS-03)、`consensus_kind` 稳定性
(F-CONS-17)上 *不严密*;Layer C(多块/多会话/跨子系统)*不闭环*。

合理性:模型诚实承认 *"verifier-only fast path, not a permissionless BFT
network"*(`consensus/src/lib.rs:6-11`),但公开叙事用 *Tendermint-style*、
*weighted precommit finality* 命名暗示完整 BFT 状态机(F-CONS-02/11)。

安全性:fixture 边界内 deterministic;L2 业务边界上 *协议层无 slashing
evidence log*(F-CONS-06/21)、*finality gap 未定义*(F-CONS-08)、*public_key
是 32 字节 hash-like bytes*(F-CONS-15)。即共识层安全声明是 *"我验证这
组签名是真的"*,不是 *"我产生过这些 finality"*。

给 claim ladder 的关键约束:**共识 finality 在协议层是 *"封闭委员会在那一刻
的签名"*,与 L1 CKB reorg 概率正交**。下游子系统若把 Myelin finality 当
*"强 finality 锚点"* 都是在协议层之外做承诺。

### 1.2 DA(`state/src/store/{segment,proof}.rs` + `cli/src/main.rs:3019-3549`)

严密度:四层 commitment 是 *嵌套 commitment*,不是 *signature chain*
(F-DA-01)。`segment_root → segment_proof → da_manifest → external_da_receipt`
的转换是 *commitment 折叠*,不是独立密码学绑定。L1 / L2 在协议层是本地
声明;L4(provider 签名)只覆盖 typed fields,不覆盖 raw bytes(F-DA-02)与
`availability_commitment`(F-DA-03)。

合理性:`production_ready` 在协议层是 *AND-of-flags* 的合取(F-DA-04),不是
*外部事实触发的不变量*。分层 DA(`local_only → testnet_beta_ready →
production_ready → l1_da_published`)是 UX 渐进披露,**不是密码学强度分级**
(F-DA-11)。

安全性:DA 失败的容错路径在协议层 *不存在*(F-DA-12/18),retention 过期
无 fallback(F-DA-14),finality gap 内无 re-anchoring(F-DA-17),provider
串谋检测未定义(F-DA-13)。audit_log_commitment 是 *字面承诺*(32 字节 hex
即通过,F-DA-05),无 audit log 引用。

给 claim ladder 的关键约束:**DA 子系统的 trust anchor 是 L1 CellTx
witness,不是 provider receipt**。任何把 *production_ready* 当作 *链上事实*
的子系统都是在协议层越界。

### 1.3 Court(`cli/src/main.rs:5778-7488`)

严密度:court bundle 的 11 项 verifier check 在 `cli/src/main.rs:5880-6097`
闭环,但 dispute path 在协议层有 4 个独立缺口:chunk_index sole identity
(F-COURT-01)、DisputedBundle anon(F-COURT-03)、`identity(field(...))` 不是
on-chain commitment(F-COURT-04)、court_economics 是 commitment-only
(F-COURT-05/14)。`l1_court_implemented: false` 是 hardcoded default,
`testnet_beta_ready` / `production_ready` 在 court_economics 是 hardcoded
false(`cli/src/main.rs:4153-4154`)。

合理性:`challenge_window_ms` 默认 60_000(`cli/src/main.rs:524`),但协议层
无最小值约束(F-COURT-02/13/16),CLI 接受 `--challenge-window-ms 1`。
`current_time_ms` 是 caller-provided(CLI arg,`cli/src/main.rs:1110`),
不是从 L1 block header 读取(F-SETTLE-06 协议层延伸)。`settlement_intent`
的 `kind = "disputed-close"` 是 hardcoded 唯一接受值(`cli/src/main.rs:6976-6978`),
`normal close / timeout exit / abort` 在协议层不存在(F-COURT-09)。

安全性:challenge window griefing 攻击在协议层 *无 escalation 路径*
(F-COURT-15/16),court_economics deployment evidence 的 stale detection
缺 protocol-layer time window(F-COURT-17),`da_availability_commitment`
进入 court_economics 但 *不绑定 production_ready*(F-COURT-19)。

给 claim ladder 的关键约束:**Court 子系统在 static-closed-committee 下
是 *self-contained* path — 没有外部 oracle、没有 escalation、没有 judge
角色**。这是 architectural 合理选择,但 plan / README 没在协议层 narrative
上声明这一点(F-COURT-11)。

### 1.4 Settlement(`cli/src/main.rs:3620-4530`)

严密度:authority cell 的 `myelin:session-settlement-authority-cell-auth:v1`
域覆盖 5 字段(`cli/src/main.rs:3665-3704`),192 字节 authority data layout
固定(`cli/src/main.rs:4389-4405`),`consumed_input_index = 1`(`cli/src/main.rs:3660`)
强制 authority cell 是 input[1]。但 message_hash 域无 activation epoch /
rotation nonce(F-SETTLE-03),attestation_hash 把 hex 字符串喂进 blake3
(F-SETTLE-02),`session_authority_commitment` 是 commitment 不是 signature
wrapper(F-SETTLE-04),`session_lineage_commitment` 在 SettlementIntent 验证
中只做 equality(F-SETTLE-05)。

合理性:`court_economics` 经过 `73ddc73 → d1bb6f7 → 4612677 → 5ae440d` 四个
commit 演化后形成 *policy commitment + (optional) deployment evidence +
stale rejection* 三层结构(§5.2 of LANE_SETTLEMENT),但 P1 缺口仍在:
`economics_commitment_algorithm` 在 with-deployment 路径下被覆盖
(F-SETTLE-07),`economics_commitment` verify 是 input self-consistent 不是
input ↔ oracle(F-SETTLE-08),`fee_policy` declared but not enforced
(F-SETTLE-05/21),`da_evidence_required = true` 是 boolean 断言而非
commitment verification(F-SETTLE-01)。

安全性:`current_time_ms` 是 caller-provided,协议层不校验(F-SETTLE-06)。
operator custody / runbook 在协议层 *无 chain anchor*,修改无 detection
(F-SETTLE-10/20)。`production_ready` 在 fixture / rehearsal 路径下 byte-
deterministic,在 production 路径下依赖外部密钥与 audit log 引用
(F-SETTLE-14/19)。

给 claim ladder 的关键约束:**Settlement 子系统的 "生产证据链"是 evidence
chain(commitment-only),不是 enforcement chain**。`l1_court_implemented =
false` 与 `l1_court_submitted = false` 在 fixture 路径下是 truth,production
candidate evidence 不被 production gate 覆盖(F-SCRIPT-14)。

## 2. 协议层 claim ladder 一致性

### 2.1 三层 claim 在四个子系统中的兑现

| Claim ladder 层 | 共识 | DA | Court | Settlement |
|---|---|---|---|---|
| **no projection report**<br/>"designed to stay close to CKB semantics" | ✓ (Cell-shaped `MyelinBlock` 字段 + 字节级域分离) | ✓ (Merkle segment + Molecule transaction bytes) | ✓ (Molecule bytes + wtxid + block-hash) | ✓ (192 字节 authority data + 6 字段 binding) |
| **successful projection**<br/>"projectable into a CKB-style transaction/context" | ✓ 在 protocol layer 是 byte-deterministic(`ordered_cell_tx_commitments → state_root_after` 链) | △ 投影存在,manifest 路径下 `l1_da_published = false`(默认);`session_carrier_submission --verifier-role final-l1-script --submit --rpc-url` 路径可 flip(`cli/src/main.rs:8603`),但依赖缺失 source | △ court bundle 16 项 verifier pass;`l1_court_implemented` 在 manifest 路径默认 false,final-l1-script 路径可 flip 但依赖缺失 source | △ `SettlementIntent` 字节级确定,manifest 路径下 `l1_court_implemented = false` / `l1_court_submitted = false` 默认;final-l1-script 路径可 flip 后者但依赖缺失 source(F-SETTLE-14) |
| **future exercised court**<br/>"disputed chunk adjudicable by the CKB-aligned path" | ✗ 协议层 *默认零兑现*(F-CONS-08 finality gap 未定义,无 L1 anchor);CLI 层无 finality-flip 路径 | ✗ final-script fixture 缺失(F-DA-07),manifest 路径下 `l1_da_published = false` 默认;CLI 层 final-l1-script helper 存在但 source 缺失 | ✗ final-script CLI helper 已存在(`cli/src/main.rs:8084-8096`),但依赖 missing `.cell` source;`identity(field(...))` 是 typed-cell metadata 不是 on-chain commitment(F-COURT-04/10/18) | ✗ manifest 路径下 `l1_court_submitted = false` 默认;CLI 层 final-l1-script 路径可 flip,但依赖 missing source + F-SETTLE-18 cellid collision 在 final-script kind 上 |

**关键观察**:

- *successful projection* 在四个子系统里都是 *off-chain verified +
  on-chain not-yet* 的二元结构:court bundle 的 `l1_court_implemented:
  false`、settlement package 的 `l1_court_submitted: false`、DA manifest
  的 `l1_da_published: false`、settlement readiness 的
  `production_submission_ready: false` —— 这四个 boolean 在 *manifest /
  package* 路径下默认 false,但 *final-l1-script live submission* 路径
  (`session_carrier_submission --verifier-role final-l1-script --submit
  --rpc-url`,`cli/src/main.rs:8084-8604`)可以 flip `l1_da_published` /
  `l1_court_submitted` 为 true;只是该路径依赖的 final-script `.cell`
  source 已缺失(merge `8661e1b`),且 final kind 的 type-args
  canonicalization 仍不完整(F-SETTLE-18 + F-CROSS-04)。**因此**:
  "boolean 永远是 false" 在 *当前工程 checkout* 上成立,但 *协议层*
  "flip 路径不存在" 不成立 —— CLI 路径存在;production-side 不可达是工程
  问题,不是协议层 invariant。
- *future exercised court* 在协议层 *工程上不可达*:CLI 路径存在但
  不可重现;没有 deployed verifier script、没有 on-chain verdict、没有
  slashed bond on chain。所有 lane 都在 *narrative* 上讨论 tier 3,在
  *protocol layer* 没有任何 invariant 支持 *当下* tier 3 的 claim,
  但协议层 *没有* 阻止未来 tier 3 落地的机制性障碍 —— 工程层 fixture
  缺失才是 tier 3 不可达的真正原因。

### 2.2 静态委员会 vs permissionless 边界

README 显式承认 `closed-validator finality ≠ permissionless L2`
(`README.md:91-104`)。但四个子系统在 *协议层* 都分别给出反例:

| 子系统 | "封闭性"在协议层的真实强度 | 与 permissionless 的张力 |
|---|---|---|
| 共识 | "closed" 是 TOML 配置级别的事实,不是 protocol invariant(F-CONS-05);"signature" 不是密码学签名(F-CONS-01/15);finality gap 未定义(F-CONS-08) | 模型在 *fixture 边界内*诚实;在 *L2 业务边界* silent misclaim(Tendermint-style 命名暗示 BFT safety) |
| DA | "production_ready" 是 AND-of-flags(F-DA-04),fixture 路径 byte-deterministic、production 路径依赖外部密钥 | "production" 在语义上是 *provider SLA*,不是 *密码学强度*(F-DA-11) |
| Court | dispute path 是 *self-contained*,无外部 oracle(F-COURT-11);`challenge_window_ms` 无 protocol 最小值(F-COURT-02);`DisputedBundle` 是 anon 的(F-COURT-03) | static-closed-committee + self-contained dispute 在 fixture 边界内自洽;在 permissionless 边界内 *unopposed* |
| Settlement | authority rotation 无 epoch(F-SETTLE-03);custody / runbook 无 chain anchor(F-SETTLE-10/20);`production_ready` fixture-vs-rehearsal 不可叠加(F-SETTLE-14) | "operator custody" 在协议层是 *声明* 而不是 *chain identity* |

**跨子系统结论**:四个子系统在 *fixture 边界内* 都是 *closed-committee
honest-but-curious* 模型;在 *L2 业务边界* 上,任一子系统单独看,都没有
permissionless 安全声明。但四个子系统 *组合起来* 在 README / claim-ladder
文档里给出的 narrative 是 *tier-by-tier proof-of-shape*,与 *proof-of-
validity-on-live-chain* 之间存在没有被任一 lane 单独识别的协议层缺口。

### 2.3 一致性 findings(F-CROSS-NN)

#### F-CROSS-01 [HIGH · claim ladder 文档不一致]

README 是 3 层(`README.md:97-101`)、`docs/security/claim-ladder.md` 是 4
层(在 *successful projection* 与 *future exercised court* 之间插入
*court bundle* tier,line 15)。`MYELIN_SESSION_L2_PLAN.md:540-553` 的
milestone exit criteria 把 *disputed close + DA + settlement* 等价于
*tier ≥ 2*,但 *tier ≥ 3* 没明确说。三个文档画了三个不同的协议层 claim 边界,
读哪一个取决于读者从哪个入口进入。**协议层影响**:外部审计 / 集成方拿到
三个不同版本的 claim ladder,无法机械分辨"我现在 climb 到哪一层"。建议方向
(协议层):把 claim ladder 写成一个 *canonical spec*,README / plan / claim-
ladder.md 三处 source-of-truth 收敛到一个文件。

#### F-CROSS-02 [CRITICAL · production_ready 在四个子系统里 4 种语义]

| 子系统 | production_ready 的语义 | file:line |
|---|---|---|
| 共识 | (无 production_ready 字段;`finality_evidence` 是 byte-deterministic cert) | `consensus/src/lib.rs:296-304` |
| DA | "AND-of-flags 的合取" | `cli/src/main.rs:3411-3418` |
| Court | "commitment, 不是 enforcement"(self-disclosed `d1bb6f7`) | `cli/src/main.rs:4153-4154` |
| Settlement | "fixture 路径 byte-deterministic;production 路径依赖外部密钥与 audit log 引用" | `cli/src/main.rs:10365-10463` + `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:13` |

同一个 boolean 字段(`production_ready: true/false`)在四个子系统里意味着
四件事。在协议层 *没有声明* 这四个 boolean 在跨子系统场景下必须一致。**协议
层影响**:DA 的 `da_availability_production_ready = true` 与 Court 的
`court_economics.production_ready = true` 同时出现时,外部审计无法分辨
"这两个 true 是同一个 production level 还是两个独立 production claim"。

#### F-CROSS-03 [HIGH · finality gap 在四个子系统都不定义]

- 共识:F-CONS-08 — `FinalisedBlock` 无 L1 anchor,commit 后到 L1 投影前可
  被覆盖;
- DA:F-DA-17 — finality gap 内无 DA re-anchoring;
- Court:F-COURT-15 — challenge window griefing,无 protocol escalation;
- Settlement:F-SETTLE-06 — `current_time_ms` 是 caller-provided,协议层
  不校验"challenge window elapsed"真实性。

四个子系统 *各自* 识别了 finality gap 的一个角度,但 *任一* 子系统都没有
引入 *cross-system finality checkpoint*。即:`Myelin finality + DA receipt
+ Court bundle + Settlement intent` 在时间维度上 *互相独立*,没有共同的
checkpoint 协议层绑定。**协议层影响**:L2 业务方按 README 的 *"successful
projection → future exercised court"* 来假设 finality durability,在
finality gap 内做的不可逆动作(签了 DA anchor、settled intent)有可能被覆盖
后作废,而 *任一* 子系统的协议层都不 catch 这个 gap。

#### F-CROSS-04 [HIGH · final-script fixture 缺失是 3 个子系统共因]

`cellscript/examples/myelin/{da-anchor-carrier,settlement-carrier,da-anchor-
final,settlement-final}.cell` 在 merge `8661e1b` 删除(LANE_COURT §0 前提)。
后果在三个子系统各自表现:

| 子系统 | final-script 缺失的协议层后果 | lane finding |
|---|---|---|
| DA | `final_l1_script_submission_ready` flag 在 protocol 层没有真实 deploy backing,F-DA-07 | F-DA-07 |
| Court | `identity(field(...))` 协议层 typed-cell metadata 没有 on-chain enforcement;final-script CLI helper 已存在(`session_carrier_submission --verifier-role final-l1-script`,`cli/src/main.rs:8084-8096`),但依赖 missing `.cell` source | F-COURT-04/10/18 |
| Settlement | final-script kind 走 `carrier_payload_type_args_hex` fall-through 到 truncated 32-byte data_hash,F-PRIM-01 在 truncated path 下提供 attacker collision 路径,F-SETTLE-18 | F-SETTLE-18/21 |

**协议层影响**:final-script 是 *protocol-layer required closure*,*工程
层 production-side unreachable*。三个子系统 *各自* 报告这个缺陷,但 *单独*
都把它当作"本子系统的 fixture 缺漏";**跨子系统** 视角下,它是 *claim ladder
tier 3 的 foundation 缺失* — CLI 路径在协议层存在(可 flip `l1_da_published`
/ `l1_court_submitted`,见 `cli/src/main.rs:8603-8604`),*协议层*
tier 3 没有可达 invariant;production-side 因 `.cell` source 缺失 +
type-args canonicalization 缺口无法重现为 tier 3 foundation。

#### F-CROSS-05 [HIGH · 静态委员会 vs permissionless 边界 4 子系统同时偷越]

README `README.md:91-104` 显式承认 *"Static committee finality alone must
not be marketed as permissionless L2 security"*。但四个子系统在协议层都
分别给出 silent misclaim 路径:共识(F-CONS-02/08/15)、DA(F-DA-04/11)、
Court(F-COURT-02/15/16/11)、Settlement(F-SETTLE-03/10/14)。

**协议层影响**:**任何一个** 子系统的 "permissionless safety claim" 都
不能从其他三个子系统的协议层得到支持(DA "production-ready" 不抗
permissionless provider,Court "challenge window" 不抗 unopposed griefing,
Settlement "custody" 不抗 operator rotation 滥用)。**这是 README 边界
声明在四个子系统的协议层都没有被 internalize 的结果**。

#### F-CROSS-06 [MEDIUM · challenge_window 在三个子系统的语义分裂]

| 子系统 | challenge_window 协议层角色 | file:line |
|---|---|---|
| Court | 单窗口检查,无 in-progress state(F-COURT-15) | `cli/src/main.rs:7000-7004` |
| Settlement | 仅 `> 0` hard constraint;`current_time_ms` caller-provided(F-SETTLE-06) | `cli/src/main.rs:6979-6980, 1110` |
| DA | 不绑定 challenge_window,court_economics 接受任意值 | `cli/src/main.rs:4170-4178` |

**协议层影响**:challenge_window 是 *跨越 DA / Court / Settlement* 的
协议层信号,但只有 Court 定义 *protocol-layer semantics*;Settlement
复述但不 enforce,DA 消费但不 enforce。**协议层字段被跨子系统共享但
没有一致 semantics**。

#### F-CROSS-07 [MEDIUM · 缺少统一 domain string registry / versioning spec]

12 个 domain 字符串(实际 codebase 中至少还有 `myelin:static-committee-
signature-hash:v1` / `myelin:tendermint-precommit-signature-hash:v1` /
`myelin:external-da-receipt-document:v2` / `myelin:session-da-proof-
molecule:v1` 等十几个,这里只列 lane 报告 cross-reference 用到的 12 个)
分散在 celltx 与 consensus / DA / Court / Settlement 之间。两点观察:

1. **separator 不一致**:celltx 用 hyphen 与 slash(`myelin-typed-cell/
   conflict-hash/v1`、`myelin-typed-cell/typed-data-hash/v1`,
   `exec/src/celltx/types.rs:301, 318`);consensus / DA / Court /
   Settlement 用 colon(`myelin:block:v1` 等,见下表)。separator 风格
   不一致*本身*不是协议 bug —— 字符串本身就是 domain 的一部分,separator
   改了 hash 也跟着变。
2. **v1 / v2 版本号共存**:12 个里有 1 个 v2(`myelin:external-da-receipt-
   provider-signature:v2`),其余 v1。**v1 / v2 各自有版本号也不是 bug**;
   bug 是协议层 *没有* 一个 canonical registry 声明下面这些问题的答案:

- 哪些 v1 / v2 是 *authoritative*(active 协议层输入)?
- 哪些是 *legacy / retired*(保留兼容但不再生产新数据)?
- v1 → v2 升级时的 compat policy(同时验证两版本?何时退役旧版)?
- 谁有权注册新 domain(命名空间 authority)?
- cross-domain collision 如何 audit(同一字节输入跨多个 domain 是否
  能产生相同 hash,即使 separator 不同)?

Settlement lane 已识别 `economics_commitment_algorithm` 在 with-deployment
下被覆盖的协议层问题(F-SETTLE-07),但这是 *算法字符串* 漂移;**域字符串
命名空间管理**的协议层缺口(active / retired / compat / authority /
cross-domain audit)未在任一 lane 单独识别。

| Domain string | 子系统 | 来源 | version |
|---|---|---|---|
| `myelin:block:v1` | 共识 | `consensus/src/lib.rs:21` | v1 |
| `myelin:static-committee-signature:v1` | 共识 | `consensus/src/lib.rs:22` | v1 |
| `myelin:tendermint-precommit:v1` | 共识 | `consensus/src/lib.rs:23` | v1 |
| `myelin-typed-cell/conflict-hash/v1` | celltx | `exec/src/celltx/types.rs:301` | v1 |
| `myelin-typed-cell/typed-data-hash/v1` | celltx | `exec/src/celltx/types.rs:318` | v1 |
| `myelin:external-da-receipt-provider-signature:v2` | DA | `cli/src/main.rs:3019-3037` | v2 |
| `myelin:session-da-availability-commitment:v1` | DA | `cli/src/main.rs:3442-3519` | v1 |
| `myelin:session-court-economics:v1` | Court / Settlement | `cli/src/main.rs:4147-4149` | v1 |
| `myelin:session-court-economics-with-deployment:v1` | Court / Settlement | `cli/src/main.rs:4163-4165` | v1 |
| `myelin:session-settlement-authority-cell-auth:v1` | Settlement | `cli/src/main.rs:3672` | v1 |
| `myelin:session-settlement-authority-cell-signature-attestation:v1` | Settlement | `cli/src/main.rs:3737-3772` | v1 |
| `myelin:operator-custody-policy-document:v1` | Settlement | `cli/src/main.rs:10003` | v1 |

## 3. 跨子系统接口一致性

### 3.1 Consensus → DA

**接口定义**:`FinalisedBlock { block, block_hash, certificate }`
(`consensus/src/lib.rs:296-304`)是 DA 锚定输入。`da_manifest.proof_valid`
(`cli/src/main.rs:6410-6440`)重验 `SegmentProof`,但 *不* 重验
`MyelinBlock.consensus_kind` 与 committee cert 的关系。

| 共识侧 | DA 侧 |
|---|---|
| `Signature64` 是 commitment 不是密码学签名(F-CONS-01) | DA 把 `finality_evidence` 当 byte-deterministic evidence 消费 |
| `timestamp_ms` 来源未定义(F-CONS-04) | timestamp 进入 block hash,同一 chunk 不同 timestamp → 不同 DA manifest |
| `finality gap` 未定义(F-CONS-08) | finality gap 内无 re-anchoring(F-DA-17) |

**影响**:**DA 不能假设 consensus finality 是 final**。接口契约应是
*"consensus finality 是 provisional,直至 L1 投影完成"*,DA 协议层没声明。

### 3.2 Consensus → Court

**接口定义**:Court bundle 的 11 项 verifier(`cli/src/main.rs:5880-6097`)
验证 `finality_evidence.block_hash == block_hash`
(`cli/src/main.rs:6075-6080`),*不*验证 `consensus_kind`、`round`、
`quorum_weight`、`signed_weight` 与 committee config 的关系。

| 共识侧 | Court 侧 |
|---|---|
| 两 engine 各有独立 cert schema(F-CONS-02/11) | `finality_evidence` 是 JSON 文本字段,只携带 *字符串*,不携带 engine-specific invariant |
| `round: u32` 协议层无 sanity check(F-CONS-12) | Court 不验证 `round` 合理性(`u32::MAX` 也合法) |
| 同一 `(height, round)` validator 可签两个 block_hash(F-CONS-06) | Court 不检测 finality equivocation;conflicting `FinalisedTendermintBlock` 都 valid |

**影响**:Court 的 chain-of-custody 是 *byte-precise* 但 *consensus-agnostic*。
`finality_evidence.block_hash == block_hash` *不足以* 推出 *"这条 chunk 真
被 ≥ quorum_power 个 validator 在合理 round 上签了"*。

### 3.3 DA → Court

**接口定义**:`court_economics_base_commitment` 输入 `da_availability_commitment`
(`cli/src/main.rs:4170-4178`)。`verify_session_settlement_intent`
(`cli/src/main.rs:7058-7340`)验证 `da_manifest.session_id / chunk_index /
segment_root / challenge_payload_hash / molecule_transaction_hash / proof_valid`。

| DA 侧 | Court 侧 |
|---|---|
| `da_availability_production_ready` fixture=false, production 依赖外部密钥(F-DA-04) | commitment 算法 *不区分* 这两种 commitment 字节(F-COURT-19) |
| `da_manifest_hash` 在 finality gap 内可能被覆盖(F-DA-17) | Court 协议层没有 *DA-recompute-on-equivocation* 路径 |
| provider receipt 签名不覆盖 `receipt_hash` / `receipt_commitment`(F-DA-02/03) | Court 把 receipt 当 byte-deterministic evidence 消费,不重验 provider signature |

**影响**:Court 对 DA 的承诺是 *byte-identical availability commitment*,
但 DA 侧 *不保证* 这条 commitment 在 cross-session / cross-evidence-mode
下稳定。DA 失败时 Court 协议层 *没有 degradation path*(F-DA-18 + F-COURT-19)。

### 3.4 Court → Settlement

**接口定义**:`verify_session_settlement_intent` 验证 court bundle 6 项
绑定 + `court_economics` schema/fields recompute 一致
(`cli/src/main.rs:7258-7283`)+ `challenge_deadline_ms > current_time_ms`+
*保留* `l1_da_published = false` + `l1_court_implemented = false`
(`cli/src/main.rs:7305-7317`)。

| Court 侧 | Settlement 侧 |
|---|---|
| `challenge_window_ms` 默认 60_000,无 protocol 最小值(F-COURT-02) | Settlement 接受 `= 1`,`economics_invariant_checked` 只要求 `> 0`(F-SETTLE-06) |
| `DisputedBundle` 是 anon 的(F-COURT-03) | Settlement 协议层不验证 challenger identity |
| `da_availability_commitment` 不绑定 production_ready(F-COURT-19) | `da_evidence_required = true` 是 boolean 断言非 commitment verification(F-SETTLE-01) |
| `l1_court_implemented: false` hardcoded default | Settlement 协议层 *强制保留* `false`(`cli/src/main.rs:7305-7317`) |

**影响**:`l1_court_implemented = false` 在 Court 与 Settlement 是 *双边
强制保留* — 两边都拒绝 *让它变 true*。这是 *self-aware* 边界,但 README
/ plan 都把它描述为 *future work*,**没有任何*协议层*机制让它从 false 变
true**。即 *future exercised court* 是 narrative-only,不是 protocol-reachable。

### 3.5 Settlement → Consensus

**接口定义**:`SettlementIntent` 绑定 `participant_set_hash` 与 `session_id`
(`cli/src/main.rs:4110, 4360+, 3620-3662`),对应 `MyelinBlock.consensus_kind`
与 session 身份(`MYELIN_SESSION_L2_PLAN.md:93-99`)。

| 共识侧 | Settlement 侧 |
|---|---|
| `consensus_kind` 字符串在 block hash 内,稳定性协议层无冻结(F-CONS-17) | `court_economics` commitment 算法对 consensus_kind 不敏感(F-SETTLE-07) |
| finality 两 engine byte-different,state transition byte-identical | Settlement 协议层要求 state transition byte-identical,finality evidence 可不同(*good* invariant) |

**影响**:**Settlement → Consensus 接口在协议层是 *byte-stable on state
root, byte-flexible on finality evidence***。兑现度较高(LANE_SETTLEMENT
§7.1)。但 `consensus_kind` 字符串稳定性是缺口(F-CONS-17):协议升级改了
`ConsensusKind::as_str()`,SettlementIntent 协议层 *无法 detect* finality
evidence 变了 *因为* 协议升级。

### 3.6 Settlement → DA

**接口定义**:`SettlementIntent` 绑定 `da_manifest_hash`
(`cli/src/main.rs:4371, 4112, 7263`)。3fda2ab recompute 路径
(`cli/src/main.rs:10408-10463`)在 final-L1 settlement 下重调
`da_availability_evidence`(`cli/src/main.rs:10446-10456`)。

| DA 侧 | Settlement 侧 |
|---|---|
| `da_availability_production_ready` 接受 fixture/production 两种 key(F-DA-04) | recompute 与原始 byte-identical,但 *不* 重验 provider signature 域(F-SETTLE-19) |
| external_receipt 的 `receipt_id` / `availability_window` 不在签名域(F-DA-02) | recompute 把 external_receipt 当 black box;替换这两个字段仍通过 |
| `availability_commitment` 折叠 receipt_hash + receipt_commitment,但 provider 签名不覆盖(F-DA-02/03) | recompute 不检测 receipt_hash 与 typed fields 绑定 |

**影响**:**Settlement recompute 路径在 final-L1 下 *继承* DA 的 provider
signature 缺口** — DA 的 F-DA-02/03 不是 *DA-only* 问题,在 recompute 路径
下被放大(LANE_SETTLEMENT F-SETTLE-19)。

## 4. F-PRIM-01 跨子系统传染分析

F-PRIM-01(`exec/src/celltx/types.rs:299-307, 316-324`):
两个 hash 函数 *各自独立*,domain 字符串与最后一个域都不同
(`compute_conflict_hash` 用 `myelin-typed-cell/conflict-hash/v1` 喂
`args || conflict_key_value`;`compute_typed_data_hash` 用 `myelin-typed-cell/
typed-data-hash/v1` 喂 `args || data`,见 §2.3 F-CROSS-07 表),*不*是
"跨两个函数 hash 输入可构造相同字节"。**真正问题在*每个函数内部* `args`
字段没有长度前缀**:

- `compute_conflict_hash` = `blake3(domain || code_hash || hash_type ||
  args || conflict_key_value)`:`args` 是 `Vec<u8>` 变长字段,与
  `conflict_key_value` 之间没有长度前缀;`(args="X", conflict_key_value="")`
  与 `(args="", conflict_key_value="X")` 在该函数内产生相同 hash。
- `compute_typed_data_hash` = `blake3(domain || code_hash || hash_type ||
  args || data)`:同样问题,`(args="X", data="")` 与 `(args="", data="X")`
  在该函数内产生相同 hash。

attacker 利用这条路径,可以在 typed-cell 之间构造 type-cell identity
碰撞;collision 完全在 celltx 协议层成立,*不*需要跨函数存在 —— 每个
函数*单独*就足以让 type-cell identity 失效。在四个子系统有四种不同传染
路径。

### 4.1 在共识子系统

`CommitteeValidator { id: String, public_key: Hash32, weight: u64 }`
(`consensus/src/lib.rs:236-244`)是 plain struct,不携带 type-cell identity。
F-PRIM-01 *不直接影响* 共识子系统(共识不使用 type-cell identity)。但
F-CONS-15 识别:`public_key: Hash32` 是 *32 字节 hash-like bytes*,协议层
不强制它是某个密码学公钥。validator 身份在协议层是 `(id, public_key)` 字
符串对,与 type-cell identity *正交*。**传染路径**:validator identity 与
*L1 type-cell identity* 在协议层互不约束;未来要锚定 validator identity
到 L1(如 validator registry cell),F-PRIM-01 会同时出现在 committee
member identity 与 consensus quorum attestation 上,届时需要协议层显式绑
定(如 `committee_id = typed_data_hash(validator_registry_cell)`)。

### 4.2 在 DA 子系统

DA manifest 的 `da_manifest_hash` 是 *Molecule transaction hash + segment_root*
的 commitment(`cli/src/main.rs:3442-3519`),不直接使用 type-cell identity。
但 final-script(`da-anchor-final.cell` / `settlement-final.cell`,已删除,
merge `8661e1b`)声明 `identity(field(...))` *声明* type-cell identity
(LANE_COURT F-COURT-04)。final-script fixture 缺失时,`identity(field(...))`
在协议层是 *typed-cell metadata* 而不是 on-chain commitment。**传染路径**:
DA production-ready 路径在 final-script kind 下需要 type-cell identity 在
L1 enforce;F-PRIM-01 让这条路径 *unreachable*(F-DA-07)。

### 4.3 在 Court 子系统

Court bundle 的 `chunk_index` 是 sole identity(F-COURT-01),但 Court bundle
引用 `molecule_transaction_hash` 与 `MyelinBlock`,不直接使用 type-cell
identity。`verify_session_court_bundle` 11 项 verifier 都是 byte-precise
recompute,不依赖 type-cell identity。但 final-script 的 `identity(field(...))`
在协议层是 typed-cell metadata,而 `exec/src/celltx/types.rs` 没有对应
`TypedCellDecl`(F-COURT-04)。**传染路径**:Court 的 final-script 路径
(tier 3 required closure)在协议层 *绑死* 到 type-cell identity;F-PRIM-01
让 final-script readiness 在 L1 不可验证 — 即 *future exercised court*
*tier 3 永久 unreachable*。

### 4.4 在 Settlement 子系统

**Settlement 子系统是 F-PRIM-01 的最严重落脚点**
(LANE_SETTLEMENT F-SETTLE-18/21)。Settlement final carrier 在 L1 上是
type-cell identity(`cli/src/main.rs:4407-4412`:type args = `session_id_hash
|| settlement_identity_hash`)。`carrier_payload_type_args_hex`
(`cli/src/main.rs:4584-4592`)只覆盖两种 carrier kind;其他 kind fall
through 到 `0x{data_hash_hex}` 32 字节(`cli/src/main.rs:4590`)。
- `(code_hash, hash_type, args="X", data="")` 与
  `(code_hash, hash_type, args="", data="X")` 产生相同 `typed_data_hash`;
- final-script kind type args 是 truncated 32-byte data_hash,attacker 可
  构造碰撞 type args *替换* authority cell / settlement cell sibling output;
- 跨 transaction uniqueness 依赖 authority cell 被 consumed;F-PRIM-01
  路径下 attacker 可制造 "non-consumed" 副本绕过 replay protection。

### 4.5 传染总结

强度排序:**Settlement(最严重,直接 chain identity) > Court(final-script
readiness unreachable) > DA(final-script DA path unreachable) > 共识(暂无
直接传染,未来 validator identity 锚定会被传染)**。**F-PRIM-01 不是 celltx-
only 问题**,是 *claim ladder tier 3 的 foundation 问题*。修复必须跨四个
子系统协调:celltx 修复(typed-cell identity 在 L1 enforce)+ DA final-script
recommit + Court final-script helper 补全 + Settlement final-script type-
args canonicalization。

## 5. 协议层修复优先级(不写具体 patch,只写方向)

按 *claim ladder tier 3 可达性* 与 *permissionless 边界声明一致性* 排序:

| 优先级 | 修复方向 | 涉及的 lane findings |
|---|---|---|
| **P0 · CRITICAL** | 跨子系统统一 `production_ready` 的语义:F-DA-04 / F-COURT-14 / F-COURT-19 / F-SETTLE-14 / F-SETTLE-19。建议方向:引入 3 个独立 boolean(`fixture_production_ready`、`rehearsal_production_ready`、`live_production_ready`),*禁止* 在跨子系统场景下用同一个 boolean | F-DA-04, F-COURT-14/19, F-SETTLE-14/19 |
| **P0 · CRITICAL** | final-script fixture 缺失是 tier 3 foundation 问题:F-DA-07 / F-COURT-10/18 / F-SETTLE-18/21。建议方向:在协议层明确 *final-script* 是 protocol-layer required closure,*committed* fixture 是 tier 3 的 *唯一* foundation;未 commit 之前,tier 3 在协议层 *不可达* | F-DA-07, F-COURT-04/10/18, F-SETTLE-18/21 |
| **P0 · CRITICAL** | finality gap 协议层缺口:F-CONS-08 / F-DA-17 / F-COURT-15/16 / F-SETTLE-06。建议方向:引入 *checkpoint_id* 概念,FinalisedBlock / DA manifest / Court bundle / Settlement intent 都引用同一 checkpoint 序号,checkpoint 由 L1 anchor 决定 | F-CONS-08, F-DA-17, F-COURT-02/15/16, F-SETTLE-06 |
| **P1 · HIGH** | 共识 finality 的 silent misclaim:F-CONS-01/02/05/11/15/17。建议方向:把 *Tendermint* 引擎名改为 *WeightedPrecommitVerifier*,`Signature64` 改名为 `PrecommitBinding64`,在 doc 中显式声明 *本协议层不实施 Byzantine 容错* | F-CONS-01/02/05/11/15/17 |
| **P1 · HIGH** | signature domain 与 challenge_window 跨子系统 protocol-layer semantics:F-CROSS-06/07。建议方向:把 *challenge_window_ms* 的 protocol-layer minimum / maximum 写进 spec;为 12 个(实际更多)domain string 引入 *canonical registry*,声明哪些 v1/v2 是 active / retired / compat 路径,并指明命名空间 authority | F-CROSS-06/07 |
| **P1 · HIGH** | authority cell authentication 缺 rotation nonce:F-SETTLE-03/16。建议方向:`myelin:session-settlement-authority-cell-auth:v1` 域加 `activation_epoch_ms`,attestation_hash 加时序区分 | F-SETTLE-02/03/16 |
| **P2 · MEDIUM** | cellid collision 跨子系统传染:见 §4。建议方向:celltx 修复 + 跨四个子系统协调 | F-PRIM-01, F-COURT-04, F-SETTLE-18/21 |
| **P2 · MEDIUM** | operator custody / runbook 无 chain anchor:F-SETTLE-10/20。建议方向:readiness 报告 *必须* 包含 chain anchor,operator custody 文档修改需新 readiness 报告 | F-SETTLE-10/20 |
| **P2 · MEDIUM** | Molecule-shaped vs Molecule-inspired 命名错位:F-CONS-10。建议方向:`to_molecule_bytes` 改名为 `to_table_bytes`,doc string 明确 *"Molecule-inspired hand-rolled encoding"* | F-CONS-10 |
| **P3 · LOW** | narrative drift:README / plan / claim-ladder.md 三处的 claim ladder 不一致;challenge_window fixture value 不是 protocol constant。建议方向:统一一个 *canonical claim ladder spec* 文档 | F-CROSS-01, F-COURT-13 |

## 6. 风险与不变项

**跨子系统必须保留的不变量**(不要在修复里碰它们):

1. **Cell-shaped MyelinBlock 字段**:`consensus/src/lib.rs:158-179` 的
   `MyelinBlock` 字段 + 三个 hash 域(`consensus/src/lib.rs:21-23`)是
   四个子系统都依赖的 *byte-stable commitment input*。任何修复 *不能*
   改 `MyelinBlock` 的字段顺序、字段类型或域字符串。

2. **`molecule_transaction_hash` 的 byte-determinism**:`cli/src/main.rs`
   的 Molecule 序列化路径在 court bundle verifier / DA manifest verifier
   / SettlementIntent verifier 三处都用 *byte-precise recompute + equality*
   验证。任何修复 *不能* 引入 *byte-fuzzy* 序列化(例如 JSON canonicalization、
   padding 差异)。

3. **CellDAG conflict_hash + typed_data_hash 命名分裂**:`exec/src/celltx/types.rs:299-307`
   与 `:316-324` *故意* 把 *stable-for-scheduling* (conflict_hash)与
   *changes-with-data* (typed_data_hash) 分开。这是 *intentional design*,
   修复 F-PRIM-01 *不能* 把两者合并,只能加 *L1 enforcement layer*。

4. **Domain string v1 / v2 命名**:任何 protocol-layer 升级 *必须* 走
   `vN+1` 命名 + 兼容旧 `vN` 验证路径。已存在的 12 个 domain string
   (见 F-CROSS-07)各自 v1 / v2,不能 *retire* 老 v1 字符串。

5. **committeed open = false**:`l1_court_implemented: false`、
   `l1_da_published: false`、`l1_court_submitted: false` 在四个子系统里
   都是 *hardcoded default*。claim ladder tier 3 的可达性 *完全* 依赖
   这三个 boolean *能在 protocol layer 被合法 flip*。**修复时这三个
   boolean 的"如何 flip"的 protocol-layer semantics 是不可省略的
   边界声明**。

6. **Static committee honest-but-curious**:`README.md:91-104` 显式承认。
   *任何* 修复不能让 static committee 越界为 permissionless BFT 网络,
   除非引入 BFT state machine(proposer 选举 / round state / locked value
   / fork choice)。

---

## 附录 A: 四份 lane 报告引用映射

| Lane 报告 | finding 计数 | 关键 file:line 锚点 |
|---|---|---|
| `LANE_CONSENSUS.md` | 22 (CRIT x4 / HIGH x5 / MED x11 / LOW x2) | `consensus/src/lib.rs:21-23` (三个 hash 域)、`:158-179` (MyelinBlock)、`:296-304` (FinalisedBlock)、`:556-596` (verify_precommit_certificate) |
| `LANE_DA.md` | 18 (MUST x13 / MAYBE x5) | `cli/src/main.rs:3019-3037` (签名域)、`:3442-3519` (availability_evidence)、`:10408-10463` (recompute) |
| `LANE_COURT.md` | 22 (CRIT x4 / HIGH x8 / MED x7 / LOW x3) | `cli/src/main.rs:5880-6097` (11 项 verifier)、`:6976-7050` (settlement_intent)、`:7058-7340` (verify_settlement_intent) |
| `LANE_SETTLEMENT.md` | 21 (P1 x6 / MED x14 / LOW x1) | `cli/src/main.rs:3620-3662, 3665-3704, 4389-4405` (authority cell)、`:4059-4336` (court_economics)、`:10003, 10079` (custody/runbook) |

## 附录 B: 已存在 audit 引用映射

| 已存在 audit | 与本 synthesis 的关系 |
|---|---|
| `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` (F-01..F-12) | 共识 lane F-CONS-06/14/16/18/21 协议层延伸 |
| `MYELIN_SWARM_AUDIT_STATE_DA.md` (25 findings) | DA lane F-DA-04/05/06/14/15 协议层延伸;§5.1 详细映射 |
| `MYELIN_SWARM_AUDIT_WHOLEREPO.md` (132 + 3 cross-lane) | F-CLI-01..35、F-SCRIPT-14、F-DOC-01/05/07、F-PRIM-01、XD-01、D1BB6F7 在 4 个 lane 的协议层延伸(全文交叉引用) |
| `MYELIN_PRODUCTION_GATE.md` | DA F-DA-07、Settlement F-SETTLE-14/19、Court F-COURT-09/18 引用 |
| `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` | Settlement §6.4 引用 production-evidence-complete 标签 |
| `MYELIN_CONSENSUS_COMPLETENESS.md` | 共识 lane F-CONS-02/11/20 引用 line 88 / 174-176 |
| `MYELIN_CKB_PROJECTION_AUDIT.md` / `MYELIN_CKB_SEMANTIC_DEVIATIONS.md` | projection report 语义 + F-CONS-10 Molecule-inspired 命名错位 |

## 附录 C: F-CROSS findings 一览

| ID | 严重度 | 简述 | 主要 file:line |
|---|---|---|---|
| **F-CROSS-01** | HIGH | claim ladder 文档 3 个版本共存 | `README.md:97-101` / `docs/security/claim-ladder.md:13-17` / `MYELIN_SESSION_L2_PLAN.md:540-553` |
| **F-CROSS-02** | CRITICAL | production_ready 4 子系统 4 种语义 | `cli/src/main.rs:3411-3418` (DA) / `:4153-4154` (Court) / `:10365-10463` (Settlement) / `consensus/src/lib.rs:296-304` (共识无字段) |
| **F-CROSS-03** | HIGH | finality gap 4 子系统都不定义 | `consensus/src/lib.rs:296-304` / `cli/src/main.rs` DA recompute / `:7000-7004` (Court) / `:1110` (Settlement) |
| **F-CROSS-04** | HIGH | final-script fixture 缺失是 3 子系统共因 | merge `8661e1b`;`cli/src/main.rs:15496+` tests 引用 missing fixture |
| **F-CROSS-05** | HIGH | 静态委员会 vs permissionless 边界 4 子系统同时偷越 | `README.md:91-104` + 四个 lane finding(见 §2.3) |
| **F-CROSS-06** | MEDIUM | challenge_window 3 子系统语义分裂 | `cli/src/main.rs:524` / `:6979-6980` / `:4170-4178` |
| **F-CROSS-07** | MEDIUM | 缺少统一 domain string registry / versioning spec | 12 个 domain strings(实际 ≥24,见 §2.3 表) |

---

报告完成。本 synthesis 在协议层识别 7 个 cross-cutting 缺口(F-CROSS-01
至 F-CROSS-07),其中 F-CROSS-02 / F-CROSS-04 与 §4 F-PRIM-01 传染分析
构成 claim ladder tier 3 的三个 foundation 缺口。不重复 4 个 lane 报告
里的 83 条 finding,而是把它们 *cross-reference* 到 *协议层不变量的兑现度*。
最高优先级是 F-CROSS-02(production_ready 语义分裂)与 F-CROSS-04(final-
script foundation 缺失)— 两者共同决定 *tier 3 在协议层是否可达*。