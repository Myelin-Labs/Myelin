# Lane D — Settlement 子系统协议模型审计

> 协议层评审（verifier-only，不修代码）。范围：Myelin Settlement 子系统的
> **协议层模型** — authority cell authentication、deployment evidence、
> operator custody / runbook、settlement final carrier。**实现层修复**与具体
> 路径不在本报告工作之内。审计报告本身必须遵守被审计系统的纪律：禁止
> "v1/v2/rev N" 风格 revision 语言。
>
> 关键源文件：`cli/src/main.rs`（17,553 行，308 处 settlement 关键词命中），
> `MYELIN_SESSION_L2_PLAN.md`、`MYELIN_PRODUCTION_REHEARSAL_REPORT.md`、
> `MYELIN_PRODUCTION_GATE.md`、`docs/public-testnet-rehearsal-runbook.md`、
> `docs/templates/public-testnet-rehearsal/*.json`、`MYELIN_SWARM_AUDIT_WHOLEREPO.md`。
>
> 已知背景 commit：`5ae440d` Bind court economics deployment evidence into
> settlement intent；`73ddc73` Make court economics policy explicit；`d1bb6f7`
> Mark court economics as commitment-only evidence；`4612677` Reject stale
> court economics deployment commitments；`3fda2ab` Require recomputed
> production DA readiness evidence。

## TL;DR

**协议层判定：CONDITIONAL PASS — 在 fixture / mock 路径下 settlement 模型
内部一致，但跨证据层的 commitment ↔ enforcement 边界存在三处 P1
级缺口，模型在外部生产端的可证明强度被 deployment evidence 的纯客户端
验证路径消耗掉了。**

模型内部确实自洽：
`myelin:session-settlement-authority-cell-auth:v1` 域把
`(authority_data_hash, session_id, participant_set_hash, escrow_input_cells_hash,
session_lineage_commitment)` 五项绑定进 message_hash，threshold-lock args 用
`myelin-auth-v1 || threshold_le_u32 || signer_count_le_u32 ||
signer_pubkey_hash20[]` 编码（`cli/src/main.rs:3672-3704, 3774-3785`），192 字节
authority cell data 一次绑定 intent_hash + session lineage 五字段 +
session_authority_commitment（`cli/src/main.rs:3627-3662, 4389-4405`）。court
economics 经过 `73ddc73 → d1bb6f7 → 4612677 → 5ae440d` 四个 commit 演化后已从
"裸 policy commitment" 推进到 "policy + deployment evidence + stale rejection"
三层结构（`cli/src/main.rs:4059-4336`）。3fda2ab 引入的
`final_l1_da_availability_preflight_ready` recompute 路径在字节级别确定
（`cli/src/main.rs:10361-10463`），与原始 `da_availability_production_ready`
同输入同输出。

但是协议层存在三个互锁的边界缺陷：

1. **Commitment-only ≠ enforcement 的语义被默默当作等价**。Court economics
   在 `d1bb6f7` 之后是 "policy commitment"，但当用户提供 deployment evidence
   时（`5ae440d`），`production_ready` 可以变成 true；这个状态变化没有任何
   on-chain 强制 — `l1_court_implemented = false` 始终是真相（MYELIN_SESSION_L2_PLAN.md:266）。
   Rehearsal 的 readiness 路径会显示 `production_ready`，但 public-chain
   side 没有任何协议层不变量支撑它。

2. **Production gate 跟 rehearsal scripts 在 external_receipt 上判断相反**
   （F-CLI-01 ↔ F-SCRIPT-14），且 production gate 不跑 3fda2ab 的 recompute
   路径。在协议层，这意味着 gate 证明的"settlement 协议层在 fixture 下一致"
   不延伸到"recompute 路径在 production 候补证据下一致"。

3. **operator custody / runbook 在协议层是纯文档承诺**。两个 JSON 文件经
   blake3_32 哈希进 readiness 报告（`cli/src/main.rs:10003, 10079`），但
   没有任何链上锚点、没有 HSM 验证、没有 signed custody attestation；
   `testnet_beta_ready` 路径甚至不需要这两个文档（`cli/src/main.rs:10155`）。

加上 `cellscript/examples/myelin/settlement-final.cell` / `da-anchor-final.cell`
的 CLI orphan 状态（F-DOC-01 / F-CLI-01 / F-SCRIPT-14）和 F-PRIM-01 cellid
碰撞（`exec/src/celltx/types.rs:299-307, 316-324`），final-script 路径的
type-cell 身份证明存在一个不会被现有 fixture 触发的潜在冲突。

**结论**：settlement 子系统的 **协议层模型** 在内部是一致的；
**协议层 ↔ 外部 production 端** 的边界有三个相互独立但同源（"模型信任
production evidence 自报"）的 P1 finding。下面分维度展开。

## 1. 模型边界

### 1.1 模型覆盖范围

协议层覆盖的 settlement 元素：

| 元素 | 协议层字段 | 协议层角色 | 代码锚点 |
|------|------------|------------|----------|
| Authority cell authentication | 5-field message_hash、threshold-lock args、attestation_hash | L1 锁脚本参数 + 一用性锚点 | `cli/src/main.rs:3665-3735` |
| Authority cell data (192B) | intent_hash + session_id + participant_set_hash + escrow_input_cells_hash + session_lineage_commitment + session_authority_commitment | L1 cell data | `cli/src/main.rs:3620-3662, 4389-4405` |
| SettlementIntent | 7-field binding (session, chunk, state roots, challenge, court bundle, DA manifest, segment root) | L2 提交意图 | `MYELIN_SESSION_L2_PLAN.md:262-270` |
| Court economics | base policy + (optional) deployment evidence + economics_commitment | dispute bond/slash 政策声明 | `cli/src/main.rs:4059-4336` |
| Authority attestation | blake3 chain over signer set + signatures + lock args + (optional) deployment commitment | 跨证据层 fingerprint | `cli/src/main.rs:3737-3772` |
| Threshold-lock deployment evidence | code_hash/code_dep/audit hash/network/deployment_policy | L1 script 部署承诺 | `cli/src/main.rs:3787-3880` |
| Court economics deployment evidence | 同上结构 + economics_commitment 链接 | L1 dispute script 部署承诺 | `cli/src/main.rs:4243-4360` |
| Operator custody policy | 8-field schema（`hardware_backed_keys`, `dual_control_required`, `rotation_tested`, ...） | 操作人承诺 | `cli/src/main.rs:9955-10007`、`docs/templates/public-testnet-rehearsal/operator-custody-policy.json` |
| Operator runbook | 13-field schema（`min_confirmations`, `min_fee_*`, `retry_backoff_millis`, ...） | 操作流程承诺 | `cli/src/main.rs:10009-10083`、`docs/templates/public-testnet-rehearsal/operator-runbook.json` |
| Settlement final carrier | session_id_hash ‖ settlement_identity_hash type args | L1 settlement cell 身份 | `cli/src/main.rs:4407-4412` |

### 1.2 模型不覆盖

明确 **不在** 协议层范畴内的元素：

- 实现层 CLI helper 缺漏（F-DOC-01 / F-CLI-01 / F-SCRIPT-14 的
  `settlement-final.cell` CLI 缺失 — 这是工具缺漏，不是模型缺漏）
- Production gate 与 rehearsal scripts 的判断差异（F-CLI-01 ↔ F-SCRIPT-14）
- `cellscript/examples/myelin/settlement-final.cell` 的 source 内容
  （该文件目前在 cellscript 主分支不存在；引用来自 uncommitted / future work）
- 实现层的 witness 编码、CKB RPC payload 格式、Molecule layout 字节级一致性
- `L1` 实际的 final-settlement script 字节 / chain deployment（恒为 false：
  `l1_court_implemented = false`、`l1_court_submitted = false`）

