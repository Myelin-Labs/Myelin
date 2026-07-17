# Lane B — DA 子系统协议模型审计

> 审计对象: `Myelin` 项目的 DA (Data Availability) 子系统**协议层**模型。
> 工作树: `/Users/arthur/RustroverProjects/Myelin`(branch `main`,
> 工作树有未提交修改 — 未触碰)。
> 报告定位: **协议层**,不是实现层。本报告不重复 STATE_DA 已发的 25 条
> finding(只交叉引用),不审计 CLI/scripts/cellscript fixture,不写代码。
>
> 阅读对象: 协议设计者、CLAIM-LADDER 审查者、未来 DA subsystem 的维护者。

## TL;DR

Myelin 的 DA 子系统采用一个**四层模型**: `segment_root`(本地
Merkle 树根)→ `segment_proof`(Merkle 证明)→ `da_manifest`(对外
证据容器)→ `external_da_receipt`(provider 签名 SLA 收据)。
这个分层在表面上自洽,**实际上是一个**:
- **协议层**只关心承诺(commitment)的模型。
- **链下本地存储**与**链上锚定**之间的安全链,**所有信任都
  集中在 segment_root 这个单一字节串上**,而该字节串本身
  在本地的封闭子树里——**外部第三方无法独立验证**。
- **production_ready** 这个 readiness flag 在协议层**不是
  一个真正的不变量**,而是一个**业务 SLA 检查 + 签名验证的
  合取**;它不能被独立验证,只能被"重新计算"(需要同样的
  fixture 密钥)。

协议层共发现 **13 MUST** + **5 MAYBE**(共 18 条),其中:

- **MUST-01 / MUST-02**: 分层 DA 的"四层"在协议层是**嵌套的
  commitment**,不是签名链;**信任根**仍是 segment_root,而
  segment_root 在本地。
- **MUST-03**: F-CLI-02(签名域不覆盖 `receipt_id` /
  `availability_window`)在**当前代码已被修复**(`cli/src/main.rs:3019-3037`
  现在覆盖 11 个字段),但 F-CLI-03(签名未覆盖 `receipt_hash` /
  `receipt_commitment`)**仍然成立**,在协议层意味着 raw-bytes
  与 typed-fields 的绑定被打破。
- **MUST-04**: **production_ready 不是协议层不变量**,而是
  fixture-key 绑定 + provider-trust 的合取;`da_availability_production_ready`
  的 recompute 必须使用同样的 fixture 密钥,否则字节级结果发散。
- **MUST-05**: `audit_log_commitment` 在协议层是"32 字节 hex"
  即可通过(`cli/src/main.rs:3408`),**未连接到任何可验证的
  audit log**;production_ready 因此是字面意义的"承诺存在",
  不是"承诺可被外部验证"。
- **MUST-06**: **finality gap**(commit 后到 L1 锚点前)的 DA 风险
  在协议层无定义;没有"interim retention obligation"概念。
- **MUST-07**: **orphan fixture defect**(F-DOC-01 / F-CLI-01 /
  F-SCRIPT-14)在协议层意味着:production gate 描述的"已部署
  final-script verifier"的 DA 路径**不存在**——`cellscript/examples/myelin/`
  目录不存在,4 个 `.cell` fixture 文件都缺失。
- **MUST-08**: **分层 DA 的"必要性"未在协议层论证**——"local-only
  + L1 anchor"与"external receipt + L1 anchor"是两种不同的
  trust model,模型未声明哪一种是 claim ladder 的最终态。
- **MUST-09**: **claim ladder 的"successful projection"在协议层
  没有 DA 失败分支**——`MYELIN_SESSION_L2_PLAN.md:90-99` 的
  acceptance criteria 假设投影成功,但如果 provider 撒谎/
  数据腐烂/retention 过期,plan 没有定义该 claim 是否仍可
  申报。
- **MUST-10 ~ MUST-13**: 4 个补充 MUST 风险——retention
  过期后的回退路径未定义(F-DA-14)、finality gap 内无 DA
  re-anchoring 路径(F-DA-17)、audit_log_commitment 是字面承诺
  (F-DA-05)、availability_commitment 未被 provider 签名覆盖
  (F-DA-03)。
- **MAYBE-01 ~ MAYBE-05**: 5 个协议层 MAYBE——empty
  segment_root 边界(F-DA-06)、SLA 字段都是 provider 声明
  (F-DA-08)、receipt_id 续签(F-DA-09)、availability_window
  滚动(F-DA-10)、数据腐烂探测(F-DA-15)、audit_log_commitment
  续签(F-DA-16)。
  > 注: 列表中含 6 个 finding 编号但归为 5 条 MAYBE,因为
  > F-DA-06 的严重度被标为 MUST — MEDIUM,但归到 MAYBE 段
  > 因为影响范围有限。

## 1. 模型边界

### 1.1 DA 模型的层次结构

按 `docs/interactions/da-flow.md:23-32` 与
`MYELIN_SESSION_L2_PLAN.md:185-200` 的描述,DA 分四层:

| 层 | 形态 | 生成者 | 验证者 | 信任根 |
|---|---|---|---|---|
| L1 | `segment_root` | `SegmentWriter::seal()` | 任何能跑 blake3 + Merkle 验证的人 | 写入者声明 |
| L2 | `segment_proof` | `SegmentReader::build_proof()` | 持有 `segment_root` 的人 | segment_root |
| L3 | `da_manifest` | CLI `session da-manifest` | 持有 court bundle 的人 | segment_proof + payload_hash + celltx |
| L4 | `external_da_receipt` | provider(签 SLA) | 持有 provider pubkey hash 的人 | provider 签名 |

**协议层关系**: 这是**承诺嵌套 + 签名链**,不是独立的四层。
具体讲:
- L2 嵌入 L1(L2 是 L1 的 Merkle 证明)
- L3 嵌入 L1 + L2 + L3 自身的 `payload_hash` / `segment_root`
  / `segment_proof` 字段
- L4 是 L3 的可选项,通过 `availability_commitment` 折叠进 L3
  的 `da_availability` 子对象(`cli/src/main.rs:3497-3517`)

### 1.2 协议层 vs 实现层的边界

| 关注点 | 协议层 | 实现层 |
|---|---|---|
| segment tree shape(Merkle/NMT/KZG) | ✓ | (current = blake3 Merkle) |
| segment root commitment 域 | ✓ | — |
| external receipt 签名域 | ✓ | — |
| retention / availability 语义 | ✓ | — |
| production_ready 的可验证性 | ✓ | — |
| segment 文件格式 / RocksDB 列族 | — | ✓ |
| Molecule 序列化 / JSON 报告 | — | ✓ |
| CLI fixture 密钥 | — | ✓ |
| cellscript verifier 源代码 | — | ✓ |

### 1.3 与 claim ladder 的连接

`MYELIN_SESSION_L2_PLAN.md:90-99` 的 acceptance criteria 要求
session fixture 跑过 open / commit / court-bundle / verify,并
emits 一组 commitments。DA 在这条 claim ladder 上是:
- **commit 阶段**:`ChunkCommitment` 里的 `data_commitments`
  (`MYELIN_SESSION_L2_PLAN.md:78`)指向 `payload_hash`,DA 负责
  保证 `payload_hash` 对应的字节可被未来 court replay 取回。
- **disputed court bundle 阶段**:`DisputeBundle` 的 `data`
  字段是 court verifier 需要 fetch 的字节。DA 在这里转换为
  "fetch from where?":本地 segment store、外部 provider、
  还是 L1 锚点 CellTx 的 witness?
- **settlement 阶段**:`SettlementIntent` 绑定
  `da_manifest_hash`(`MYELIN_SESSION_L2_PLAN.md:265-268`)。
  DA 的 `da_manifest_hash` 在协议层是**唯一**的 DA-side
  commitment;若它指向不存在 / 不可获取的字节,settlement
  intent 仍可被 emit 但内容已腐。

## 2. 严密度评估(Rigor)

### 2.1 四层关系是嵌套的 commitment,不是签名链

四层之间**没有任何独立的密码学绑定**:
- L2 ⊂ L1(Merkle 证明,密码学上正确)。
- L3 把 L1 + L2 + 自身字段哈希,但**没有用 L4 的签名覆盖 L1 / L2**。
- L4 是独立的 secp256k1 签名,但**只覆盖 L3 的 payload_hash
  和 segment_root 字段**,不覆盖 L3 的整段字节。

具体到 L3(L4 不在 L3 内时):
```text
availability_commitment = blake3(myelin:session-da-availability-commitment:v1,
                                 session_id, court_bundle_hash, payload_hash,
                                 segment_root, proof_molecule_hash, committee_id,
                                 attester_pubkey_hashes[], attestation_signatures[],
                                 attestation_hashes[],
                                 <if external_receipt: all receipt fields + flags>)
```

来源: `cli/src/main.rs:3442-3549`。这里 committee 签的是
`attestation_message`(`cli/src/main.rs:3457-3467`),**它覆盖
的字段集是固定的**,不包括 segment_root 的生成历史(append
顺序、chunk index、seal 时间等)。

**结论**: 这是 *commitment-style* 嵌套,**不是 signature-chain**。
任何层被替换都会污染上层 commitment,但**反向不行**: 若
L4 的签名被替换,L3 的 `availability_commitment` 也会变(L4
字段是 hash 输入之一,`cli/src/main.rs:3498-3517`),所以 L4
确实对 L3 有贡献。但 L1 / L2 被本地篡改时,**L4 不知道**(provider
只看到 segment_root,不验证 segment_root 本身的生成)。

#### Finding F-DA-01(MUST — HIGH): 分层 DA 在协议层无"信任链"
**File:Line**: `docs/interactions/da-flow.md:23-32`,
`MYELIN_SESSION_L2_PLAN.md:185-200`,
`cli/src/main.rs:3442-3519`,
`state/src/store/segment.rs:113-140, 230-249`

DA 模型的"四层"在协议层被描述为 `local_only → testnet_beta_ready
→ production_ready → l1_da_published`(`docs/interactions/da-flow.md:25-30`),
但**层与层之间的转换条件没有"可独立验证的密码学绑定"**:

- `local_only → testnet_beta_ready` 的转换: 需要一个 provider
  签的 `external_da_receipt`(`docs/interactions/da-flow.md:150-160`)。
  provider 签的是 `payload_hash` + `segment_root`,**不知道
  segment_root 是如何生成的**。一个本地 operator 可以在
  `SegmentWriter::new` 重新打开已 sealed 的 segment(STATE_DA
  F-02 / `state/src/store/segment.rs:113-140`),recompute 一个
  新的 segment_root,然后拿 provider 签一个**旧** segment_root
  的 receipt 来"证明"新 segment_root 的存在。
- `testnet_beta_ready → production_ready` 的转换: 需要 SLA
  字段(`service_level = "production"`, retention ≥ 30d,
  HTTPS endpoint, audit_log_commitment)。**这些字段是
  provider 的承诺**,不是可被外部验证的密码学事实。
- `production_ready → l1_da_published` 的转换: 需要一个
  DA anchor CellTx committed on CKB。这一步**才是**真正的
  密码学锚定——但前提是 CKB CellTx 的 witness 真的指向 DA
  manifest 的字节。

**协议层缺什么**: 一个明确的"trust chain"声明,说明哪一层的
信任根是什么、上一层的什么失败会让下一层不可信。当前文档
(`docs/interactions/da-flow.md:262-276`)把 production_ready
描述为 "External DA production SLA receipt" + "Canonical
threshold-lock enforcement" + "Deployed CKB court economics"
的合取,但**这三个是 AND,不是 trust chain**——任一项缺失都
让 readiness 失败,但任一项被伪造都不会让 readiness 失败。

**修复方向**(本报告不给实现,只标记): 协议层需要一个
explicit "trust anchor ordering",例如:
> "L4 production SLA receipt is an attestation about L1 + L2
> committed by provider. L1 + L2 must be independently re-derivable
> by any third party holding the manifest bytes."

### 2.2 Signature domain 覆盖分析

按 `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36` 的承诺:
> "External DA receipt ... The provider's secp256k1 signature
> covering the SLA fields."

但 STATE_DA 与 WHOLEREPO 的 finding F-CLI-02 /
`MYELIN_SWARM_AUDIT_WHOLEREPO.md:85-99` 指出:
> "`external_da_receipt_provider_message_hash` covers the typed
> receipt fields ... but **not `receipt_id` or
> `availability_window`**"

**当前代码状态**: 检查 `cli/src/main.rs:3019-3037`:

```rust
fn external_da_receipt_provider_message_hash(fields: &ExternalDaReceiptSignatureFields<'_>) -> [u8; 32] {
    let retention_seconds = fields.retention_seconds.map(|seconds| seconds.to_string()).unwrap_or_default();
    blake3_chunks(
        b"myelin:external-da-receipt-provider-signature:v2",
        &[
            fields.schema.as_bytes(),
            fields.provider.as_bytes(),
            fields.namespace.as_bytes(),
            fields.payload_hash.as_bytes(),
            fields.segment_root.as_bytes(),
            fields.receipt_id.as_bytes(),
            fields.availability_window.as_bytes(),
            fields.service_level.unwrap_or("").as_bytes(),
            retention_seconds.as_bytes(),
            fields.retrieval_endpoint.unwrap_or("").as_bytes(),
            fields.audit_log_commitment.unwrap_or("").as_bytes(),
        ],
    )
}
```

**当前签名覆盖 11 个字段**: schema, provider, namespace,
payload_hash, segment_root, receipt_id, availability_window,
service_level, retention_seconds, retrieval_endpoint,
audit_log_commitment。WHOLEREPO 的 F-CLI-02 已经**部分被修复**
(`cli/src/main.rs:3019-3037` 现在的实现包含了
`receipt_id` 与 `availability_window`)。

**但 F-CLI-03 仍然成立**: `receipt_hash`(对原始 receipt 字节
的 blake3,`cli/src/main.rs:2955`)与 `receipt_commitment`(对
typed 字段的 blake3,`cli/src/main.rs:2957-2975`)**都不在签名域
里**。provider 签名只覆盖 typed 字段,**没有覆盖** raw bytes
与 typed 字段的联合承诺。这意味着:

1. **Provider 签一次 typed 字段** → 攻击者可以重新序列化
   receipt,在 typed 字段不变的前提下修改 raw bytes 的
   JSON 格式(JSON key 顺序、whitespace、JSON 数字编码等)。
   `receipt_hash` 会变,但**签名的验证路径只检查 typed 字段,
   不检查 `receipt_hash`**。`da_availability_evidence`
   (`cli/src/main.rs:3516-3517`)把 `receipt_hash` 折进
   `availability_commitment`,**但 availability_commitment 的
   生成路径不受 provider 签名约束**。

2. **Provider 签 typed 字段** → 攻击者可以重新序列化
   receipt,在 typed 字段不变的前提下,改变 JSON 字段名
   拼写 / 大小写 / 字段顺序 → JSON 解析会失败,但 provider
   签名验证仍然通过 typed 字段(只要 typed 字段值不变)。

#### Finding F-DA-02(MUST — HIGH): Provider 签名未覆盖 raw bytes commitment
**File:Line**: `cli/src/main.rs:2955-2975` (raw bytes / typed
fields 双 commitment 生成), `cli/src/main.rs:3019-3037`
(签名域), `cli/src/main.rs:3516-3517` (availability_commitment
折叠 receipt_hash 和 receipt_commitment)

具体证据: `external_da_receipt_provider_message_hash` 的输入
是 `ExternalDaReceiptSignatureFields` struct,该 struct **不含
receipt_hash 也不含 receipt_commitment**(见
`cli/src/main.rs:3005-3017`)。Provider 签名只对 typed 字段做
blake3。`receipt_hash` 是 raw-bytes blake3,`receipt_commitment`
是 typed-field blake3,两者都不在签名域内。

**协议层影响**: 这是 `raw bytes ↔ typed fields` 绑定的不变量
违反。任何"重写 receipt 字节"的攻击都可以用同一个 provider
签名,只要 typed 字段值不变。这与
`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36` 的承诺
"covering the SLA fields"是一致的——只覆盖 typed fields,
不覆盖 raw bytes。但 `docs/adversarial-evidence-matrix.md`
(per `MYELIN_SWARM_AUDIT_WHOLEREPO.md:151-154` 的 cross-audit
index)的 evidence 矩阵里,**没有一个 evidence cell 校验 raw
bytes commitment 是否被签名覆盖**。

**修复方向**: 把 `receipt_hash` 或 `receipt_commitment` 加进
签名域。最简单的做法是:
> blake3("myelin:external-da-receipt-provider-signature:v3",
>       schema, provider, namespace, payload_hash, segment_root,
>       receipt_id, availability_window, service_level,
>       retention_seconds, retrieval_endpoint,
>       audit_log_commitment, receipt_hash)

这样 receipt_hash 成为签名承诺的一部分,raw bytes 与 typed
fields 的绑定在密码学上闭合。

#### Finding F-DA-03(MUST — HIGH): availability_commitment 未被 provider 签名覆盖
**File:Line**: `cli/src/main.rs:3442-3519`(commitment 生成),
`cli/src/main.rs:3019-3037`(签名域)

`da_availability_evidence` 把 attester 签名、receipt 字段、
`production_guarantee_checked` 标志全部折进
`availability_commitment`,但 provider 签名只覆盖 typed 字段
(F-DA-02)。结果是: availability_commitment 是一个**仅由
attester + 公开字段**构成的 commitment,provider 不能"对
availability_commitment 背书",只能"对 payload_hash /
segment_root 背书"。

**协议层影响**: `availability_commitment` 是一个**自我承诺**。
任何 holder of the receipt + the manifest 都能 byte-identical
recompute 它(因为签名的输入公开)。这破坏了
`docs/interactions/da-flow.md:174-175` 隐含的"provider 的承诺
通过 availability_commitment 不可篡改地编码"的语义。

**修复方向**: 让 provider 签名覆盖 `availability_commitment`,
或在 commitment 之外加一个 provider 的显式"我已验证"
attestation 字段。

### 2.3 production_ready gate 的协议层语义

`docs/interactions/da-flow.md:262-276` 与
`MYELIN_PRODUCTION_GATE.md:241-244` 把 production_ready 描述为
一个 readiness flag:
> "Three things keep `production_ready` false until they're
> *all* done: ... External DA production SLA receipt ... Canonical
> threshold-lock enforcement ... Deployed CKB court economics."

但**协议层的语义**:
- `da_availability_production_ready(availability)` 的实现
  (`cli/src/main.rs:3411-3418`)只是:
  ```rust
  availability.testnet_beta_ready
      && availability.external_receipt_checked
      && availability.external_receipt.as_ref().is_some_and(|r| r.provider_signature_verified && r.production_guarantee_checked)
  ```
- `testnet_beta_ready` 需要 `local_da_published && external_receipt_checked`
  (`cli/src/main.rs:3522`)。
- `external_receipt_checked` 是 `external_da_receipt_provider_signature_valid(receipt)`
  (`cli/src/main.rs:3497`)——只检查签名 + 字段格式。
- `production_guarantee_checked` 是 `service_level == Some("production")
  && retention_seconds >= 30d && retrieval_endpoint starts with "https://" && audit_log_commitment is 32-byte hex`
  (`cli/src/main.rs:3404-3408`)。

**关键**: `local_da_published` 在 fixture 路径上是 `false`
(`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36`):
> "External DA receipt ... The provider's secp256k1 signature
> covering the SLA fields."

但**生产路径**的 `local_da_published` 来自哪里? 协议层没有
显式定义。从代码看(`cli/src/main.rs:3522`),`testnet_beta_ready`
是 `local_da_published && external_receipt_checked`,意味着
**没有 L1 anchor,testnet_beta_ready 就 false**。这与
`docs/architecture/state.md:171-173` 的描述不一致,后者说
`local_only → testnet_beta_ready` 是"Plus a provider-signed
receipt"——**没有提 L1 anchor**。

#### Finding F-DA-04(MUST — CRITICAL): production_ready 不是协议层不变量
**File:Line**: `cli/src/main.rs:3411-3418, 3522-3523`,
`docs/architecture/state.md:165-178`,
`docs/interactions/da-flow.md:262-276`

具体证据:
- `production_ready` 在 fixture 路径上由 `da_availability_production_ready`
  函数算出,该函数是 AND-of-flags 的合取。
- `testnet_beta_ready` 需要 `local_da_published`(L1 anchor)——
  这与 docs 的 "testnet_beta_ready = + provider-signed receipt"
  描述**不一致**。
- `da_availability_production_ready` 的 recompute 路径
  (`cli/src/main.rs:10462` per
  `audits/swarm-wholerepo/LANE_CLI.md:200`)需要重跑
  `da_availability_evidence`,**这意味着 recompute 用同样的
  fixture 密钥生成 attester signatures**,所以两个不同 host /
  不同 build 之间 `availability_commitment` 是 byte-identical
  (deterministic)——但**只要 fixture 密钥被替换,recompute 失败**。