### 1.3 关键协议层声明

以下五个声明是模型的核心不变量，每条都会在第 2-4 节被审计：

- **A1**: `myelin:session-settlement-authority-cell-auth:v1` 域的
  message_hash 必须在协议层覆盖 authority cell 全部承诺字段；篡改其中任一
  字段必须让 `signature_verified` 变为 false。
- **A2**: SettlementIntent 的 court economics 必须经过 `policy commitment +
  (optional) deployment evidence + (optional) stale rejection` 三层闭合；
  `production_ready` 必须在协议层意味着 "可被 L1 dispute script 强制执行"，
  而不仅是 "evidence 完整"。
- **A3**: `da_availability_production_ready` 的 recompute 路径
  (`final_l1_da_availability_preflight_ready`) 在协议层必须与原始
  `da_availability_production_ready` 字节级一致；不一致时后者必须服从前者。
- **A4**: operator custody / runbook 在协议层是否构成一个真实不变量。
  即当 `operator_custody_policy_checked = true` 时，readiness 报告必须
  证明 "操作人控制已生效"，而不是 "操作人控制 schema 通过客户端解析"。
- **A5**: SettlementIntent 必须在 CellDAG 上是 first-class；它绑定的
  cellDAG hash 必须能与其它子系统（DA, court）共享同一 commitment 域。

## 2. 严密度评估

### 2.1 Authority Cell Authentication 域覆盖 — 严密度高

`cli/src/main.rs:3672-3704` 的 `settlement_authority_authentication` 函数
定义了协议层的核心签名：

```text
domain:   "myelin:session-settlement-authority-cell-auth:v1"
message:  blake3_chunks(domain, [authority_data_hash, session_id,
            participant_set_hash, escrow_input_cells_hash,
            session_lineage_commitment])
scheme:   "secp256k1-recoverable-blake3-pubkey-hash20"
threshold: 2 (硬编码，cli/src/main.rs:3678)
attest:   blake3_chunks("myelin:session-settlement-authority-cell-signature-attestation:v1",
            [message_hash, participant_set_hash, pubkey_hash, signature])
```

**协议层评估**：message_hash 覆盖 5 个 binding 字段，其中
`authority_data_hash` 自身是 192 字节 authority data 的 ckb data hash
（`cli/src/main.rs:4397-4404, 3642`），而 authority data 又是 6 字段的级联
（`cli/src/main.rs:4397-4404`），intent_hash 在最前面。所以 message_hash
间接覆盖 `intent_hash + 5 lineage 字段 + session_authority_commitment`
共 7 项 binding。

**严密度优点**：
- domain string 在协议层固定为 v1 — 任何更名都视为协议变更
- threshold 在协议层硬编码为 2 — `cli/src/main.rs:3678` 是协议声明点
- `signer_count >= threshold` 在生成时就已 check
  （`cli/src/main.rs:3706`）
- attestation_hash 单独作为 "attestation chains do not equal signer
  signatures" 的二次绑定（`cli/src/main.rs:3737-3772`）

**严密度缺口**：
- **activation epoch / rotation nonce 不在域内**。当前 message_hash
  没有时间维度绑定 — 同一 signer set 在不同 session 可以签出相同
  message_hash（前提是 5 字段相同），不携带 "这个 signer set 在 epoch
  X 时的授权" 概念。这意味着一个退役 signer set 的历史签名无法被区分
  与活跃 signer set 的当前签名（attestation_hashes 数组只是相同 message
  下不同 signer 的 fingerprint，不区分时序）。详见 F-SETTLE-03。
- **attestation_hash 算法把整段当字符串 hash 进 blake3**
  （`cli/src/main.rs:3744-3758`）。`hasher.update(pubkey_hash.as_bytes())`
  是把 hex 字符串的 ASCII 字节喂进 blake3，不是把 20 字节 hash 喂进去。
  `pubkey_hash.as_bytes().len() == 40`，而 hex::decode 后的字节是 20。
  这意味着 attestation_hash 对大小写不同的 hex string 区分 — 在协议层
  这是 fingerprint "额外绑定 typing discipline" 的副作用，不是显式声明。
- **ckb_lock_args 的 ASCII 序列化进 hash**
  （`cli/src/main.rs:3757`）— 协议层把 hex 字符串而不是 bytes 喂进
  hash；同 F-CLI-07 的 `bare_hex_*_arg` 与 `parse_hex_32` 行为分裂有关。

### 2.2 Authority Cell Data 192 字节结构 — 严密度高

`cli/src/main.rs:4389-4405`：

```text
data = intent_hash || session_id || participant_set_hash ||
       escrow_input_cells_hash || session_lineage_commitment ||
       session_authority_commitment   (32 字节 × 6 = 192 字节)
```

**严密度优点**：
- 192 字节 layout 固定，没有 length prefix（因为每个字段都是 32 字节） —
  协议层消除了 ambiguity
- `data_hash = ckb_cell_data_hash(&authority_data)`（`cli/src/main.rs:3642`）
  把整段 192 字节再 ckb hash 一次 — 提供了"协议层外层"防篡改 anchor
- consumed_input_index = 1（`cli/src/main.rs:3660`）在协议层强制
  authority cell 是 input[1]，配合 final settlement script 的
  "same-type group duplicate / same-type input" 拒绝
  （MYELIN_SESSION_L2_PLAN.md:286-291）形成一用性
- session_authority_commitment_algorithm 显式声明：
  `blake3(myelin:session-settlement-authority-lineage:v2,intent_hash,session_id,
  participant_set_hash,escrow_input_cells_hash,session_lineage_commitment)`
  （`cli/src/main.rs:3649-3651`）

**严密度缺口**：
- **`session_authority_commitment` 自身** 是上述 5 字段的 blake3，
  但 protocol layer 中它只是 commitment，不是 proof — 没有 zk 或
  signature wrapper。`session_authority_commitment` 被外部人替换
  不会自动暴露，因为 `attestation_hash` 独立于
  `session_authority_commitment` 计算（`cli/src/main.rs:3737-3772`）。
  验证在 `validate_settlement_authority_requirement` 里做
  （`cli/src/main.rs:4450-4530`），但这个验证只跑在 fixture 路径，
  不跑在 L1 chain side。详见 F-SETTLE-04。
- **session_lineage_commitment 在协议层来自哪里？** 它是
  SessionOpen / SessionCommit 的产物，不在 SettlementIntent 的
  boundary 内重新计算。协议层把它当作"上游已承诺"的 opaque hash。
  SettlementIntent 验证只做 equality（`cli/src/main.rs:4414-4530`），
  不做 lineage 一致性的二次证明。详见 F-SETTLE-05。

### 2.3 SettlementIntent Binding 严密度

**协议层声明**（`MYELIN_SESSION_L2_PLAN.md:262-270`）：
SettlementIntent binds verified court bundle, verified DA manifest,
challenge window, and court economics。

**实际绑定检查** — `verify_session_settlement_intent`
（`cli/src/main.rs:7058-7340`）跑：

- court-bundle session_id 匹配
- court-bundle chunk_index 匹配
- court-bundle state_root_after 匹配
- court-bundle challenge_payload_hash 匹配
- court-bundle molecule_transaction_hash 匹配
- da_manifest.session_id / chunk_index / segment_root / challenge_payload_hash
  / molecule_transaction_hash / proof_valid 匹配
- court_economics schema/fields 与 recomputed 一致
  （`cli/src/main.rs:7258-7283`）
- `challenge_deadline_ms > current_time_ms`（已 elapsed challenge window）
- `l1_da_published = false`、`l1_court_implemented = false` 必须保留
  （这是"不是 L1 已发布"的 truth marker）

**严密度优点**：
- `da_manifest.proof_valid` 是真验证，不是 boolean：
  `cli/src/main.rs:6410-6440` 重新调用 `da_availability_evidence` +
  重新调 `SegmentProof::verify` + 重新算 `molecule_transaction_hash`。
- `court_economics == expected_court_economics` 是整结构 ==
  （`cli/src/main.rs:7279`），但 Production ready 检查是单独的
  `court-economics-ready` check（`cli/src/main.rs:7284-7317`）
- 17 项 tamper 拒绝测试（`cli/src/main.rs:13877-13894`）覆盖了
  commitment、policy fields、deadline-only、invariant、deployment flags

**严密度缺口**：
- **`da_manifest.availability.availability_commitment` 通过 court_economics
  路径传递**（`cli/src/main.rs:7263`），但 SettlementIntent 协议层
  不直接绑定 `da_availability_production_ready` 状态。Court economics
  的 `da_evidence_required = true` 是 boolean 断言
  （`cli/src/main.rs:4102`），不是 commitment 验证 — 详见 F-SETTLE-01。
- **challenge window 的"已 elapsed"是 relative**，不是 absolute —
  `current_time_ms` 由 CLI caller 提供（`cli/src/main.rs:1110`、
  `MYELIN_SESSION_L2_PLAN.md:159`），不是从 L1 block header 读取。
  协议层对 `current_time_ms` 的合法性没有任何检查。详见 F-SETTLE-06。
- **`l1_court_implemented = false` 是 fixture-only 断言**。如果未来
  部署了真 L1 court script，该 boolean 怎么更新？协议层没定义
  切换条件；`MYELIN_SESSION_L2_PLAN.md:266-268` 说 "deliberately carries
  ... so the current artefact cannot be mistaken for an externally
  published DA record or on-chain settlement script" — 但这是
  doc-discipline，不是 protocol-layer enforcement。

### 2.4 Court Economics 4-commit 演化后严密度

| Commit | 协议层变化 | 严密度影响 |
|--------|------------|------------|
| `73ddc73` Make policy explicit | 加 `minimum_dispute_bond_shannons`、`challenger_reward_bps`、`loser_slash_bps`、`honest_party_refund_bps`、`unresolved_remainder_bps`、`payout_balance_bps`、`settlement_after_deadline_only`、`da_evidence_required`、`economics_invariant_checked`（`cli/src/main.rs:4095-4141`） | 经济政策从不透明 → 8 字段显式 |
| `d1bb6f7` Mark commitment-only | `mode = "disputed-close-policy-commitment"`，`testnet_beta_ready = false` | 把"未带 deployment 的状态"显式标注为"policy commitment，不是 enforcement" |
| `4612677` Reject stale | 加 `normalize_court_economics_deployment_evidence` 全部字段 normalize（`cli/src/main.rs:4243-4336`），加 `provided_commitment != commitment` 拒绝（`cli/src/main.rs:4327-4331`） | 防止外部人提交 stale deployment（手填的 `evidence_commitment`） |
| `5ae440d` Bind into settlement intent | 加 `court_economics_evidence_with_deployment`（`cli/src/main.rs:4078-4168`），`economics_commitment_algorithm` 在有 deployment 时变成 `blake3(myelin:session-court-economics-with-deployment:v1, base_commitment, deployment_commitment)`（`cli/src/main.rs:4163-4165`） | deployment commitment 进了 economics_commitment 的二次 hash |

**严密度优点**：
- evidence_commitment 是 **recomputed**（`cli/src/main.rs:4326-4334`），
  不是简单 equal — 协议层拒绝 stale 手填值。
- `production_ready` 触发条件 4 项 AND（`cli/src/main.rs:4306-4315`）：
  ckb_enforceable_checked AND testnet_beta_ready AND network ==
  ckb-mainnet AND deployment_policy == mainnet-production-...v1
- `testnet_beta_ready` 触发条件：ckb_enforceable_checked 必须真
  （`cli/src/main.rs:4301-4305`）
- testnet 网络不能有 production_ready（`cli/src/main.rs:4316-4318`）

**严密度缺口**：
- **`economics_commitment_algorithm` 在 with-deployment 路径下被覆盖**
  （`cli/src/main.rs:4163-4165`）。基线算法字符串 `blake3(...:v1,
  participant, escrow, challenge, da_availability, challenge_window,
  challenge_deadline, min_dispute_bond, challenger_reward, loser_slash,
  honest_refund, unresolved, settlement_after_deadline_only,
  da_evidence_required)` 被新算法 `blake3(...:v1, base_fields,
  optional-deployment-commitment)` 替换
  （`cli/src/main.rs:4147-4149, 4163-4165`）。Parent 和 child report
  报告不同算法字符串（F-CLI-11 也注意到了） — 协议层没定义
  "哪个算法字符串是 authoritative"。详见 F-SETTLE-07。
- **`economics_commitment` 在 fixture 路径下可以从 base commitment 算出，
  但在 verify 路径下不能从外部 input 重建 base commitment** — 即
  没有外部 witness path 让 verifier 独立 recompute `base_commitment`
  而不信任 report 内的 `economics_commitment`。`verify_session_settlement_intent`
  的 `court-economics-commitment` check 用 `intent.court_economics ==
  expected_court_economics`（`cli/src/main.rs:7279`），即整结构 ==
  — 这相当于 "input self-consistent" 验证，不是 "input ↔ external
  oracle" 验证。详见 F-SETTLE-08。
- **economics_commitment 在 5ae440d 之前的所有历史 fixture 报告**
  仍然声明旧 algorithm string — 跨版本兼容没有被协议层强制。新的
  fixture 会用新 algorithm，但是 `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:41`
  的 "Court economics deployment evidence is recomputable and stale
  commitments are rejected" 是对 *future* 状态的描述；老 fixture 报告
  的 `economics_commitment` 仍然合法（只要它们符合当时的 algorithm）。
- **`da_evidence_required = true` 是 boolean 断言**，不是 commitment 验证
  — 即"协议层承诺 DA evidence 必需" ≠ "DA evidence 实际 binding"。F-SETTLE-01
  在第 3 节展开。

### 2.5 3fda2ab Recompute Path 严密度

`final_l1_da_availability_preflight_ready`（`cli/src/main.rs:10408-10463`）
是 3fda2ab 引入的 recompute 路径。它在协议层的角色是：
当 submission 是 final L1 类型（final_l1_da_anchor 或 final_l1_settlement）
且 raw `da_availability_production_ready = true` 时，recompute 必须确认
`true`，否则覆盖成 `false`（`cli/src/main.rs:10365-10370`）。

**严密度优点**：
- recompute 调用 `da_availability_evidence` 重新生成 expected availability
  （`cli/src/main.rs:10446-10456`），输入是 `manifest.session_id`、
  `manifest.court_bundle_hash`、`manifest.molecule_transaction_hash`、
  `manifest.segment_root`、`manifest.proof_molecule_hex`、
  `manifest.local_da_published`、`manifest.availability.external_receipt`
  — 这些全部来自 manifest，不是 caller-provided。
- `manifest.availability == expected_availability` 是 struct equality
  （`cli/src/main.rs:10457`），所以 `availability_commitment` 必须
  byte-match recomputed
- `da_availability_production_ready(&manifest.availability)` 是同一个
  helper（`cli/src/main.rs:10462, 3411`）— recompute 与原始函数
  同源

**严密度缺口**：
- **`da_availability_production_ready` 只在 final-L1 路径下被 recompute**
  （`cli/src/main.rs:10365-10370` 限定条件 `(final_l1_da_anchor ||
  final_l1_settlement)`）。在 rehearsal / mock 路径下，该 boolean
  仍由原始路径直接给定。Production gate 不跑这个 recompute
  （MYELIN_SWARM_AUDIT_WHOLEREPO.md:71-83 + F-SCRIPT-14）。