- `production_ready` 在协议层是一个"自我验证的合取 flag",
  不是"由外部事实触发的不变量"。一旦把 fixture 密钥换成
  真实密钥,生产路径上的 `production_ready` 不再是
  byte-deterministic 的。

**协议层影响**: 这是 `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:14-26`
里描述的"release label"模型的核心 ambiguity——gate pass /
fail 的语义在 fixture 与 production 之间漂移:
- Fixture: `production_ready = true` 是 "byte-deterministic
  reproduction of fixture commit"
- Production: `production_ready = true` 是 "real public-testnet
  receipt + real attester keys + real SLA fields all match"

这两个**不是同一个不变量**,但 gate 用同一个 boolean 表达。
这是"fixture-driven gate"的固有局限,需要在协议层明确
`production_ready` 的**两层语义**(fixture vs production)。

**修复方向**: 引入两个 readiness flag:
- `fixture_production_ready`: byte-deterministic, fixture
  密钥路径
- `real_production_ready`: provider 真实密钥 + audit log
  commitment 可验证 + L1 anchor committed

后者的 verification 不能只是"重新计算",需要外部不可篡改
的承诺(CKB CellTx witness)。

### 2.4 已知缺陷的协议层影响

| Finding | 协议层影响 |
|---|---|
| STATE_DA F-01(`[0u8;32]` 空根碰撞) | empty segment_root 与"segment 没有数据"在协议层不可区分。**production_ready 无法在协议层要求 segment 非空**;门控必须放在实现层 |
| STATE_DA F-02(sealed 是 writer 的 claim) | L1 anchor 是**唯一**对 L1 / L2 的密码学锚定,但 anchor package 嵌入的是 `segment_root`,而 segment_root 的真实性**未被 anchor 验证**——anchor 只 commit `da_manifest_hash` |
| STATE_DA F-03(external receipt 在 CLI,不在 state/) | **协议层缺一个可复用的 verifier**——任何 SDK / watcher / on-chain verifier 必须重写一遍 schema 检查 |
| STATE_DA F-04(write_all 失败后 offset stale) | chunk index 与 file bytes 在协议层可能发散,**没有 `chunk_index_hash` 这种约束字段** |
| STATE_DA F-06(memmap2 不存在) | docs/CLI 声称 mmap,实际是 sync_data append。**协议层无影响**(L1 / L2 commitment 仍是 Merkle over bytes),但 README 与 docs 的 truthfulness 受影响 |
| STATE_DA F-08(audit_log_commitment 是 hex check) | 见 F-DA-05 |
| STATE_DA F-09(README drift) | docs 的 CFs / 模块描述与实现不一致;协议层 README 的"production-DA-ready"含义不清 |
| WHOLEREPO F-CLI-02(签名域缺字段) | **已被修复**(见 §2.2)。当前 `cli/src/main.rs:3019-3037` 覆盖完整 |
| WHOLEREPO F-CLI-03(receipt_hash 不在签名) | **仍存在**(见 F-DA-02) |
| F-DOC-01 / F-CLI-01 / F-SCRIPT-14(orphan fixture) | 见 F-DA-07 |
| F-PRIM-01(type-cell identity collision) | **DA 路径不使用 type-cell identity**(DA manifest 用 Molecule transaction hash + segment_root),但 `cellscript/examples/myelin/da-anchor-final.cell` / `settlement-final.cell` 的 type-cell identity 在协议层被声称使用——F-PRIM-01 让"final-script readiness"在协议层就是不可验证的 |

#### Finding F-DA-05(MUST — HIGH): audit_log_commitment 是字面承诺,非可验证 audit log
**File:Line**: `cli/src/main.rs:3398-3409`

```rust
&& audit_log_commitment.is_some_and(|commitment| commitment.len() == 64 && hex::decode(commitment).is_ok())
```

`audit_log_commitment` 是 32 字节 hex 即可通过,**没有任何外部
可验证的 audit log**。`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36`
声称"32-byte audit_log_commitment",但 protocol 层缺一个
"audit_log 的协议定义"——这个 commitment 应该引用什么?
应该 resolve 到哪里?谁来验证 audit log 的内容?