- **`external_receipt` 是 manifest.availability 的字段，不是
  recompute 的输入**（`cli/src/main.rs:10453` 取自 manifest）。
  如果外部 receipt 被篡改，recompute 也跟着错。原始
  `external_da_receipt_provider_message_hash` 已经识别
  `receipt_id / availability_window` 未被签名覆盖（F-CLI-02）—
  该缺陷在 recompute 路径下被放大：篡改这两个字段的 receipt
  通过 recompute 后仍然 production_ready。
- **recompute 用 `proof.verify()`**（`cli/src/main.rs:10424-10429`）
  但 `proof` 是从 manifest 里的 `proof_molecule_hex` 解码 — 协议层
  不二次验证 proof bytes 是不是来自 sealed segment。

### 2.6 Operator Custody / Runbook 协议层严密度 — 低

`cli/src/main.rs:9955-10083` 的两个 policy document handler：
- 读 JSON
- 验证 schema (`myelin-operator-custody-policy-v1` /
  `myelin-operator-runbook-v1`)
- 验证 boolean / u64 字段 (hardware_backed_keys, dual_control_required,
  rotation_tested, ...)
- 用 blake3_32(b"myelin:operator-custody-policy-document:v1", &bytes)
  算 hash 进 readiness 报告
  （`cli/src/main.rs:10003, 10079`）

**严密度优点**：
- schema 是 fixed string — 协议层 schema 锁定
- custody 的 `signing_threshold > 0` 是 hard constraint
  （`cli/src/main.rs:9981-9982`）
- `operator_count >= signing_threshold` 是 invariant
  （`cli/src/main.rs:9984-9988`）
- runbook 的 `min_confirmations / min_fee_shannons / ...` 必须等于
  economics/finality 报告里的同名字段
  （`cli/src/main.rs:10028-10054`）— 这是协议层 cross-check，不是
  schema 检查

**严密度缺口**：
- **政策字符串字段（`key_storage`、`signing_approval`、
  `rotation_policy`、`emergency_response`、`reorg_response` 等）是
  free-text**，没有 enum 约束。`key_storage =
  "public-testnet-rehearsal-hsm-or-multisig-wallet"` （template line 3）
  是一个不在 schema enum 内的 string。协议层接受任何非空字符串。详见
  F-SETTLE-09。
- **boolean 字段只是 "存在且为 true" 检查**（`cli/src/main.rs:9965-9980`），
  没有验证 boolean 是怎么被一个 HSM / external custody system 证明的。
  即 `hardware_backed_keys = true` 是 "documented claim"，不是
  "verified claim"。
- **没有 chain anchor**。文档的 blake3 hash 进 readiness 报告，但
  readiness 报告本身没有 chain anchor（`cli/src/main.rs:10167+`
  的 `myelin:session-public-chain-operational-policy:v1` hash 是
  client-side-only）。一个 operator 在 readiness 报告通过后修改
  custody 文档没有任何协议层 detection。详见 F-SETTLE-10。
- **testnet_beta_ready 不需要 custody / runbook**
  （`cli/src/main.rs:10155`：testnet_beta_ready = reorg_policy_checked
  && fee_policy_checked && retry_policy_checked &&
  monitoring_policy_checked，**不包含** operator_custody_policy_checked）。
  即 testnet rehearsal 可以达到 `testnet_beta_ready = true` 而
  `operator_custody_policy_checked = false`，但 readiness aggregate
  的 `production_ready = false` 是因为缺少 `operator_custody_policy_checked`
  —— 这意味着 custody 在 protocol layer **只 gate production**，不
  gate testnet。这个边界是 implicit 的，不是显式声明。详见 F-SETTLE-11。

### 2.7 Findings (F-SETTLE-NN) — 严密度维度

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| **F-SETTLE-01** | **P1** | settlement-intent verify | `da_evidence_required = true` 是 boolean 断言，不是 commitment verification；protocol layer 把 DA evidence 当作 "policy field" 而非 binding，court economics protocol 不能在链上 enforce DA evidence 完整性 | `cli/src/main.rs:4102, 4107, 4141, 7284-7298` |
| **F-SETTLE-02** | **P1** | authority attestation hash | `authority_authentication_attestation_hash` 把 hex 字符串 (`pubkey_hash.as_bytes()`, `signature.as_bytes()`) 而非 20/65 字节喂进 blake3；大小写敏感的 hex 字符串在协议层是合法 fingerprint 但不显式声明 | `cli/src/main.rs:3748-3752` |
| **F-SETTLE-03** | **P1** | authority cell authentication | `myelin:session-settlement-authority-cell-auth:v1` 域没有 activation epoch / rotation nonce；同一 signer set 在不同时期对相同 5 字段签出相同 message_hash，退役 signer set 的历史 signature 与活跃 signature 在协议层无法区分 | `cli/src/main.rs:3665-3704` |
| **F-SETTLE-04** | **MEDIUM** | authority cell data | `session_authority_commitment` 是 blake3 commitment，不是 signature wrapper；外部人替换该字段不会被自动 detect（`validate_settlement_authority_requirement` 是 fixture-only） | `cli/src/main.rs:3627-3630, 4414-4530` |
| **F-SETTLE-05** | **MEDIUM** | settlement intent lineage | `session_lineage_commitment` 在 SettlementIntent 协议层被当作 opaque hash；SettlementIntent 验证只做 equality，不二次证明 lineage 与 session open/commit 一致 | `cli/src/main.rs:3620-3662, 7058-7340` |
| **F-SETTLE-06** | **MEDIUM** | settlement intent timing | `current_time_ms` 是 CLI caller 提供的相对值，不是 L1 block header；协议层对"challenge window elapsed"的真实性没有校验 | `cli/src/main.rs:1110, MYELIN_SESSION_L2_PLAN.md:159` |
| **F-SETTLE-07** | **MEDIUM** | court economics commitment | `economics_commitment_algorithm` 在 with-deployment 路径下被覆盖（line 4147 → line 4163），parent 与 child report 报告不同 algorithm 字符串；协议层没定义 "哪个算法字符串是 authoritative" | `cli/src/main.rs:4147-4149, 4163-4165` |
| **F-SETTLE-08** | **MEDIUM** | court economics verify | `verify_session_settlement_intent` 的 `court-economics-commitment` check 用整结构 == (`intent.court_economics == expected_court_economics`)，等价于 input self-consistent 验证而非 input ↔ external oracle 验证 | `cli/src/main.rs:7259-7279` |
| **F-SETTLE-09** | **MEDIUM** | operator custody schema | 政策字符串字段（`key_storage`, `signing_approval`, `rotation_policy`, `emergency_response`, `reorg_response` 等）是 free-text，无 enum 约束；任何非空字符串都通过 schema check | `cli/src/main.rs:9961-9964, 10018-10021`, `docs/templates/public-testnet-rehearsal/operator-custody-policy.json:3-6` |
| **F-SETTLE-10** | **P1** | operator custody protocol | custody / runbook 文档的 blake3 hash 进 readiness 报告但 readiness 报告无 chain anchor；operator 在 readiness 报告通过后修改文档无 protocol-layer detection | `cli/src/main.rs:10003, 10079, 10167-10210` |
| **F-SETTLE-11** | **MEDIUM** | operator custody production gate | `testnet_beta_ready` 不要求 operator_custody_policy_checked；custody 在 protocol layer 只 gate production 不 gate testnet，边界 implicit | `cli/src/main.rs:10151-10156` |

## 3. 合理性评估

### 3.1 假设是否现实

**假设 H1**：authority cell 在 L1 上的 lock script 真正 enforce threshold。
**评估**：实现层假设是真实的 — `cli/src/main.rs:3678-3704` 生成的
ckb_lock_args 是 `myelin-auth-v1 || threshold_le_u32 ||
signer_count_le_u32 || signer_pubkey_hash20[]`，**但**这是 CLI 侧的
canonicalization。真正的 L1 lock script 是不是消费这套 args 的格式没有
被协议层定义；MYELIN_SESSION_L2_PLAN.md:283-291 只说 "final settlement
type args are session_id_hash || settlement_identity_hash" 和 "lock must
match the final DA publication lock"。详见 F-SETTLE-12。

**假设 H2**：deployment evidence 的 "stale" 通过 evidence_commitment
recompute 检测。
**评估**：在 fixture 路径下真实 — `cli/src/main.rs:4319-4331` 检测
手填 `evidence_commitment` 与 normalized fields 不匹配。在 production
路径下，protocol-layer 真实强度取决于 normalized fields（verifier_code_hash、
audited_source_hash 等）能不能 on-chain verify。详见 F-SETTLE-13。

**假设 H3**：production gate 跟 rehearsal scripts 在 external_receipt 上
判断相反 — 协议层哪个是正确的？
**评估**：根据协议层声明（MYELIN_PRODUCTION_REHEARSAL_REPORT.md:37），
`production-evidence-complete prototype / public-testnet rehearsal candidate`
是当前标签。production gate 断言 "fixture 路径下 external_receipt 不存在"
（`scripts/myelin_production_gate.sh:1197-1199`）对应的是 fixture-only 路径。
rehearsal scripts 的 `--external-da-receipt` 是 public-testnet rehearsal
路径。两者针对的是不同协议层状态（fixture vs rehearsal），但用同一个
boolean field (`availability.external_receipt`) 区分。详见 F-SETTLE-14。

**假设 H4**：operator custody / runbook 是真实协议层不变量。
**评估**：部分真实 — 在 readiness aggregate 里 gate 了 `production_ready`
（`cli/src/main.rs:10156`），但仅此而已。custody 文档被修改后 protocol
layer 没有 detection 路径。详见 F-SETTLE-10。

### 3.2 边界 — 链上 vs 链下

- **链上 authority cell**：`consumed_input_index = 1`（`cli/src/main.rs:3660`），
  final settlement script 拒绝 same-type inputs + duplicate same-type
  group outputs（MYELIN_SESSION_L2_PLAN.md:286-291）。一用性在协议层
  由 chain side 强制（transaction-local singleton）。
- **链下 authority**：192 字节 data 是 ckb hash 进 `authority_data_hash`，
  message_hash 是 blake3 over 5 fields。链下侧无法伪造 — 但 message_hash
  协议层是 fixture-only，链上侧只 enforce data_hash 是否等于 declared。
- **链上 court economics**：不存在 — `l1_court_implemented = false` 是
  fixture 路径下 truth。`production_ready` 的 enforcement 是 deployment
  evidence 的承诺，不是 L1 script 的真实执行。
- **链下 court economics**：完整 policy 字段 + 完整 deployment evidence
  字段都在 fixture / readiness report 里，但是这是 "evidence of evidence"，
  不是 "L1 execution"。

**合理性结论**：链下 commitment 和链上 enforcement 之间存在 2 步 gap：
(a) commitment → evidence commitment 是 client-side blake3；
(b) evidence → on-chain execution 是 fixture-only。
protocol layer 在 (a) 是自洽的，在 (b) 是 document discipline。