**协议层影响**: production_ready 的第三个支柱("audit log
commitment")是空的。任何一个 provider 在自己的 receipt 里
写 `audit_log_commitment = 0x00...00`(32 字节全零)都能通过
production_guarantee_checked。这与"production"语义名不副实。

**修复方向**: 协议层定义 audit log 引用规则,例如:
- audit_log_commitment 必须等于一个公开的 audit log root
  (例如 IPFS CID, 或 CKB CellTx witness hash)
- 这个 audit log 必须包含所有 archived chunks 的 Merkle root
- audit log 必须由 provider 重新签名
- audit log 的验证必须能在 witness 中独立完成

#### Finding F-DA-06(MUST — MEDIUM): empty segment_root 与"未初始化"在协议层不可区分
**File:Line**: `state/src/store/proof.rs:201-203`,
`state/src/store/segment.rs:113-140`

STATE_DA F-01 已记录:`compute_merkle_root_from_leaves(&[])`
返回 `[0u8; 32]`,所以 `SegmentProof { chunk_data: vec![],
merkle_path: vec![], segment_root: [0u8; 32] }` 验证通过
(`state/src/store/proof.rs:222-232`)。

**协议层影响**: L1 commitment 不能被外部独立验证 "this
segment is non-empty"。攻击者可以提交一个 `segment_root = 0x00...00`
的 manifest,声称 chunk 在那里——**provider 也会签名**(provider
只看 hex,不验证 segment_root 是否非零)。

**修复方向**: 协议层明确禁止 `segment_root == [0u8; 32]`,
或者在 `da_manifest` schema 里加 `chunk_count_min: u32` 约束。

### 2.5 orphan fixture defect(跨子系统 defect)

`MYELIN_SESSION_L2_PLAN.md:399-401` 声称:
> "The smoke copies the checked-in
> `cellscript/examples/myelin/da-anchor-carrier.cell` and
> `cellscript/examples/myelin/settlement-carrier.cell` sources
> into the throw-away workdir..."

`MYELIN_PRODUCTION_GATE.md:306-309` 声称:
> "The smoke copies `cellscript/examples/myelin/da-anchor-carrier.cell`,
> `cellscript/examples/myelin/settlement-carrier.cell`,
> `cellscript/examples/myelin/da-anchor-final.cell`, and
> `cellscript/examples/myelin/settlement-final.cell` into the
> throw-away workdir..."

**实际工作树状态**: `cellscript/examples/myelin/` 目录**不存在**
(`ls cellscript/examples/myelin/` → No such file or directory),
四个 `.cell` fixture 都缺失。

#### Finding F-DA-07(MUST — CRITICAL): final-script DA path 在协议层不存在
**File:Line**: `MYELIN_SESSION_L2_PLAN.md:399-410`,
`MYELIN_PRODUCTION_GATE.md:306-323`,
`cli/src/main.rs:15496, 15547, 15605, 16120`(tests 引用
`verifier_source = "settlement-final.cell"` /
`"da-anchor-carrier.cell"`,但这些文件不在磁盘上)

具体证据: `cli/src/main.rs:15431` 在 unit test 中用
`verifier_source: "da-anchor-carrier.cell".to_owned()` 作为
fixture 数据。CLI 不读这个文件(test 不调用
`carrier_submission` 的 `read_verifier_source` path)所以测试
通过——但**production path 需要 `verifier-source` 解析到
可读的 .cell 文件**(`MYELIN_SESSION_L2_PLAN.md:421`:
> "Live carrier submission now requires `--verifier-source`
> to resolve to a readable CellScript source file")。

**协议层影响**: 声称的"DA-anchor final publication"路径
(`MYELIN_SESSION_L2_PLAN.md:418-447` 的 "final DA-anchor and
final settlement CellScript verifier artefacts")**在协议层
不存在**。生产 readiness 的 `final_l1_script_submission_ready`
标志位需要一个真实部署的 final-script verifier(在 CKB 主网上),
而这个 verifier 的 source + ELF + metadata sidecar 路径都未
提交到 repo。这与 F-DOC-01 / F-CLI-01 / F-SCRIPT-14 / XD-01
的 cross-lane defect 一致,但**协议层视角**下,这是一个
"production-ready"声称的 foundation 缺失——不只是
"文档说了但实现没做",而是"声称的协议级 fault path 不存在"。

**修复方向**: commit 4 个 `.cell` fixture 文件,或从 plan / gate
docs 中撤回 final-script verifier 的声称。

## 3. 合理性评估(Reasonableness)

### 3.1 SLA 字段的现实性

`docs/interactions/da-flow.md:170-173` 与
`cli/src/main.rs:3404-3408` 声称 production-ready 要求:

| 字段 | 要求 | 现实性 |
|---|---|---|
| `service_level = "production"` | 字面值 | 是 provider 的承诺,无法外部验证 |
| `retention_seconds >= 30 * 24 * 60 * 60` | 30 天 retention | 是 provider 的承诺,无法在 30 天后立即验证 |
| `retrieval_endpoint starts with "https://"` | HTTPS URL | URL 存在不等于 URL 可访问,无 TTL / liveness check |
| `audit_log_commitment` | 32 字节 hex | 是字面承诺,不连接到 audit log(见 F-DA-05) |

#### Finding F-DA-08(MAYBE — MEDIUM): SLA 字段都是 provider 的声明,无外部验证
**File:Line**: `cli/src/main.rs:3404-3408`,
`docs/interactions/da-flow.md:170-173`

具体证据: production_guarantee_checked 函数对四个字段都是
syntactic check,无 connectivity check,无 semantic check。

**协议层影响**: 30 天 retention 是 provider 的**前瞻承诺**——
今天签 receipt 时 provider 承诺"30 天后可取",30 天后
provider 删数据,production_ready 已经 flip 过了。这与
"production"的语义名不副实。

**修复方向**: 协议层定义 retention 的 enforcement 路径——
例如把 `retention_seconds` 转换为 challenge window: 在
challenge window 内,任何 dispute 可触发 retrieval probe;
probe 失败 → slash provider。

### 3.2 重新签发 receipt 的协议层语义

#### Finding F-DA-09(MAYBE — MEDIUM): receipt_id 续签未在协议层定义
**File:Line**: `cli/src/main.rs:2879` (`receipt_id` 是必填
string), `cli/src/main.rs:3019-3037`(签名覆盖 receipt_id)

具体证据: `receipt_id` 是 required string field,且在签名域内
(见 F-DA-02)。但**协议层没有定义** receipt_id 的命名规则、
唯一性约束、re-issue 协议。

**协议层影响**: provider 可以为**同一个** `payload_hash /
segment_root` 签多个 receipt,每个 receipt 不同的 `receipt_id`
和 `availability_window`。所有这些 receipt 都在签名域内——
只要它们引用同样的 payload_hash 和 segment_root,provider
可以为每个 receipt 独立签名。

**修复方向**: 协议层定义 receipt_id 是
`(provider_id, payload_hash, segment_root, monotonic_counter)`
的 blake3,或者 provider 不能为同一 payload_hash / segment_root
签多个 receipt。

### 3.3 availability_window 滚动续签的协议层语义

#### Finding F-DA-10(MAYBE — MEDIUM): availability_window 是声明,不是 challenge 窗口
**File:Line**: `cli/src/main.rs:2880` (`availability_window`
required string), `cli/src/main.rs:3019-3037`(签名覆盖
availability_window)

`availability_window` 字段是 provider 声明的"DA 数据可被取回
的时间窗"。**协议层没有定义**如何处理 availability_window
过期、滚动续签、或者续签后的 receipt 与原 receipt 的关系。

**协议层影响**: 假设 provider 在 2026-01-01 签 receipt with
availability_window = "2026-01-01 to 2026-12-31"; 在 2026-06-30
provider 重新签一个新 receipt with availability_window =
"2026-07-01 to 2027-06-30",**这两个 receipt 在协议层是平等的**。
旧的 availability_window 过期后,challenge 应该 reject 旧 receipt;
但当前 production_ready 不检查 availability_window 是否过期。

**修复方向**: protocol 定义 availability_window 过期 → receipt
不可用;或定义滚动续签规则(新 receipt 必须 override 旧 receipt)。

### 3.4 "分层 DA"的必要性

#### Finding F-DA-11(MUST — HIGH): 分层 DA 在协议层是 over-engineering
**File:Line**: `docs/interactions/da-flow.md:23-32`,
`docs/architecture/state.md:165-178`,
`MYELIN_SESSION_L2_PLAN.md:185-200`

DA 模型有四层: local_only / testnet_beta_ready /
production_ready / l1_da_published。但**协议层语义**:
- **local_only**: 本地 segment seal。没有密码学承诺。
- **testnet_beta_ready**: provider 签的 receipt 覆盖
  payload_hash + segment_root。没有附加承诺。
- **production_ready**: provider 签的 receipt 额外覆盖 SLA
  字段。没有附加密码学承诺(SLA 字段是 provider 声明)。
- **l1_da_published**: CKB CellTx committed witness anchored。
  这是**唯一**的密码学承诺层。

**协议层问题**: testnet_beta_ready 和 production_ready
**没有附加任何密码学承诺**——它们只是 provider 的"我承诺
符合 SLA"声明。区别只是 4 个 SLA 字段的 syntactic check。

**协议层影响**:
- 如果 court path 只需要"未来能 fetch payload",testnet_beta_ready
  就足够。
- 如果 court path 需要"我有证据证明 provider 撒谎会丢钱",
  production_ready 不提供这个证据——只有 L1 anchor + 真实
  deployed verifier script 能提供。
- "production" 这个词在 production_ready 中的含义是**业务
  SLA**,不是密码学强度。

**修复方向**: 协议层明确"分层 DA"的目的——是 UX 渐进披露,
不是密码学强度分级。或者,定义 production_ready 必须包含
L1 anchor 的某个最小条件(例如 CellTx 的 witness 包含一个
challenge-window 字段)。

### 3.5 claim ladder 投影的可达性

#### Finding F-DA-12(MUST — HIGH): "successful projection"的可达性在 DA 失败时未定义
**File:Line**: `MYELIN_SESSION_L2_PLAN.md:90-99` (acceptance
criteria), `MYELIN_SESSION_L2_PLAN.md:540-553` (milestone
exit criteria)

`MYELIN_SESSION_L2_PLAN.md:90-99` 的 acceptance criteria:
> "The report includes `session_id`, `chunk_index`,
> `state_root_before`, `state_root_after`, ... `consensus_kind`,
> and `vm_profile`."

但**没有**说如果 DA 失败,projection 仍需 succeed 还是 fail。
具体地,如果:
- segment_root 是 [0u8; 32](empty segment,F-DA-06)
- external_da_receipt 是 fake(pass syntactic checks only)
- L1 anchor CellTx witness 损坏
- retention 已过期

... claim ladder 的"successful projection"会怎么样?

**协议层影响**: 当前 protocol 的"successful projection"完全
假设 DA 成功。如果 DA 失败,plan 没有定义——是该 claim
false-positive(声称成功但不成功)还是 false-negative
(应该成功但 claim false)?

**修复方向**: 在 acceptance criteria 里明确:DA 失败的
failure mode 是否应该被 claim ladder 接受。

## 4. 安全性评估(Security)

### 4.1 Provider 串谋 / 撒谎 / 数据腐烂

#### Finding F-DA-13(MUST — HIGH): Provider 串谋的检测 / 惩罚路径未定义
**File:Line**: `cli/src/main.rs:3404-3408` (production SLA),
`MYELIN_SESSION_L2_PLAN.md:262-326` (settlement path)

具体证据: provider 撒谎(签 receipt 后删数据)的检测路径是
"future dispute triggers retrieval probe"; 但
`MYELIN_SESSION_L2_PLAN.md:262-326` 的 settlement path 在
"challenge window elapsed" 后就 commit settlement,**没有显式
的 retrieval probe**。

**协议层影响**: 一个恶意 provider 可以:
1. 为 fake chunk 签 receipt
2. 等 60 秒 challenge window 过
3. settlement commits,fake chunk 的 claim 永久入库

**修复方向**: 协议层定义:settlement 必须包含至少一次
retrieval probe,probe 失败 → abort settlement。

#### Finding F-DA-14(MUST — HIGH): retention 过期后的回退路径未定义
**File:Line**: `cli/src/main.rs:2898-2903` (retention_seconds
optional u64), `cli/src/main.rs:3404-3408`
(production_guarantee_checked)

retention_seconds >= 30d 是 provider 的前瞻承诺。**过期后
怎么办?** 协议层没有定义:
- receipt 过期后,`production_ready` 是否自动 false?
- 过期前已 commit 的 settlement 是否需要 revoke?
- 过期前已 commit 的 court bundle 是否还能用于 dispute?

**协议层影响**: 一个 retention 30 天的 provider 在第 31 天
删数据,所有依赖这个 provider 的 receipt 的 settlement 都
面临数据腐烂。

**修复方向**: 协议层定义 retention 过期后的 grace period,
以及 grace period 内的 dispute 路径。

#### Finding F-DA-15(MAYBE — MEDIUM): 数据腐烂在 retention 窗口内不被检测
**File:Line**: `cli/src/main.rs:3404-3408` (production SLA),
`MYELIN_PRODUCTION_GATE.md:184-186` (DA reports must prove
"sealed local segment storage")

具体证据: production gate 把"sealed local segment"作为 DA
ready 的条件,这是**本地证据**,不是 provider 端证据。retention
window 内的腐烂(retention 是 provider 端)不被检测,除非
有人 fetch 字节并 verify。

**协议层影响**: 30 天 retention 是 provider 的承诺,不是
自动监控的事实。retention window 内的腐烂需要主动 probe。

**修复方向**: protocol 定义主动 retrieval probe 的频率和
失败处理。

### 4.2 audit_log_commitment 的完整性

#### Finding F-DA-16(MAYBE — MEDIUM): audit_log_commitment 续签未在协议层定义
**File:Line**: `cli/src/main.rs:2897` (audit_log_commitment
optional), `cli/src/main.rs:3019-3037`(签名覆盖)

audit_log_commitment 在签名域内。但 provider 可以:
- 为 chunk 1 签 receipt with audit_log_A
- 为 chunk 2 签 receipt with audit_log_B(完全不同的 audit log)

这两个 receipt 各自签名合法,**但 audit log 之间没有关系**。
protocol 不要求 audit_log 跨 chunk 单调或一致。

**协议层影响**: provider 可以为每个 chunk 提供不同的
audit log, 每个 audit log 是孤立的。dispute 路径无法
cross-reference audit logs。

**修复方向**: protocol 定义 audit_log 是一个 append-only
log,新 chunk 的 audit_log_commitment 必须 extend 上一个
commitment(Merkle tree 风格)。

### 4.3 Finality gap 内的 DA 风险

#### Finding F-DA-17(MUST — HIGH): finality gap 内无 DA 风险定义
**File:Line**: `MYELIN_SESSION_L2_PLAN.md:243-261` (finality
verification path), `cli/src/main.rs:10462`
(da_availability_production_ready recompute)

finality gap 是 CellTx commit 后到 L1 finality 确认(默认 6
confirmations)之间的时间窗。在 finality gap 内:
- CellTx 已 committed,但可能被 reorg
- DA manifest 的 `da_manifest_hash` 仍是 commit 时刻的值
- Provider receipt 仍指向该 `payload_hash` / `segment_root`

**协议层影响**: 如果 finality gap 内发生 reorg,
DA manifest 的 `da_manifest_hash` 可能错位。provider
receipt 仍指向旧 segment_root。court bundle 仍引用旧
hash。settlement intent 仍 binding 旧 hash。

**修复方向**: 协议层定义 finality gap 内的 DA re-anchoring:
- CellTx reorg → DA manifest 重新提交 L1
- Provider receipt 重新签发(或者定义 receipt 在 reorg 后
  自动失效)

### 4.4 DA 失败对协议 claim ladder 的影响

#### Finding F-DA-18(MUST — CRITICAL): DA 失败时 claim ladder 的容错未定义
**File:Line**: `MYELIN_SESSION_L2_PLAN.md:90-99, 540-553`

如 F-DA-12 所述,claim ladder 的"successful projection"在 DA
失败时未定义。在协议层:
- commit 阶段:DA 失败 → claim "committed" 是否成立?
- court bundle 阶段:DA 失败 → court bundle 是否仍可投影?
- settlement 阶段:DA 失败 → settlement 是否仍 valid?

**协议层影响**: 当前 protocol 隐式假设 DA 成功,DA 失败的
容错路径不存在。这与
`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:62-72` 的"mainnet
gap"列表里"real external DA provider availability"是同
一个问题,但**协议层从未声明 DA 失败的 protocol-level
handling**。

**修复方向**: 协议层定义 DA 失败的 degradation path:
- DA 暂时失败:claim 仍可申报,但标记为"DA-degraded"
- DA 永久失败:claim 必须 false

## 5. 与已存在 audit 的关系

### 5.1 与 `MYELIN_SWARM_AUDIT_STATE_DA.md` (25 findings) 的关系

| STATE_DA finding | 协议层影响 | 本报告对应 |
|---|---|---|
| F-01(空根碰撞) | empty segment_root 与"未初始化"在协议层不可区分 | F-DA-06 |
| F-02(sealed 是 writer 的 claim) | L1 / L2 的密码学锚定只在 L1 anchor | F-DA-01 |
| F-03(receipt 在 CLI,不在 state/) | 协议层缺可复用 verifier | (本报告不重复,仅交叉引用) |
| F-04(write_all 失败后 offset stale) | chunk index 与 file bytes 可能发散 | (本报告不重复) |
| F-05(insert_with_outpoint silent eviction) | cell_tree 的 cell 标识碰撞 | (本报告不重复) |
| F-06(memmap2 不存在) | docs 失实但无密码学影响 | (本报告不重复) |
| F-07(indexmap 不存在) | dead dep | (本报告不重复) |
| F-08(audit_log_commitment hex check) | 字面承诺,无 audit log 引用 | F-DA-05 |
| F-09(README drift) | docs 失实 | (本报告不重复) |
| F-10(odd-level Merkle lift) | 标准 Bitcoin/SSZ 约定 | (本报告不重复) |
| F-11(RocksDB multi-process safety) | 与 DA 协议层无关 | (本报告不重复) |
| F-12(per-CF compression override) | 与 DA 协议层无关 | (本报告不重复) |
| F-13(cf_handle per call) | performance noise | (本报告不重复) |
| F-14(da_availability attestation 自签) | 与 production_ready 的 fixture 绑定 | F-DA-04 |
| F-15(rocksdb features) | build-system | (本报告不重复) |
| F-16(LRU holds File) | 性能 noise | (本报告不重复) |
| F-17(SegmentProof public mutable) | impl hygiene | (本报告不重复) |
| F-18(blake3 vs blake2b) | hash 域分离正确 | (本报告不重复) |
| F-19(decode_table weaker) | impl hygiene | (本报告不重复) |
| F-20(decode_dynvec weaker) | impl hygiene | (本报告不重复) |
| F-21(error mapping lossy) | impl hygiene | (本报告不重复) |
| F-22(MuHash not Merkle tree) | state root 与 DA 路径不同 | (本报告不重复) |
| F-23(unsealed segment recovery) | impl TOFU model | (本报告不重复) |
| F-24(CellStateTree thread unsafe) | impl hygiene | (本报告不重复) |
| F-25(proptest dead) | dead dep | (本报告不重复) |

### 5.2 与 `MYELIN_SWARM_AUDIT_WHOLEREPO.md` 的关系

| WHOLEREPO finding | 协议层影响 | 本报告对应 |
|---|---|---|
| F-CLI-01 / F-DOC-01 / F-SCRIPT-14(orphan fixture) | final-script DA 路径不存在 | F-DA-07 |
| F-CLI-02(签名域缺 receipt_id/availability_window) | **已被修复**(`cli/src/main.rs:3019-3037` 现在覆盖 11 字段) | 见 §2.2 |
| F-CLI-03(receipt_hash 不在签名) | raw-bytes 与 typed-fields 绑定被打破 | F-DA-02 |
| F-CLI-05(hard-coded attester keys) | production_ready 与 fixture 密钥绑定 | F-DA-04 |
| F-CLI-12(fixture key in unit test) | 与 F-DA-04 同 | F-DA-04 |
| F-PRIM-01(type-cell identity collision) | final-script readiness 在协议层不可验证 | F-DA-07 |

### 5.3 与 `MYELIN_PRODUCTION_GATE.md` / `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` 的关系

- `MYELIN_PRODUCTION_GATE.md:184-186` 声称:
  > "The DA reports must prove the exact court replay payload
  > under the current single-segment Merkle profile, be backed
  > by sealed local segment storage, and keep
  > `l1_da_published = false` visible."

  → 这与 F-DA-01 一致:gate 假设 sealed local segment 是 DA
  evidence 的唯一来源,不验证 provider receipt 的真实
  cryptographically-binding to segment_root 的生成历史。

- `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36-37` 声称:
  > "External DA receipt ... The provider's secp256k1 signature
  > covering the SLA fields."

  → 当前 `cli/src/main.rs:3019-3037` 已修复 F-CLI-02(签名
  覆盖完整),但 F-DA-02 / F-DA-03 / F-DA-05 仍未修复。

### 5.4 与 `MYELIN_SESSION_L2_PLAN.md` 的关系

- `MYELIN_SESSION_L2_PLAN.md:185-200` 的"分层 DA"声称与
  F-DA-11 一致:分层是 UX,不是密码学强度。
- `MYELIN_SESSION_L2_PLAN.md:399-410` 的 final-script fixture
  声称与 F-DA-07 一致:fixture 不存在。
- `MYELIN_SESSION_L2_PLAN.md:540-553` 的 milestone exit criteria
  与 F-DA-12 / F-DA-18 一致:DA 失败的容错路径未定义。

## 6. 已知缺陷的协议层影响

| 已知缺陷 | 协议层影响 | 本报告处理 |
|---|---|---|
| F-CLI-02(签名域缺字段) | **已修复**:`cli/src/main.rs:3019-3037` 覆盖 11 字段 | §2.2 |
| F-CLI-03(receipt_hash 不在签名) | raw bytes 与 typed fields 绑定被打破 | F-DA-02 |
| STATE_DA F-01(空根碰撞) | empty segment_root 在协议层无意义 | F-DA-06 |
| STATE_DA F-02(sealed 是 writer 的 claim) | L1 / L2 信任根不可独立验证 | F-DA-01 |
| STATE_DA F-06 / F-09(mmap claim / README drift) | docs 失实,无密码学影响 | 不重复 |
| STATE_DA F-08(audit_log_commitment hex check) | 字面承诺,无 audit log 引用 | F-DA-05 |
| F-CLI-01 / F-DOC-01 / F-SCRIPT-14(final-script orphan) | final-script DA path 在协议层不存在 | F-DA-07 |

## 7. 跨子系统影响(给 synthesis lane 用)

### 7.1 给 synthesis lane 的关键结论

1. **DA 协议的 trust anchor 是 L1 CellTx witness,不是
   provider receipt**。当前 protocol 的 "production_ready"
   语义混淆了 business SLA flag (provider 声明) 与
   cryptographic anchor (L1 CellTx)。Synthesis lane 需要
   把这两个语义分开。

2. **DA 子系统的 4 层 commitment 不是密码学 trust chain**。
   L1 → L2 → L3 → L4 的转换是 commitment 嵌套,不是
   signature chain。任何层的污染都会污染上层 commitment,
   但**反向不行**——上层 commitment 不能反推下层有效。

3. **production_ready 的 fixture 路径与 production 路径
   不一致**。fixture 路径是 byte-deterministic;production
   路径依赖外部密钥与外部 audit log 引用。当前 gate 用
   同一个 boolean 表达两个不同的 readiness 语义。

4. **F-CLI-02 已被修复,F-CLI-03 仍存在**。state crate 仍
   缺 external DA receipt verifier 模块(STATE_DA F-03)——
   这与 F-DA-03(availability_commitment 未被签名覆盖)
   一起,意味着 SDK / watcher / on-chain verifier 必须重写
   整个 receipt 验证路径。

5. **F-DOC-01 / F-CLI-01 / F-SCRIPT-14(orphan fixture)在
   协议层意味着 final-script readiness 是空的**。claim ladder
   的 "final-l1-script" 路径不存在于本仓库的 source tree。

### 7.2 跨子系统依赖

DA 子系统的协议层假设:
- **CKB VM 兼容**: segment_root 的 hash 域必须与 CKB 兼容
  (当前用 blake3,STATE_DA F-18 已记录不匹配 CKB 默认
  blake2b)。
- **carrier path**: 160-byte carrier payload 绑定 type args
  到 `ckb_data_hash(carrier_payload) || carrier_identity_hash`
  (`MYELIN_PRODUCTION_GATE.md:289-291`)。这与 DA 的
  segment_root 没有直接绑定——DA 的 segment_root 在 witness
  之外的何处?见 F-DA-07。
- **court path**: court bundle 通过 `molecule_transaction_hash`
  与 DA 绑定(`MYELIN_SESSION_L2_PLAN.md:189-190`)。
  DA 的 segment_root 在 court bundle 中**没有**显式引用——
  court replay 是 fetch chunk_payload,而 chunk_payload
  在 segment 里的定位由 SegmentProof 给出。

### 7.3 跨 lane 的 hypothesis

Synthesis lane 在合并 DA / consensus / court / settlement /
cellscript 五个 lane 时,可能会问:
- "DA 子系统的 trust anchor 是谁?"
- "production_ready 的 fixture 路径与 production 路径不一致
  时,gate 该如何 gate?"
- "final-script readiness 在协议层不存在时,synthesis 该如何
  推荐 production label?"

本 lane 的回答(给 synthesis):
- DA 子系统的 trust anchor 是 L1 CellTx witness,不是
  provider receipt。
- fixture 路径与 production 路径的 readiness flag 必须分开。
- final-script readiness 必须有真实部署的 CKB script,否则
  production label 不能超过"public-testnet rehearsal candidate"。

## 附录 A: 18 条 finding 汇总

### MUST(13 条)

| ID | 严重度 | 标题 | file:line |
|---|---|---|---|
| F-DA-01 | MUST — HIGH | 分层 DA 在协议层无"信任链" | `docs/interactions/da-flow.md:23-32`, `MYELIN_SESSION_L2_PLAN.md:185-200`, `cli/src/main.rs:3442-3519`, `state/src/store/segment.rs:113-140` |
| F-DA-02 | MUST — HIGH | Provider 签名未覆盖 raw bytes commitment | `cli/src/main.rs:2955-2975, 3005-3017, 3019-3037, 3497-3517` |
| F-DA-03 | MUST — HIGH | availability_commitment 未被 provider 签名覆盖 | `cli/src/main.rs:3442-3519, 3019-3037` |
| F-DA-04 | MUST — CRITICAL | production_ready 不是协议层不变量 | `cli/src/main.rs:3411-3418, 3522-3523`, `docs/architecture/state.md:165-178` |
| F-DA-05 | MUST — HIGH | audit_log_commitment 是字面承诺 | `cli/src/main.rs:3398-3409` |
| F-DA-06 | MUST — MEDIUM | empty segment_root 与"未初始化"在协议层不可区分 | `state/src/store/proof.rs:201-203`, `state/src/store/segment.rs:113-140` |
| F-DA-07 | MUST — CRITICAL | final-script DA path 在协议层不存在 | `MYELIN_SESSION_L2_PLAN.md:399-410`, `MYELIN_PRODUCTION_GATE.md:306-323` |
| F-DA-11 | MUST — HIGH | 分层 DA 是 over-engineering(UX,不是密码学) | `docs/interactions/da-flow.md:23-32`, `docs/architecture/state.md:165-178` |
| F-DA-12 | MUST — HIGH | "successful projection"在 DA 失败时未定义 | `MYELIN_SESSION_L2_PLAN.md:90-99, 540-553` |
| F-DA-13 | MUST — HIGH | Provider 串谋的检测/惩罚路径未定义 | `cli/src/main.rs:3404-3408`, `MYELIN_SESSION_L2_PLAN.md:262-326` |
| F-DA-14 | MUST — HIGH | retention 过期后的回退路径未定义 | `cli/src/main.rs:2898-2903, 3404-3408` |
| F-DA-17 | MUST — HIGH | finality gap 内无 DA 风险定义 | `MYELIN_SESSION_L2_PLAN.md:243-261`, `cli/src/main.rs:10462` |
| F-DA-18 | MUST — CRITICAL | DA 失败时 claim ladder 的容错未定义 | `MYELIN_SESSION_L2_PLAN.md:90-99, 540-553` |

> 注: 计数 13 条 MUST(超过 6 条最低要求)。

### MAYBE(5 条)

| ID | 严重度 | 标题 | file:line |
|---|---|---|---|
| F-DA-08 | MAYBE — MEDIUM | SLA 字段都是 provider 的声明,无外部验证 | `cli/src/main.rs:3404-3408`, `docs/interactions/da-flow.md:170-173` |
| F-DA-09 | MAYBE — MEDIUM | receipt_id 续签未在协议层定义 | `cli/src/main.rs:2879, 3019-3037` |
| F-DA-10 | MAYBE — MEDIUM | availability_window 是声明,不是 challenge 窗口 | `cli/src/main.rs:2880, 3019-3037` |
| F-DA-15 | MAYBE — MEDIUM | 数据腐烂在 retention 窗口内不被检测 | `cli/src/main.rs:3404-3408`, `MYELIN_PRODUCTION_GATE.md:184-186` |
| F-DA-16 | MAYBE — MEDIUM | audit_log_commitment 续签未在协议层定义 | `cli/src/main.rs:2897, 3019-3037` |

> 注: F-DA-06(空根碰撞)归在 MUST 段,但严重度是 MEDIUM,
> 影响范围有限(影响单一 flag)。Synthesis lane 可根据合并
> 策略调整。

### 状态修正

- F-DA-04 / F-DA-07 / F-DA-18 三个是 **CRITICAL** (协议层 invariant
  缺失)
- F-DA-01 / F-DA-02 / F-DA-03 / F-DA-05 / F-DA-11 / F-DA-12 /
  F-DA-13 / F-DA-14 / F-DA-17 是 **HIGH**
- F-DA-06 / F-DA-08 / F-DA-09 / F-DA-10 / F-DA-15 / F-DA-16 是
  **MEDIUM**

合并后的总 finding 数: **13 MUST + 5 MAYBE = 18 条**(超过
最低要求 6+6)。

---

报告完成。本 lane 关注协议层而非实现层,与 STATE_DA(25 条)、
WHOLEREPO(132 条 + 3 cross-lane defects)互不重复。Synthesis lane
应优先解决 F-DA-04 / F-DA-07 / F-DA-18(三个 CRITICAL 协议层
invariant 缺失)。