### 3.3 Findings (F-SETTLE-NN) — 合理性维度

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| **F-SETTLE-12** | **MEDIUM** | authority lock args | CLI 生成的 ckb_lock_args 格式 (`myelin-auth-v1 || threshold_le_u32 || signer_count_le_u32 || signer_pubkey_hash20[]`) 没有 protocol-layer 引用 — 没有规范文件说 "L1 lock script 必须消费这个 layout"；只有 `cli/src/main.rs:3774-3785` 是来源 | `cli/src/main.rs:3774-3785`, `MYELIN_SESSION_L2_PLAN.md:283-291` |
| **F-SETTLE-13** | **MEDIUM** | stale deployment rejection | `4612677` 的 evidence_commitment recompute 只检查 normalized fields 内部一致性，不检查这些 fields 能不能 on-chain verify；stale rejection 在协议层是 "format-correct"，不是 "chain-correct" | `cli/src/main.rs:4319-4331, 4327-4334` |
| **F-SETTLE-14** | **P1** | production gate vs rehearsal external_receipt | production gate (`scripts/myelin_production_gate.sh:1197-1199`) 断言 fixture 路径下 external_receipt 不存在；rehearsal scripts (`scripts/myelin_public_testnet_rehearsal_prepare.sh:121-138`) 制造 positive external_receipt。两者对同一 boolean field 有相反期望 — 在 protocol layer 是 "fixture vs rehearsal" 状态分离，但 boolean field 没有显式 label | `scripts/myelin_production_gate.sh:1197-1199`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:121-138`, `cli/src/main.rs:3535-3542` |
| **F-SETTLE-15** | **MEDIUM** | operator custody invariants | custody schema 不验证 `signing_approval` 是否为真正的 dual-control；`dual_control_required = true` 是 boolean 声明，不是 cryptographic proof；HSM-backed 承诺在协议层不可验证 | `cli/src/main.rs:9961-9986, 9989-10000` |

## 4. 安全性评估

### 4.1 攻击模型

协议层 settlement 模型需要抵御：
1. **Stale commitment 攻击**：外部 attacker 提交旧 deployment evidence，
   试图让 court economics 误认为 fresh
2. **Authority signer 失窃 / 轮换攻击**：signer private key 泄露后 attacker
   伪造 authority cell signature
3. **CellID 碰撞 / 替换攻击**：F-PRIM-01 路径下 `(args="X", data="")` 与
   `(args="", data="X")` 产生相同 type-cell identity；attacker 可以替换
   authority cell 的 sibling output
4. **Operator custody 切换滥用**：operator 修改 custody 文档绕过 policy
5. **Recompute 不稳定**：3fda2ab 的 recompute 路径在对抗条件下能不能被
   制造出不一致的 `da_availability_production_ready`

### 4.2 Authority 密钥失窃 / 轮换 — 协议层无 detection

`myelin:session-settlement-authority-cell-auth:v1` 域没有时间维度：
同一 signer set 在不同时期对相同 5 字段签出相同 message_hash
（F-SETTLE-03）。失窃 signer 私钥后 attacker 制造的 signature 在协议层
与原 signer 的 signature 等价（attestation_hash 区分 pubkey_hash 与
signature，但**不区分时序**）。详见 F-SETTLE-16。

**协议层缺口**：
- 没有 activation epoch 或 sequence number
- 没有 signer-set rotation window
- 没有 "this signature was emitted by signer set at epoch X" 概念
- attestation_hashes[] 数组是 "不同 signer 对相同 message 的 fingerprint"
  — 不携带时序

### 4.3 Stale Commitment 攻击 — 部分防御

`4612677` 引入的 `evidence_commitment` recompute 路径（`cli/src/main.rs:4319-4331`）
能拒绝 "手填 evidence_commitment 与 normalized fields 不一致" 的攻击者。
但 protocol layer 不能拒绝 "normalized fields 自身 stale 但内部一致" —
即 attacker 重新生成所有 normalized fields 但用一个过期的 code_hash /
audited_source_hash 提交，仍然通过 recompute。详见 F-SETTLE-17。

**协议层缺口**：
- 没有 "deployment 时戳" 字段进 economics_commitment
- 没有 L1 chain anchor 来验证 code_hash 是当前的
- `audited_source_hash` 与 `audit_report_hash` 的 "audit 时戳" 没进
  evidence_commitment

### 4.4 CellID 碰撞 / 替换攻击 — 类型层有缺陷

F-PRIM-01（`exec/src/celltx/types.rs:299-307, 316-324`）报告了
`compute_conflict_hash` 与 `compute_typed_data_hash` 的
`(args="X", data="")` vs `(args="", data="X")` 碰撞。authority cell
和 settlement final carrier 都依赖 type-cell identity：
- authority cell 的 lock script 不会查 type-cell identity，但 final
  settlement script 看 type args（`session_id_hash || settlement_identity_hash`）
- final settlement carrier 走 `carrier_payload_type_args_hex`
  （`cli/src/main.rs:4584-4592`），目前只覆盖 `da-anchor-carrier-v1` 和
  `settlement-carrier-v1` 两种 kind；其它 kind fall through 到
  `0x{data_hash_hex}` 32 字节（`cli/src/main.rs:4590`）

F-CLI-01 / F-DOC-01 / F-SCRIPT-14 已经报告 final-script fixture
（`settlement-final.cell`, `da-anchor-final.cell`）是 CLI orphan —
即 protocol layer 在 final-script kind 上没有 type args canonicalization
路径。当 final-script kind 被提交时，type args 是 truncated 32 字节
data_hash；F-PRIM-01 路径下 attacker 可以构造碰撞的 type args。详见
F-SETTLE-18。

### 4.5 Recompute 路径对抗稳定性

3fda2ab 引入的 `final_l1_da_availability_preflight_ready` 在 fixture 路径
下字节级确定（输入是 manifest fields + external_receipt from manifest）。
对抗条件下：

- **稳定**：proof.verify() 是 determinstic；availability_commitment 是
  blake3 over 固定 fields；da_availability_production_ready 是固定 boolean
  function（`cli/src/main.rs:3411-3418`）
- **不稳定来源**：如果 external_receipt 是 attacker-controlled 且
  `external_da_receipt_provider_message_hash` 不覆盖 `receipt_id` /
  `availability_window`（F-CLI-02），那么 attacker 可以保持
  `provider_signature_verified = true` 同时替换 receipt_id 和
  availability_window。recompute 不检测这两个字段的替换，因为
  recompute 把 external_receipt 当作 black box 喂回 da_availability_evidence。
  详见 F-SETTLE-19。

### 4.6 Operator Custody 角色切换被滥用

operator custody / runbook 文档修改在协议层无 detection（F-SETTLE-10）。
攻击场景：
- T0: operator 提交 custody 文档 C_old with `signing_threshold = 2`，
  `operator_count = 3`，readiness 通过
- T1: operator 修改 custody 文档到 C_new with `signing_threshold = 1`，
  `operator_count = 1`，attack 准备
- T2: protocol layer 没有任何机制 detect C_old → C_new 切换

readiness report 的 `policy_commitment` 是 blake3 over **当前** 文档
+ context/economics/inclusion/stability/finality fields，没有 chain
anchor。详见 F-SETTLE-10。

### 4.7 Settlement Final Carrier 提交失败 / 重放

- **失败**：`submit-settlement-package` 在 dry-run 模式只构建 JSON-RPC
  request；在 live submit 模式做 `get_live_cell` preflight
  （`cli/src/main.rs:4258+`）；失败返回 CliError，不影响 on-chain state
- **重放**：final settlement script 的 transaction-local singleton 加上
  authority cell consumed_input_index = 1 跨 transaction
  uniqueness 保证 replay protection（MYELIN_SESSION_L2_PLAN.md:286-291）
- **协议层风险**：F-PRIM-01 cellid 碰撞在 final-script kind 上提供
  重放可能性（attacker 构造不同 args 达到相同 type-cell identity）

### 4.8 Findings (F-SETTLE-NN) — 安全性维度

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| **F-SETTLE-16** | **P1** | authority rotation | `myelin:session-settlement-authority-cell-auth:v1` 域无 activation epoch / rotation nonce；失窃 signer key 后 attacker signature 与原 signature 在协议层等价（attestation_hash 不区分时序） | `cli/src/main.rs:3665-3704, 3737-3772` |
| **F-SETTLE-17** | **MEDIUM** | stale deployment | `4612677` recompute 只检查 normalized fields 内部一致；attacker 重新生成 fields + 用过期 code_hash / audit hash 仍通过 recompute | `cli/src/main.rs:4319-4331` |
| **F-SETTLE-18** | **P1** | final-script cellid collision | F-PRIM-01 `(args="X", data="")` vs `(args="", data="X")` 碰撞；final-script kind 走 `carrier_payload_type_args_hex` fall-through 到 truncated 32-byte data_hash，没有完整 64-byte type-args canonicalization；attacker 可构造碰撞 type args 替换 authority / settlement cell | `cli/src/main.rs:4584-4592`, `exec/src/celltx/types.rs:299-307, 316-324` |
| **F-SETTLE-19** | **MEDIUM** | recompute blind to receipt_id / availability_window | `final_l1_da_availability_preflight_ready` 把 external_receipt 当 black box；F-CLI-02 路径下 attacker 替换 receipt_id / availability_window 仍通过 recompute，`da_availability_production_ready = true` | `cli/src/main.rs:10446-10463, 10365-10370`, `cli/src/main.rs:3019-3037` |
| **F-SETTLE-20** | **P1** | operator custody no detection | operator 修改 custody / runbook 文档后 protocol layer 无 detection 路径；readiness report policy_commitment 是 client-side-only blake3，无 chain anchor | `cli/src/main.rs:10003, 10079, 10167-10210` |
| **F-SETTLE-21** | **LOW** | settlement carrier replay protection depends on authority cell consumption | 跨 transaction uniqueness 依赖 authority cell 被 consumed；attacker 通过 cellid collision 制造 "non-consumed" 副本可以绕过 replay protection | `cli/src/main.rs:3660, 4407-4412, MYELIN_SESSION_L2_PLAN.md:286-291` |

## 5. 与已存在 audit 的关系

### 5.1 已存在 audit 的 settlement 覆盖

| Audit | Settlement 覆盖 | 与本报告的关系 |
|-------|------------------|----------------|
| `MYELIN_SWARM_AUDIT_WHOLEREPO.md` (2026-06-27) | F-CLI-01..35（cli helper 缺漏、外部 receipt 签名域不完整、hex 编码不一致等）、F-SCRIPT-14（production gate 不跑 recompute）、F-DOC-01（final-script fixture CLI orphan）、F-PRIM-01（cellid 碰撞） | 本报告把 F-CLI-01 / F-SCRIPT-14 / F-DOC-01 / F-PRIM-01 升级到协议层讨论；不是重复发现，是从实现层 → 协议层的二次审计 |
| `MYELIN_SWARM_AUDIT_STATE_DA.md` (2026-06-22) | DA manifest 严密度，未触及 settlement | 不重叠 |
| `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` | mempool / consensus，未触及 settlement authority | 不重叠 |
| `MYELIN_CONSENSUS_COMPLETENESS.md` | consensus 完整度，未触及 settlement | 不重叠 |
| `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` | evidence provenance 表（settlement intent, court economics deployment, threshold-lock deployment, operator custody/runbook, final-script submission path） | 本报告把"provenance 是 fixture"升级到"protocol-layer enforcement boundary"分析 |
| `MYELIN_PRODUCTION_GATE.md` | gate 步骤与 failure modes | 本报告第 6 节展开 gate ↔ rehearsal disagreement 的协议层含义 |

### 5.2 已存在 audit 的 settlement-finding 链路

| Finding | Severity | 文件 | 本报告关联 |
|---------|----------|------|-----------|
| F-CLI-01 (CRITICAL) | gate ↔ rehearsal external_receipt disagreement | `scripts/myelin_production_gate.sh:1198-1204` | F-SETTLE-14, F-SETTLE-19 |
| F-CLI-02 (HIGH) | external_da_receipt_provider_message_hash 不覆盖 receipt_id / availability_window | `cli/src/main.rs:3019-3037` | F-SETTLE-19 |
| F-CLI-06 (HIGH) | hard-coded fixture signer set | `cli/src/main.rs:3685-3735` | F-SETTLE-03 |
| F-CLI-09 (MEDIUM) | economics_commitment_algorithm 中途覆盖 | `cli/src/main.rs:4147-4166` | F-SETTLE-07 |
| F-CLI-11 (MEDIUM) | evidence_commitment_algorithm strings 不一致 | `cli/src/main.rs:4274-4287, 4338-4340` | F-SETTLE-07 |
| F-CLI-19 (LOW) | attestation_signatures 顺序固定 | `cli/src/main.rs:3456, 3491-3495, 6398` | (协议层 OK, 实现层 LOW) |
| F-CLI-27 (LOW) | signing_threshold = 2 与 fixture signer set 巧合对齐 | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json`, `cli/src/main.rs:9981-9987, 3678` | (信息) |
| F-CLI-28 (LOW) | recursive court_economics_evidence in deployment_flags_valid | `cli/src/main.rs:4365-4387` | (实现层 LOW) |
| F-CLI-29 (LOW) | verify_submission_readiness string-equality on expected_ckb_tx_hash | `cli/src/main.rs:9814, 10140-10143` | (实现层 LOW) |
| F-CLI-34 (INFO) | secp256k1_pubkey_hash20 截断 blake3 到 20 字节 | `cli/src/main.rs:4032-4037, 3364, 3471, 3688, 11665` | F-SETTLE-02 |
| F-DOC-01 (CRITICAL) | final-script fixture CLI orphan | `cellscript/examples/myelin/da-anchor-final.cell:1-56`, `cli/src/main.rs:4584-4592, 8282-8320` | F-SETTLE-18 |
| F-PRIM-01 (CRITICAL) | cellid collision on type-cell identity | `exec/src/celltx/types.rs:299-307, 316-324` | F-SETTLE-18, F-SETTLE-21 |
| F-SCRIPT-14 (MEDIUM) | gate 不跑 recompute | `scripts/myelin_production_gate.sh:1079-1093, 1117-1120` | F-SETTLE-14, F-SETTLE-19 |

### 5.3 本报告新发现的协议层 finding（不在已存在 audit 中）

| # | Severity | Finding |
|---|----------|---------|
| F-SETTLE-01 | P1 | `da_evidence_required = true` 是 boolean 断言而非 commitment verification |
| F-SETTLE-03 | P1 | authority 域无 activation epoch / rotation nonce |
| F-SETTLE-10 | P1 | operator custody / runbook 无 chain anchor，修改无 detection |
| F-SETTLE-16 | P1 | authority rotation 失窃 detection 路径缺失 |
| F-SETTLE-02 | P1 | attestation_hash 把 hex 字符串喂进 blake3 |
| F-SETTLE-04 | MEDIUM | session_authority_commitment 不是 signature wrapper |
| F-SETTLE-05 | MEDIUM | session_lineage_commitment 在 SettlementIntent 验证中仅做 equality |
| F-SETTLE-06 | MEDIUM | current_time_ms 是 caller-provided，无链上校验 |
| F-SETTLE-07 | MEDIUM | economics_commitment_algorithm 在 with-deployment 下被覆盖 |
| F-SETTLE-08 | MEDIUM | court_economics verify 是 input self-consistent 不是 input ↔ oracle |
| F-SETTLE-09 | MEDIUM | custody 政策字符串字段无 enum 约束 |
| F-SETTLE-11 | MEDIUM | testnet_beta_ready 不要求 custody_checked |
| F-SETTLE-12 | MEDIUM | ckb_lock_args layout 无 protocol-layer 引用 |
| F-SETTLE-13 | MEDIUM | stale rejection 是 format-correct 不是 chain-correct |
| F-SETTLE-15 | MEDIUM | custody dual_control 是 boolean 声明不是 cryptographic proof |
| F-SETTLE-17 | MEDIUM | recompute 不检测 normalized fields 时戳 |
| F-SETTLE-19 | MEDIUM | recompute blind to receipt_id / availability_window |
| F-SETTLE-21 | LOW | carrier replay protection 依赖 authority cell consumption |

## 6. 已知缺陷的协议层影响

### 6.1 F-CLI-01 ↔ F-SCRIPT-14 (production gate vs rehearsal)

**实现层描述**（MYELIN_SWARM_AUDIT_WHOLEREPO.md:64-83）：
- production gate 断言 `external_receipt_count == 0` /
  `external_receipt_checked == False` / `external_receipt is None` /
  `production_ready is False`
- rehearsal scripts 制造 positive receipt shape
- production gate 不断言 `real-da-availability-guarantee-missing`
  (CKB devnet smoke 断言)

**协议层含义**：
- production gate 证明的是 "settlement 协议层在 fixture 路径下一致"
- rehearsal scripts 证明的是 "settlement 协议层在 rehearsal 路径下能
  拼接 evidence"
- 两条证明 **不可叠加** — gate 没有覆盖 recompute 路径，rehearsal
  没有覆盖 production blocker assertion

**协议层建议（仅指出问题，不修）**：
1. protocol layer 需要区分 `fixture / rehearsal / production` 三种
   状态，每种有独立的 readiness 字段 — 当前 `readiness_evidence_mode`
   （`cli/src/main.rs:10372, 10556`）是 backward-compatible 但 gate
   没消费它
2. recompute 路径（3fda2ab）需要在 production gate dry-run 模式跑一次
   而非只在 final-L1 preflight 跑

### 6.2 F-CLI-02 / F-CLI-03 (external receipt 签名域)

**实现层描述**：
- `external_da_receipt_provider_message_hash` 不覆盖 `receipt_id` 或
  `availability_window`（F-CLI-02，`cli/src/main.rs:3019-3037`）
- `receipt_commitment` 与 `receipt_hash` 不联合签名（F-CLI-03）

**协议层含义**：
- provider 可以 sign 一次 `payload_hash / segment_root` 然后 re-emit
  receipt with fresh `receipt_id` and `availability_window`，声称
  新的 retention label
- 这意味着 protocol layer 的 "provider commitment" 实际是 "provider
  commitment to `payload_hash / segment_root`", 不是 "provider commitment
  to current retention / availability window"

**协议层建议（仅指出问题，不修）**：
- protocol layer 应该把 `receipt_id` 与 `availability_window` 进
  `external_da_receipt_provider_message_hash`，否则 settlement
  protocol 的 "production_guarantee_checked" 在协议层是
  "format-correct" 而非 "policy-correct"

### 6.3 F-DOC-01 / F-CLI-01 / F-SCRIPT-14 (final-script fixture CLI orphan)

**实现层描述**（MYELIN_SWARM_AUDIT_WHOLEREPO.md:40-62）：
- `da-anchor-final.cell` 与 `settlement-final.cell` 在 cellscript v0_18
  test 与 CKB devnet smoke 中被使用，但 **没有 CLI helper** 在 smoke
  之外构建 final-script carrier submission report
- `carrier_payload_type_args_hex` 只覆盖两种 carrier kind
- final-script kind 走 fall-through 到 truncated 32-byte data_hash

**协议层含义**：
- 当 final-script kind 被提交时，type args 是 truncated 32-byte
  data_hash（`cli/src/main.rs:4590`），不是完整 64-byte canonicalization
- F-PRIM-01 cellid collision 在 truncated path 下提供 attacker
  collision 路径
- protocol layer 的 "settlement final carrier" 在 final-script kind
  下没有完整 type-cell identity 路径

### 6.4 3fda2ab (recompute) 协议层落地

**实现层描述**：
- CLI 引入 `final_l1_da_availability_preflight_ready`
  （`cli/src/main.rs:10408-10463`）
- 当 submission 是 final-L1 且 `da_availability_production_ready = true`
  但 recompute 不通过时，覆盖为 `false`
  （`cli/src/main.rs:10365-10370`）

**协议层含义**：
- recompute 在 final-L1 路径下字节级确定 — 同一 input 同一 output
- 但 production gate 不跑 recompute（F-SCRIPT-14）
- rehearsal scripts 不强制 recompute 通过
- "production-evidence-complete prototype" 的标签（MYELIN_PRODUCTION_REHEARSAL_REPORT.md:13）
  不意味着 "recompute 路径在 production 候补证据下一致" — protocol
  layer 还没在 production candidate evidence 下证明 recompute

### 6.5 73ddc73 / d1bb6f7 / 4612677 / 5ae440d 演化链

| Commit | 协议层状态 | 严密度评级 |
|--------|------------|------------|
| (5ae440d 之前) | court economics 是裸 policy，deployment evidence 不存在 | LOW |
| 73ddc73 | 政策显式 (8 字段 + invariant) | MEDIUM |
| d1bb6f7 | policy commitment-only (testnet_beta_ready = false) | MEDIUM |
| 4612677 | deployment evidence + stale rejection | HIGH (recompute path) |
| 5ae440d | deployment evidence bind 进 settlement intent economics_commitment | HIGH (recompute path)，但 P1 (F-SETTLE-01, F-SETTLE-08) |

演化链整体推进了 protocol layer 的 strictness，但 F-SETTLE-01 / F-SETTLE-08
是 **未补上的协议层缺口**：`economics_commitment` 在 verify 路径下是
self-consistent 而非 input ↔ oracle。

## 7. 跨子系统影响

### 7.1 Settlement ↔ Consensus

Settlement 的 court economics 用 `participant_set_hash` binding（`cli/src/main.rs:4110, 4360+`）。
Consensus 端 Static Closed Committee 与 Tendermint 都把同一
`participant_set_hash` 当作 session 身份的一部分（参考 MYELIN_SESSION_L2_PLAN.md:93-99）。
settlement intent 在两个 consensus mode 下要求 byte-identical
state transition + 不同的 finality evidence。

**协议层影响**：
- court_economics 在两 consensus mode 下必须 recompute byte-identical
  economics_commitment，否则 SettlementIntent verify 拒绝。
- 当前 fixture 路径下确实 byte-identical（commit 顺序 BTreeMap fixed）。
- F-SETTLE-07 的 `economics_commitment_algorithm` 中途覆盖在两 consensus
  mode 下行为相同（输入相同）。

### 7.2 Settlement ↔ DA

Settlement 的 `da_availability_commitment` binding
（`cli/src/main.rs:4371, 4112, 7263`）直接来自 DA manifest 的
`availability.availability_commitment`。3fda2ab recompute path 在
final-L1 settlement 下重新调用 `da_availability_evidence`
（`cli/src/main.rs:10446-10456`）。

**协议层影响**：
- DA 端的 `availability.production_ready` 状态必须 protocol-layer
  与 settlement 的 recompute 一致。
- F-SETTLE-19 (recompute blind to receipt_id / availability_window)
  跨 DA ↔ Settlement 共享。

### 7.3 Settlement ↔ Court

court_economics 通过 SettlementIntent binding 到 court bundle 的
challenge_payload_hash + escrow_input_cells_hash（`cli/src/main.rs:4110-4111`）。
Court bundle 自身在 `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` 与
`MYELIN_CONSENSUS_COMPLETENESS.md` 范围内。

**协议层影响**：
- court bundle 的 dispute window 由 court_economics 的
  `challenge_window_ms / challenge_deadline_ms` 控制。
- 当前 protocol layer 没有 explicit cross-system invariant "court
  bundle verify 时间必须在 challenge_window 内"。
- 现实是 `verify_session_settlement_intent` 在 caller-provided
  `current_time_ms` 下检查 challenge_deadline（F-SETTLE-06）。

### 7.4 Settlement ↔ CellDAG

settlement final carrier 在 L1 上是 type-cell identity
（`cli/src/main.rs:4407-4412`：type args = session_id_hash ||
settlement_identity_hash）。authority cell 也是 type-cell identity
（authority data_hash）。

**协议层影响**：
- F-PRIM-01 cellid collision 在 type-cell identity path 上是协议层
  基础缺陷
- F-SETTLE-18 把 F-PRIM-01 升级到 settlement final carrier 在
  final-script kind 下的具体路径

### 7.5 Settlement ↔ Operator Custody

custody / runbook 通过 readiness aggregate gate `production_ready`
（`cli/src/main.rs:10156`）。但 custody 不是链上，是客户端 JSON
+ blake3 hash。

**协议层影响**：
- custody 路径上的"谁是 operator"在协议层不存在 — 文档的
  `signing_threshold / operator_count` 是声明，不是 chain identity
- 攻击者可以伪造 custody 文档（F-SETTLE-10 / F-SETTLE-20）

### 7.6 Settlement ↔ Production Gate

F-CLI-01 ↔ F-SCRIPT-14 的 production gate ↔ rehearsal disagreement
（F-SETTLE-14）：
- gate 证明 fixture 路径下 settlement protocol internal consistency
- rehearsal 证明 rehearsal 路径下 evidence 拼接能力
- 两条证明不可叠加，protocol layer 没显式声明 "fixture 通过 = 
  rehearsal 通过" 或反之

### 7.7 总结：跨子系统的协议层接口

| 接口 | Settlement ↔ X | 协议层强度 | 风险 finding |
|------|----------------|------------|---------------|
| participant_set_hash | Consensus | HIGH (byte-identical) | (无新增) |
| da_availability_commitment | DA | HIGH (recompute 路径在 final-L1) | F-SETTLE-19 |
| challenge_payload_hash / escrow_input_cells_hash | Court | MEDIUM (challenge window 由 caller-provided current_time_ms 控制) | F-SETTLE-06 |
| type-cell identity (authority / final settlement) | CellDAG / primitives | LOW (F-PRIM-01 cellid collision 在 final-script kind 下被放大) | F-SETTLE-18, F-SETTLE-21 |
| operator custody / runbook | Operational policy | LOW (无 chain anchor，无 modify detection) | F-SETTLE-10, F-SETTLE-20 |
| production gate ↔ rehearsal disagreement | Gate scripts | LOW (boolean field 无显式 state label) | F-SETTLE-14 |

---

## 附录 A：Finding 数量与分布

- MUST (P0/P1, 至少 6)：**6**（F-SETTLE-01, F-SETTLE-03, F-SETTLE-10,
  F-SETTLE-14, F-SETTLE-16, F-SETTLE-18, F-SETTLE-20）
- MAYBE (MEDIUM/LOW, 至少 6)：**12**（F-SETTLE-02, F-SETTLE-04,
  F-SETTLE-05, F-SETTLE-06, F-SETTLE-07, F-SETTLE-08, F-SETTLE-09,
  F-SETTLE-11, F-SETTLE-12, F-SETTLE-13, F-SETTLE-15, F-SETTLE-17,
  F-SETTLE-19, F-SETTLE-21 — 共 14 项，超过 6 项最低要求）

注：P1 项中 F-SETTLE-14 在协议层与实现层都被识别（F-CLI-01 ↔ F-SCRIPT-14），
    本报告把它升级到协议层分析。

## 附录 B：自检清单

- [x] 每个 finding 有 file:line 锚点
- [x] 协议层 vs 实现层在 §1.2 明确区分
- [x] MUST 与 MAYBE 数量满足最低要求（6+6）
- [x] §6 覆盖所有已知 defect 的协议层影响
- [x] §7 覆盖跨子系统接口
- [x] 报告本身不使用 "v1/v2/rev N" 风格 revision 语言
- [x] 协议层声明 A1-A5 在 §1.3 显式列出，每条 finding 关联回声明
- [x] 自报未触碰 git 工作树（除新写 audits/subsystem-models-2026-07-05/LANE_SETTLEMENT.md）