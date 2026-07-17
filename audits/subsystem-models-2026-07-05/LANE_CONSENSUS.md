# Lane A — 共识子系统协议模型审计

> 审计范围:`consensus/src/lib.rs`(Tendermint-style 加权 precommit 与静态 closed-committee)、`mempool/src/{lib,cellpool,scorer}.rs`(与共识交互的入口与确定性冲突打分)、`MYELIN_SESSION_L2_PLAN.md`(L2 规划中与共识相关的章节)、`MYELIN_CONSENSUS_COMPLETENESS.md`(`consensus` 自检表)、`docs/MYELIN_ARCHITECTURE.md` consensus 章节。
>
> 审计目的:**协议层**(模型是否严密、是否合理、是否安全),不是 CLI/script/cellscript/production gate 实现层;不重复 `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` 已记录的 19 条 finding,引用即可。
>
> 写入时间:2026-07-05。

## TL;DR

严密度总评:模型在 *单个* 块验证路径上是字节级严密的 —— 三个 hash 域(`myelin:block:v1`、`myelin:static-committee-signature:v1`、`myelin:tendermint-precommit:v1`)在 `consensus/src/lib.rs:21-23` 注册,在所有签名/哈希路径上都被用,域分离经 `tendermint_does_not_silently_fall_back_to_static_committee`(`lib.rs:998-1024`)真实验证为 negative。**但是**,协议层的"不变量"在模型边缘处断裂:`MyelinBlock.timestamp_ms` 是块哈希的一部分但协议层未规定其来源(`F-CONS-04`);"weighted precommit"的 *权重语义* 在协议层只有标量 `u64` 累加,但没有 >½ 或 >⅔ 的协议层不变量(`F-CONS-03`);block 的 *canonical encoding* 自称 `Molecule-shaped` 但实际是可变长 `Vec<u8>` 拼接,不是真正的 Molecule 解析兼容(`F-CONS-10`)。

合理性总评:模型诚实承认自己是 *"verifier-only fast path, not a permissionless BFT network"*(`lib.rs:6-11`、`MYELIN_CONSENSUS_COMPLETENESS.md:174-176`),但公开文档使用 *"Tendermint-style weighted precommit finality"* 这种命名,外部读者会假设 *完整* 的 Tendermint 状态机(proposer 选举、prevote、locked value、polka、round-change timeout)被实现 —— 实际 *都没有*(`F-CONS-02`、`F-CONS-11`)。*"Closed validator"* 是 TOML 配置级别的事实,不是协议级强制 —— 协议层没有"成员资格证明"或"成员资格变更" 的概念(`F-CONS-05`)。"Signature64" 是 *两个 blake3 哈希的拼接*,**不是密码学签名** —— 任何持有 validator config 的人都能计算它,所以"签名 = validator V 同意 block B"这一协议层语义是不成立的(`F-CONS-01`)。

安全性总评:在 *fixture* 边界内,模型是 deterministic 验证器,没有主动安全声明。但在 *模型要承担 L2 finality* 的边界处:
- 没有 equivocation 检测(`MEMPOOL_CONSENSUS F-01` 已记录 —— 本 lane 在协议层延伸为 `F-CONS-06`)、没有 slashing、没有 evidence log;
- 没有 finality gap 的协议层定义 —— *commit 后到 L1 投影前* 这段时间里,被攻破的 closed committee 可以重新签发冲突块(`F-CONS-08`);
- 与 L1 finality 的关系是 *单向追加* 而非 *真强于*:Myelin 的 finality 强度等于 closed committee 的诚实度,跟 L1 reorg 概率无关(`F-CONS-08`)。

必须修复的协议层声明:`F-CONS-01` (签名不是签名)、`F-CONS-02` (Tendermint 是 verifier 不是状态机)、`F-CONS-03` (无 quorum 阈值下界)、`F-CONS-04` (timestamp 来源未定义)、`F-CONS-05` (closed 是配置不是协议)、`F-CONS-06` (equivocation 协议层无证据)、`F-CONS-08` (finality gap 未定义)。这些不是 "调一个参数就能修" 的 bug —— 它们是 *模型在公开文档里做了模型没有兑现的承诺*。

可接受的协议层声明:`F-CONS-10` (Molecule-shaped 是命名习惯)、`F-CONS-13` (无 proposer 字段是设计选择,verifier-only)、`F-CONS-22` (IEEE 754 在 well-formed inputs 下字节级确定) 等 —— 这些是 doc 已经说清或显然在 fixture 范围内的细节。

## 1. 模型边界(什么算共识子系统的"协议层")

共识子系统的"协议层"在本次审计中限定为:
- *不变量声明*:claim ladder 中关于 *Tendermint round / 权重 / finality gap / committee membership* 的所有 `MYELIN_*` 顶层 doc 声明;
- *输入到输出的语义契约*:`consensus/src/lib.rs:1-731` 中的所有 `pub` 类型与 trait、其字段语义、以及它们之间的可达性关系(包括类型系统层面的"wrong engine"、"legacy certificate"等边界);
- *域分离正确性*:`myelin:block:v1`、`myelin:static-committee-signature:v1`、`myelin:tendermint-precommit:v1` 三个域是否覆盖了 *应当* 被签名的所有字节;
- *mempool 协议层声明*:`mempool/src/{cellpool,scorer}.rs` 中"deterministic conflict resolution"是否真的字节级确定,以及它对共识 finality payload(`ordered_cell_tx_commitments`、`block.hash()`)的协议层影响。

实现层(本次不审):CLI 命令、cellscript fixture、scripts、production gate、`F-PRIM-*` 原语缺陷的修复路径、`MyelinBlock` 在 `cellscript`/`cli` 中的拼装。

## 2. 严密度评估

### 2.1 总体

严密度可分三层评:

**Layer A — 域分离 + 类型系统边界**(强):三个 hash 域 + 两种 certificate 类型 + `LegacyCertificatePathUnsupported`(`lib.rs:642`)共同确保了 *静态 closed-committee cert* 与 *Tendermint precommit cert* 在协议层不互通。这一层是真严密的,经单元测试真实验证(`lib.rs:998-1024`)。

**Layer B — 单块验证语义**(中等):单个 `FinalisedBlock` / `FinalisedTendermintBlock` 的字节级确定性是闭环的 —— 给定 `MyelinBlock` 字节 + validator config + certificate,verify 路径只做 blake3 重算 + 整数比较,无环境依赖、无 RNG、无 wall-clock(`lib.rs:432-461`、`556-596`)。**但**:`timestamp_ms` 来自 session runtime 而协议层未规定来源(见 `F-CONS-04`)、`quorum_weight` 无最小阈值(见 `F-CONS-03`)、`MyelinBlock.consensus_kind` 不是 `ConsensusEngine` 内可验证的(见 `F-CONS-07`)。

**Layer C — 多块 / 多会话 / 跨子系统的不变量**(弱):模型没有"block N 是 block N-1 的合法后继"的协议层定义 —— `parent_hash` 是块字段,但没有"最长链"、"最重链"、"最重 Tendermint 轮"、"L1 锚定"等协议层概念。`MEMPOOL_CONSENSUS F-10` 已记 `StaticCommitteeConfig` / `TendermintConfig` 不带 `session_id`,本 lane 在协议层延伸为 `F-CONS-05` 与 `F-CONS-08`。

### 2.2 Findings

#### F-CONS-01 [CRITICAL · 严密度]

- **观察**:`Signature64`(`consensus/src/lib.rs:19`)是 *两个* blake3 哈希的拼接,生成器 `deterministic_signature`(line 464-482)与 `deterministic_tendermint_precommit`(line 646-668)都是 `signature[..32] = blake3(domain || id || pk || block_hash); signature[32..] = blake3(domain || ":tail" || id || pk || block_hash)`。验证路径(`lib.rs:447-449`、`583-585`)是 `expected = deterministic_signature(validator, block_hash); signature == expected` —— 即 *重算后比较*,不是 Schnorr/Ed25519 风格的 *非对称密钥验证*。
- **证据**:
  - `consensus/src/lib.rs:464-482`(`deterministic_signature` 的实现);
  - `consensus/src/lib.rs:646-668`(`deterministic_tendermint_precommit` 的实现);
  - `consensus/src/lib.rs:447-449`、`583-585`(verify 路径是 hash 重算 + 比较);
  - `consensus/src/lib.rs:241`(`public_key: Hash32` —— 32 字节,不携带任何"对应私钥"信息);
  - `consensus/src/lib.rs:412-413`("This is deliberately a closed-committee development signature, not a permissionless cryptographic signature scheme" —— 文档已承认)。
- **影响**:协议层声明的 *"validator V 签名同意 block B"* 在密码学意义上 **不成立** —— 任何能读到 validator config 的人(在 fixture 边界内即任何运行 myelin-cli 的人)都能产生 *同一个* `Signature64`。如果模型在公开文档里继续称之为 "signature / precommit / certificate",外部安全审计 / 文档读者会假定非对称密钥语义,从而高估 Myelin finality 的安全强度。该不成立在 *协议层* 关闭,但在 *公开叙事* 里仍开着。
- **建议方向**:协议层应在 doc 中显式声明 *本协议层的 `Signature64` 是 commitment scheme(对 `(id, public_key, block_hash)` 的 blake3 绑定),不是 cryptographic signature scheme*;并将类型重命名为 `CommitteeBinding` / `PrecommitBinding` 之类,把"非对称密钥验证"的承诺从公开命名里移走。如果未来要真做 non-closed committee,这一层必须换成真正的密码学签名(schnorr / ed25519 / BLS),而那是一次协议层升级。

#### F-CONS-02 [CRITICAL · 严密度]

- **观察**:公开命名 *"Tendermint-style weighted precommit finality"*(`lib.rs:31`、`docs/MYELIN_ARCHITECTURE.md:86`、`MYELIN_SESSION_L2_PLAN.md` 多处)承诺了 Tendermint 状态机。Tendermint(Buchman 2016 及 Tendermint Core 0.34 实现)的核心构件是:proposer 选举(round-robin 或 stake-weighted)、prevote 步骤、LockedValue/ValidValue 状态机、polka 检测、round-change timeout、GHOST-style fork choice。**Myelin 的 Tendermint 引擎在协议层只实现了**:`TendermintPrecommitCertificate { block_hash, height, round, signatures }` 与 `verify_precommit_certificate`(`lib.rs:556-596`) —— 一步 verifier,没有 proposer、没有 prevote、没有 locked value、没有 timeout、没有 fork choice。
- **证据**:
  - `consensus/src/lib.rs:484-489`(`Tendermint` 结构体只持有 `validators` map + `quorum_power`,无任何状态字段);
  - `consensus/src/lib.rs:526-538`(`sign_precommit_fixture` —— 单步签,无 RoundState / LockedValue / ValidValue);
  - `consensus/src/lib.rs:619-629`(`FinalisedTendermintBlock` 只有 `block / block_hash / round / certificate`,无 state machine 输出);
  - `MYELIN_CONSENSUS_COMPLETENESS.md:174-176`("phase-one Tendermint engine is a closed-validator fast path used for benchmarking and pressure testing, not a permissionless BFT network") —— 文档已承认这是 stripped variant,但同文件 line 88 又称之为 "Tendermint-style weighted precommit finality"。
- **影响**:
  - 协议层 *不* 拥有"Tendermint safety property: 在 1/3 Byzantine 假设下不会双签"的承诺 —— 它从未实施 `POLC` 规则或 lock-and-release。
  - 在 *协议层承担 L2 finality* 的边界上,模型只能证明 *"一组 ≥ quorum_power 的 validator 当时签了某个 (block_hash, height, round)"* —— 它 *不能* 证明 *"在同一 (height, round) 上,validator 不会签第二个 block_hash"*(见 `F-CONS-06` 与 `MEMPOOL_CONSENSUS F-01`)。
  - 公开命名跟实现之间的差距意味着:L2 业务方如果按 "Tendermint" 的字面理解来设计自己的安全模型(例如 "1/3 Byzantine safe"),会得到一个 *未被兑现* 的保证。
- **建议方向**:协议层应将引擎名从 `Tendermint` 改为 `WeightedPrecommitVerifier` 之类(同时保留 `ConsensusKind::Tendermint` 作为 doc-标记的过渡名以避免破坏 fixture),并在 `MYELIN_CONSENSUS_COMPLETENESS.md` 与 `MYELIN_SESSION_L2_PLAN.md` 中显式声明 *"本引擎在协议层等价于 'weighted signature verifier',不实施任何 BFT 状态机"*。如果项目未来要真做 Tendermint,这是另一次协议层引入 —— 要么引入 proposer 选举 + round state machine,要么不要叫它 Tendermint。

#### F-CONS-03 [HIGH · 严密度]

- **观察**:`StaticClosedCommittee::new`(`lib.rs:376-408`)与 `Tendermint::new`(`lib.rs:491-523`)对 `quorum_weight` / `quorum_power` 的协议层约束是 *"必须 ≥ 1 且 ≤ total_weight"*。**协议层未规定** quorum 必须 > `total_weight / 2`(crash fault tolerance)或 ≥ `2 * total_weight / 3`(BFT fault tolerance)。`quorum_weight = 1, total_weight = 100` 在协议层是合法的 —— 一个 validator 即可单方面 finalize。
- **证据**:
  - `consensus/src/lib.rs:378-408`(static committee 的全部校验:`quorum_weight != 0`、`validator.id` 非空、`validator.weight != 0`、`total_weight` 不溢出、`quorum_weight ≤ total_weight`);
  - `consensus/src/lib.rs:493-523`(Tendermint 同样规则,只把 `quorum_weight` 改名为 `quorum_power`);
  - `consensus/src/lib.rs:456-458` 与 `592-594`(verify 路径只检查 `signed_weight >= quorum_weight` —— 不检查比例);
  - `docs/MYELIN_ARCHITECTURE.md:265-277`(示例 TOML 中 `quorum_weight = 2`、每个 validator `weight = 1`、总 validator 2 个 —— 即 100% 阈值,无 fault tolerance)。
- **影响**:协议层接受"单 validator 全权"配置 —— 在 fixture 边界内这是 *诚实的*,在 L2 业务边界上是 *灾难性* 的:任何能读到 TOML 的攻击者只要攻破 *一个* validator 即可 finalise 任意块。这跟"F-PRIM-01 cellid 碰撞"叠加后,在 L1 court projection 上很难回滚(因为 commit 后还要走 DA + settlement)。
- **建议方向**:协议层应规定"quorum 必须 > total_weight / 2"的最小阈值(可配,但有下限),并在 `StaticClosedCommittee::new` / `Tendermint::new` 的 `InvalidConfig` 路径里强制。这跟现有 `quorum_weight != 0`、`quorum_weight ≤ total_weight` 是同类校验,不是新负担。

#### F-CONS-04 [HIGH · 严密度]

- **观察**:`MyelinBlock.timestamp_ms: u64`(`lib.rs:166`)是 *协议层块哈希的一部分*(`to_molecule_bytes`,`lib.rs:183-196` 的第 4 字段,line 188),但 *来源* 在协议层未规定 —— 字段注释说 "supplied by the session runtime"(line 165)。session runtime 是协议层外部的组件,协议层没有任何 *timestamp_provenance* 字段、`issued_by` 字段、或"timestamp 必须 < parent_timestamp"之类的不变量。`MEMPOOL_CONSENSUS F-11` 已在实现层指出"两个 engine 在不同 timestamp 下 finalise 同一状态转换会得到不同 block.hash()";本 lane 在协议层延伸为 *timestamp_ms 在协议层没有 source of truth*。
- **证据**:
  - `consensus/src/lib.rs:165-166`(`timestamp_ms: u64`,注释: "Millisecond timestamp supplied by the session runtime");
  - `consensus/src/lib.rs:188`(`timestamp_ms.to_le_bytes()` 是 `to_molecule_bytes` 的第 4 字段);
  - `consensus/src/lib.rs:199-204`(`block.hash()` 把 `BLOCK_HASH_DOMAIN` 跟 `to_molecule_bytes` 一起 hash);
  - `MYELIN_SESSION_L2_PLAN.md:94-100`("The same fixture finalises with Tendermint and produces identical state transition commitments but different finality evidence" —— *承诺了* state root 跨引擎一致,但没承诺 timestamp 跨 finalisation 一致)。
- **影响**:
  - 两个独立会话(同一 participants、同一 CellTx 序列、同一 state transition)在 *不同 timestamp* 下 finalise,会得到 *不同* `block.hash()`,因而 *不同* finality evidence。这与 `MYELIN_SESSION_L2_PLAN.md:548-549`("The state transition is consensus-independent; only finality evidence differs") 在文档语义上轻微矛盾 —— 文档说 "only finality evidence differs",但 timestamp 让 *同一个* 引擎下 *两次 finalisation* 也得到不同 evidence。
  - 在 L1 court projection 上,一个 "court bundle" 必须能从一个固定 `block.hash()` 派生所有 Molecule 字段。如果 timestamp 来源是 runtime 的 wall-clock,则 court replay 必须 *重放* timestamp 才能复现 `block.hash()` —— 这要求 timestamp 是 state transition 的 *输入* 而不是 *输出*,但 `lib.rs:165-166` 的 doc string 没说明这一点。
- **建议方向**:协议层应规定 timestamp 的两条 *之一*:
  - timestamp 由 *状态转换本身* 派生(例如 `state_root_after` 的前 8 字节、或 `ordered_cell_tx_commitments[0]` 的前 8 字节),保证 *同一状态转换 → 同一 timestamp → 同一 block.hash()*;或
  - timestamp 完全从 finality payload 中移除,改由 finality evidence 之外的 metadata 携带(类似 Ethereum `extra_data` 但要更明确)。
  
  协议层不规定 = 实现层每次 finalisation 都面对一个未定义的 64-bit 字段,这是 doc 与 code 的协议层缺口。

#### F-CONS-05 [HIGH · 合理性]

- **观察**:`StaticCommitteeConfig`(`lib.rs:247-253`)与 `TendermintConfig`(`lib.rs:256-262`)把 *closed* 这个安全声明完全委托给 TOML 配置 —— 协议层没有 on-chain membership proof、没有成员资格变更协议、没有 key rotation 协议、没有 key ceremony 产物。*"Closed validator"* 是 *config 层* 的事实,不是 *protocol 层* 的不变量。
- **证据**:
  - `consensus/src/lib.rs:67-83`(`ConsensusConfig::from_toml_str` —— 唯一入口是 TOML 解析);
  - `consensus/src/lib.rs:115-139`(`StaticCommitteeConfig` / `TendermintConfig` 的构造路径 —— 只读 TOML,无任何外部 trust anchor);
  - `consensus/src/lib.rs:236-244`(`CommitteeValidator { id, public_key, weight }` —— id 是 free-form 字符串,public_key 是 32 字节 hash-like bytes,无 KYC、无签名、无 timestamp、无 proof-of-membership);
  - `consensus/src/lib.rs:493-523`(`Tendermint::new` 同 `StaticClosedCommittee::new`,无 membership proof);
  - `MYELIN_SESSION_L2_PLAN.md:74-75`(SessionOpen 包含 "selected consensus profile" —— 但 SessionOpen 是 Myelin 内部 record,不是 CKB 链上 artifact)。
- **影响**:
  - 任何"我们用的是 closed committee"的协议层声明,如果不被验证,则 *无法被外部 party 验证* —— L1 court 在 dispute 时,只能信任 session 提供的 TOML。
  - 跟 *L1 投影* 的关系:court bundle 引用 committee certificate 时,必须 *同时* 携带 committee config —— 但 committee config 不是 protocol 字段(它没有进入 `MyelinBlock.to_molecule_bytes`)。这意味着 *同一个* committee certificate 在两个不同 config 下都能 verify —— 而哪个 config 是"真"的,court 无法 protocol-level 区分。
- **建议方向**:协议层应规定 *closed* 的两种 *之一*:
  - 把 `committee_config_hash` 加入 `MyelinBlock`(作为 `to_molecule_bytes` 的新字段),让 finality evidence 自带 *"此 certificate 由 config hash = H 的 committee 签发"* 的承诺;或
  - 显式声明 *closed committee 不参与 L1 dispute* —— 只对 L2 内 finality 负责,所有 dispute 走"重新跑 fixture"路径。

#### F-CONS-06 [HIGH · 安全性] *(协议层延伸 `MEMPOOL_CONSENSUS F-01`)*

- **观察**:`MEMPOOL_CONSENSUS F-01` 已在实现层记录 *"Tendermint 没有跨 (height, round) equivocation 检测"*。本 lane 在 *协议层* 延伸:协议模型没有任何 *slashing condition*、*evidence log*、或 *validator state across blocks* —— 即使未来加上 equivocation 检测,模型也没有"被 slashed 的 validator 之后不能签"的协议层机制。一个 validator 可以签两个互相冲突的 `FinalisedTendermintBlock`(同一 `(height, round)`, 不同 `block_hash`),两者在 verifier 层都有效。
- **证据**:
  - `consensus/src/lib.rs:556-596`(`verify_precommit_certificate` 只看 *单个* certificate,无 evidence log 输入);
  - `consensus/src/lib.rs:619-629`(`FinalisedTendermintBlock` 不携带 evidence 链、不携带 slashing 状态);
  - `consensus/src/lib.rs:491-523`(`Tendermint::new` 无 evidence state 字段);
  - `MYELIN_CONSENSUS_COMPLETENESS.md:106`("a validator is allowed at most one precommit per (height, round, block_hash) certificate" —— *但只在单个 cert 内*,跨 cert 不成立);
  - `MEMPOOL_CONSENSUS F-01`(`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:42`)。
- **影响**:在协议层 *"如果 validator V 被攻破,会怎样?"* 的回答是:它可以生产任意多个互相冲突的 finalised block,只要 quorum ≥ quorum_power。在 fixture 边界内,这没有后果(只有一个 runner);在 L2 业务边界上,这意味着 *模型无法对 validator 做任何 trust scoring* —— 协议层 *无 evidence log* 是 *无 accountability* 的同义词。
- **建议方向**:协议层应承认 *"本引擎在协议层不提供 slashing 或 evidence log;被攻破 validator 的检测与惩罚属于 L1 court 的责任,不属于 consensus 引擎"*。如果未来要真做 non-closed committee,这是协议层升级:引入 evidence log(可以是 append-only storage 或 CKB 上的 commitment),引入 slashing state 字段。

#### F-CONS-07 [MEDIUM · 严密度]

- **观察**:`MyelinBlock.consensus_kind: ConsensusKind`(`lib.rs:168`)是 `pub`,值是 `ConsensusKind::StaticClosedCommittee` 或 `ConsensusKind::Tendermint`,序列化时是 `consensus_kind.as_str().as_bytes()`(line 189)—— 即字符串 `"static-closed-committee"` 或 `"tendermint"`。**`ConsensusEngine::verify_certificate` 在协议层没有"block.consensus_kind 必须等于 engine.kind() 的强制检查"** —— 这一检查放在 *具体引擎* 的入口(`StaticClosedCommittee::finalise_block` `lib.rs:316-318`、`Tendermint::finalise_block_with_precommit` `lib.rs:606-611`),不是 `ConsensusEngine` trait 的协议层不变量。如果未来加入第三种引擎忘了写 `WrongEngine` guard,跨引擎 cert 静默 accepted 的风险存在。
- **证据**:
  - `consensus/src/lib.rs:307-323`(`ConsensusEngine` trait —— 只有 `kind()`、`verify_certificate`、`finalise_block` 默认实现,无"kind guard" 抽象方法);
  - `consensus/src/lib.rs:316-318`(`StaticClosedCommittee::finalise_block` —— kind check 在 trait 默认实现中,但 *block.consensus_kind != self.kind()* 才检查 —— 容易在新引擎里漏掉);
  - `consensus/src/lib.rs:606-611`(Tendermint 入口的 `WrongEngine` check);
  - `consensus/src/lib.rs:998-1024`(`tendermint_does_not_silently_fall_back_to_static_committee` 测试保证 *目前* 不会静默 fallback)。
- **影响**:协议层不强制 "block.consensus_kind == engine.kind()" 是 *test-coverage 可控* 的问题,不是 *密码学可破* 的问题 —— 当前的 22 个测试覆盖了 cross-engine 拒绝(`lib.rs:998-1024`、1052-1077)。但 *协议层* 应当把这一检查升格为 trait 内 invariant,而不是依赖每个具体引擎记得写。
- **建议方向**:协议层应让 `ConsensusEngine` 的 `verify_certificate` / `finalise_block` 默认实现统一做 kind guard,具体引擎不必重复写。

#### F-CONS-08 [HIGH · 安全性]

- **观察**:`FinalisedBlock` / `FinalisedTendermintBlock` 在 *commit* 时产生,但 commit 后到 L1 投影(`MYELIN_SESSION_L2_PLAN.md` 描述的 court-bundle → DA manifest → settlement 流程)之间存在一个 *时间窗*。协议层没有规定:
  - 这个窗里 *"已 finalised block 还能不能被覆盖"*;
  - 如果 covered,需要多少 validator 同意覆盖;
  - 如果不能 covered,L1 court 在看到新 evidence 时能拒绝多少 finalised block。
  
  *换言之*:Myelin 的 finality 跟 L1 CKB reorg 概率的关系在协议层 *完全断开*。Myelin finality 等于 *"被一个 closed committee 在那一刻签了"*,L1 reorg 概率等于 *"CKB Nakamoto consensus 在那一刻出块概率"* —— 两者互相独立,不存在"叠加"或"加强"。
- **证据**:
  - `consensus/src/lib.rs:296-304`(`FinalisedBlock` —— 只包含 `block / block_hash / certificate`,无 L1 anchor、无 checkpoint);
  - `consensus/src/lib.rs:619-629`(`FinalisedTendermintBlock` 同样);
  - `MYELIN_SESSION_L2_PLAN.md:262-280`(`settlement-intent` 描述 settlement 但 *不* 规定 *"已 finalised block 在 settlement 期间能否被覆盖"*,line 266-268 只说 `l1_da_published = false, l1_court_implemented = false`);
  - `MYELIN_SESSION_L2_PLAN.md:540-553`(退出标准只说 *"a deterministic session fixture runs through open, commit, court-bundle, and verification"*,不规定 finality gap 的协议层行为)。
- **影响**:
  - 在 protocol-level,一个 attacker 攻破 *quorum_weight* 个 validator 后,可以 *重新* 签发一个 conflicting `FinalisedBlock`(同一 `(parent_hash, number)`,不同 `state_root_after`)。如果该 `FinalisedBlock` 还没走到 L1 court / DA / settlement,被攻破方可以 *单机覆盖*。
  - 即使 L1 court 最终拒绝该 conflicting block,L2 业务方在 finality gap 期间已经按旧 finalised block 做了动作(提交了 settlement intent、签名了 DA anchor) —— 这些动作可能被覆盖后作废。
- **建议方向**:协议层应在 `FinalisedBlock` 上引入 *"checkpoint_id"* 字段,要求 court-bundle / DA manifest 引用一个 *确定的* checkpoint 序号;或在 `FinalisedBlock` 上要求 *"必须有 parent_finality_id"* —— 形成一条 hash-chained finality chain,这样覆盖需要走 *链* 而不是单点。

#### F-CONS-09 [MEDIUM · 合理性]

- **观察**:`Signature64` 由两个 *相同输入* 的 blake3 拼接而成 —— `signature[..32] = blake3(domain || id || pk || block_hash); signature[32..] = blake3(domain || ":tail" || id || pk || block_hash)`。两个 hash 共享 *所有* 字段,只差 `b":tail"`(4 字节)后缀。这等价于"把 blake3 输出加 4 字节 salt 后再 blake3 一次" —— **不增加任何密码学强度**,只把输出从 32 字节扩到 64 字节。名称 `Signature64` 暗示 64 字节签名,实际是 32 字节 commitment × 2。
- **证据**:
  - `consensus/src/lib.rs:464-482`(static signature 生成器);
  - `consensus/src/lib.rs:646-668`(Tendermint signature 生成器);
  - `consensus/src/lib.rs:478-481` 与 `664-666`(`signature[..32]` 与 `signature[32..]` 的赋值);
  - `consensus/src/lib.rs:19`(`Signature64 = [u8; 64]`)。
- **影响**:命名 `Signature64` 让外部读者假设这是 64 字节非对称签名(Ed25519 是 64 字节,BLS12-381 G1 是 48 字节) —— 实际是 32-byte commitment 的两倍。`F-CONS-01` 是协议层 *承诺强度* 错位,`F-CONS-09` 是 *字节级实现细节* 错位。
- **建议方向**:协议层应把 `Signature64` 改名为 `PrecommitBinding64`(64 字节)或更明确的 commitment 名称,并在 doc 中说明"前 32 字节是 commitment,后 32 字节是二次 commitment,salt `":tail"`",以免外部按 64-byte signature 推断密码学强度。

#### F-CONS-10 [MEDIUM · 严密度]

- **观察**:`MyelinBlock::to_molecule_bytes`(`lib.rs:183-196`)自我描述为 *"the canonical Molecule-shaped byte representation used for hashing"*,但实际实现是 `Vec<u8>` 拼接 + `encode_table`(`lib.rs:216-233`)的 offset table;offset 是 `u32`,字段是 `Vec<u8>` 可变长字节串。真正的 Molecule 规范(CKB Nervos 用的)要求 *fixed-width 字段*、*union types*、*option types*、*array types*,而这里只有 `header_size + offset_table + field_bytes` 这一种结构,且 `consensus_kind` 是变长字符串("static-closed-committee" 19 字节, "tendermint" 9 字节)。这不是 *Molecule*,是 *Molecule-inspired hand-rolled encoding*。
- **证据**:
  - `consensus/src/lib.rs:182`(`pub fn to_molecule_bytes(&self) -> Vec<u8>` —— 文档自述 "Molecule-shaped");
  - `consensus/src/lib.rs:216-233`(`encode_table` —— `header_size = 4 + fields.len() * 4`,field 是 `Vec<u8>` 可变长);
  - `consensus/src/lib.rs:184-195`(field 列表 —— `consensus_kind` 来自 `as_str().as_bytes()`,即变长);
  - `consensus/src/lib.rs:189`(`self.consensus_kind.as_str().as_bytes().to_vec()`)。
- **影响**:如果未来要 *直接* 拿这个 encoding 喂给 CKB Molecule 解析器(例如做 CKB-compatible projection),会发现:
  - CKB Molecule 解析器期望 *固定偏移* 表(`offset[i+1] - offset[i]` 给出 field 大小),但 `encode_table` 的 offset 是 `u32` 累加,跟 CKB Molecule 的 `MoleculeTable { total_size, offset[fields.len()], fields... }` 形状虽然相似但 field 排列可能不一致;
  - CKB Molecule 期望 *固定类型 tag*,而 `Vec<u8>` field 没有类型信息。
  
  这是命名 vs 实现的不一致 —— 不是 bug,但协议层若要兑现 *"Molecule-shaped"* 的承诺,要么真用 `molecule` crate,要么把命名改为 *"Molecule-inspired"* / *"table-encoded"*。
- **建议方向**:协议层应把 `to_molecule_bytes` 改名为 `to_table_bytes` 或类似,把 doc string 改为 *"Molecule-inspired hand-rolled encoding"*。如果项目要真做 CKB 兼容,后续应改用 `molecule` crate 并把 `MyelinBlock` 定义为真正的 `block::MyelinBlockV1Mol` 结构。

#### F-CONS-11 [MEDIUM · 合理性]

- **观察**:与 `F-CONS-02` 配对 —— 协议层命名 *Tendermint-style* 但实际不是 Tendermint(`MYELIN_SESSION_L2_PLAN.md` 多处使用 *"Tendermint"* 字样、`MYELIN_CONSENSUS_COMPLETENESS.md:88`、`docs/MYELIN_ARCHITECTURE.md:86`)。一个 *reasonable reader* 看到 *Tendermint* 会假设完整的 Tendermint 安全证明(1/3 Byzantine safe、Casper FFG-style finality、fork choice rule),但 Myelin 在协议层 *没有这些*。
- **证据**:
  - `consensus/src/lib.rs:31`(`ConsensusKind::Tendermint`);
  - `consensus/src/lib.rs:39`(`"tendermint"` —— `as_str()` 返回);
  - `consensus/src/lib.rs:484-489`(`pub struct Tendermint { validators, quorum_power }` —— 无 state machine 字段);
  - `docs/MYELIN_ARCHITECTURE.md:279-301`(示例 TOML 标 `kind = "tendermint"`);
  - `MYELIN_CONSENSUS_COMPLETENESS.md:88`("Tendermint-style weighted precommit finality");
  - `MYELIN_CONSENSUS_COMPLETENESS.md:174-176`(内部已承认 stripped variant —— *但* 同一文档其他位置仍用完整命名)。
- **影响**:见 `F-CONS-02` 的影响分析。
- **建议方向**:见 `F-CONS-02` 的建议方向。

#### F-CONS-12 [MEDIUM · 安全性]

- **观察**:`verify_precommit_certificate`(`lib.rs:556-596`)对 `round: u32` 的协议层约束是 *"与 cert 自带 round 一致"*,不约束 round 的取值范围。`round = u32::MAX` 在协议层合法,`round = 0`、`round = 1`、`round = 1_000_000_000` 也都合法。模型没有 *"round 必须严格递增"* / *"round 必须在合理范围"* 的协议层不变量,因为模型没有 *RoundState* / *Round 序列* 的概念(round 是 cert 自带字段,不是全局状态)。
- **证据**:
  - `consensus/src/lib.rs:569-571`(`if certificate.round != round { return Err(WrongRound) }` —— 只比较 cert 与 expected,不检查 round 的值);
  - `consensus/src/lib.rs:646-668`(signature 输入包括 `round.to_le_bytes()`,但 round 不参与任何 sanity check);
  - `consensus/src/lib.rs:289-290`(`pub round: u32` —— 字段类型 u32,无 Newtype 约束);
  - `consensus/src/lib.rs:541-553`(fixture helper 直接接受 `round: u32`)。
- **影响**:一个攻击者可以构造 *"round = 999999999"* 的 cert,verifier 接受它 —— 因为 verifier 没机会知道这是 *不合理* 的 round。在 L1 court 投影上,*"这个 cert 在 round 999999999 签了"* 是 *语义上不可信* 的(round 数量暗示从创世开始已经走完 999999999 轮共识),但 verifier 没有依据拒绝。
- **建议方向**:协议层应规定:
  - `MyelinBlock.number` (u64)与 `cert.height` (u64)必须相等 —— `lib.rs:613` 已实现;
  - `cert.round` 必须 < *某合理上限*(例如 `u32::MAX` 即可,但 doc string 要说明 "本协议层不实施 round 序列,仅记录 round 字段");
  - 或引入 `RoundNumber(u32)` newtype 并在构造时检查上限。

#### F-CONS-13 [LOW · 严密度]

- **观察**:`MyelinBlock`(`lib.rs:158-179`)没有 `proposer: [u8; 32]` 或 `proposer_signature: Signature64` 字段。Tendermint 协议要求每轮有 proposer,proposer 由确定性规则(round-robin 或 stake-weighted)选举产生。Myelin 的 Tendermint 引擎在协议层 *没有 proposer 概念* —— 这是因为模型是 *verifier-only*;谁提议 block、提议哪个 block,是外部过程(可能是 session runtime,也可能是 L1 court 投影)。但 *protocol-level* 这意味着:同一 `(height, round)` 可以有 *多个* validator *同时* precommit 不同 `block_hash`(因为没有任何机制选 proposer);只有当 ≥ quorum_power 的 validator *恰好* precommit 同一 `block_hash` 时,该块被 finalize。
- **证据**:
  - `consensus/src/lib.rs:158-179`(`MyelinBlock` 字段列表 —— 无 `proposer_*`);
  - `consensus/src/lib.rs:307-323`(`ConsensusEngine` trait —— 无 proposer selection 方法);
  - `consensus/src/lib.rs:484-489`(`Tendermint` struct 字段 —— 无 proposer state);
  - `consensus/src/lib.rs:556-596`(`verify_precommit_certificate` —— 不验证 proposer,只验证 precommit signatures)。
- **影响**:
  - 协议层 *不能* 阻止 "50% validator precommit block A, 51% validator precommit block B" 的分裂 —— 因为没有 proposer 来 *收敛*。
  - 这其实是 `MEMPOOL_CONSENSUS F-01` 的协议层镜像:`F-01` 是 *签后不检测*,`F-CONS-13` 是 *签前不收敛*。
  - 在 fixture 边界内这无所谓(只有一个 runner);在 L2 业务边界上,*"weighted precommit"* 的安全性完全依赖 *"所有 validator 在签前能看到对方打算签什么"* —— 这是协议层外的 social assumption。
- **建议方向**:协议层应明确 *proposal 是协议层外的过程*,并在 doc 中说明 *"本协议的 finality 不抗 concurrent precommit 分裂;若需要抗分裂,需要 L1 court 在收到 conflicting finalised block 时强制二选一"*。或者引入 *proposer* 概念 + proposer 优先级规则。

#### F-CONS-14 [MEDIUM · 严密度] *(协议层延伸 `MEMPOOL_CONSENSUS F-03/F-04`)*

- **观察**:mempool 的 *"确定性冲突打分"* 依赖:
  - `DETERMINISTIC_POOL_TIMESTAMP = 0`(`cellpool.rs:13`)—— 字节级确定;
  - `TransactionScorer` 的 `f64` 加权(`scorer.rs:88-96`:`α · fee_density + β · unlockability - γ · deps_width`);
  - `ConflictKey::from_entry` 的 `f64 → u64` saturating cast(`cellpool.rs:51-63`);
  - `get_sorted` 的 `f64::total_cmp`(`cellpool.rs:234-242`)。
  
  Rust 的 IEEE 754 规范保证 `+ - * /`、`as u64` saturating cast、`f64::total_cmp` 在 *well-formed inputs* 下是字节级确定的。**但**:`scorer.rs` 没 NaN guard(输入可能产生 NaN 时 — 例如 `effective_size = 0`、`fee = 0`,`0/0 = NaN`),`get_sorted` 的 `partial_cmp().unwrap()` 在 NaN 时 *panic*(`MEMPOOL_CONSENSUS F-04`)。**协议层没有** *"打分函数必须在所有输入下不 panic"* 的不变量 —— 它只说 *"deterministic"*。
- **证据**:
  - `mempool/src/cellpool.rs:13`(`DETERMINISTIC_POOL_TIMESTAMP`);
  - `mempool/src/cellpool.rs:51-63`(`ConflictKey::from_entry` 的 f64 cast);
  - `mempool/src/cellpool.rs:234-242`(`get_sorted` 的 `total_cmp`);
  - `mempool/src/scorer.rs:88-96`(`compute_score` 的 f64 加权);
  - `mempool/src/scorer.rs:99-109`(`compute_fee_density` —— `effective_size = size.max(cycles_size)` 两者都可为 0,`0/0 = NaN` 不会发生在 `> 0.0` 守护后);
  - `MEMPOOL_CONSENSUS F-03/F-04`(`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:44-45`)。
- **影响**:在 *协议层承担 L2 finality* 的边界上,mempool 的确定性是 *block.hash()* 的输入(通过 `ordered_cell_tx_commitments` 字段) —— `block.hash()`(`lib.rs:199-204`)是 blake3 重算,只要 ordered_cell_tx_commitments 列表的 *顺序* 确定,block hash 确定。**因此**,只要 mempool 的 *deterministic sort* 字节级稳定,共识 finality 的 hash chain 才稳定。**所以** mempool 的 f64 依赖是协议层 *隐含的* 不变量,需要 doc 明确承诺。
- **建议方向**:协议层应在 `MYELIN_SESSION_L2_PLAN.md` 与 `MYELIN_CONSENSUS_COMPLETENESS.md` 中明确:
  - 打分函数用 IEEE 754 严格语义,Rust 实现字节级确定;
  - 所有 NaN / ±∞ 输入必须 *在 scorenter 之前* 被剔除(目前 `compute_fee_density` 通过 `effective_size > 0.0` 守护,但 `compute_unlockability` 通过 `inputs.is_empty()` 早期返回 —— 守护 *不完整*);
  - `get_sorted` 的 `partial_cmp().unwrap()` 必须改为 `total_cmp().reverse()` 或显式 NaN 处理(实现层 `F-04`,协议层需要保证 *"排序不 panic"* 的不变量)。

#### F-CONS-15 [MEDIUM · 安全性]

- **观察**:`CommitteeValidator::public_key: Hash32`(`lib.rs:241`)是 *32 字节 hash-like bytes*,`parse_hex_32`(`lib.rs:150-154`)只验证 *"是 32 字节"*,不验证 *"是某个密码学公钥"*。在 fixture 边界内,32 字节 `[seed; 32]`(`lib.rs:741`)作为 public key —— 这是 *任意* 32 字节,不携带任何密码学承诺。**协议层后果**:`F-CONS-01` 的 *"signature 不是密码学签名"* 在 *协议层* 完全可观察 —— public_key 本身没有 *key generation* 概念,没有 *key ownership proof* 概念。"Validator V 拥有 public key PK" 这句话在协议层 *没有定义*。
- **证据**:
  - `consensus/src/lib.rs:150-154`(`parse_hex_32` —— 只检查 32 字节);
  - `consensus/src/lib.rs:241`(`pub public_key: Hash32`);
  - `consensus/src/lib.rs:386-398`(`StaticClosedCommittee::new` —— 不验证 public key 是某个 curve 上的点);
  - `consensus/src/lib.rs:500-513`(`Tendermint::new` 同上);
  - `consensus/src/lib.rs:741`(测试 fixture: `public_key: [seed; 32]`)。
- **影响**:在 L2 业务边界上,如果 *public_key 字段* 被解读为 *"validator V 的身份证明"*,则攻击者可以构造 *任意* 32 字节 public key,声称自己是 validator;协议层不阻止。**这是 `F-CONS-01` 的协议层根**。
- **建议方向**:协议层应在 `CommitteeValidator` 上加 *"public key 是 32 字节 hash-like bytes" 的明确 doc 注释*,并显式声明 *"本协议层不规定 public key 必须是某个曲线上的点;validator 身份由 (id, public_key) pair 共同标识,trust 来自 TOML 配置"*,以免被外部理解为 Ed25519 / secp256k1 / BLS 兼容。

#### F-CONS-16 [MEDIUM · 安全性] *(协议层延伸 `MEMPOOL_CONSENSUS F-09`)*

- **观察**:`MEMPOOL_CONSENSUS F-09` 已在实现层记录 *"Tendermint signature 域不包含 consensus_kind;若同一 (validator_id, public_key) 在两引擎的 config 中都出现,cert 本身在两个 engine 下都 parse"*,engine-level guard 在 block.consensus_kind 上做。本 lane 在 *协议层* 延伸:`CommitteeValidator`(`lib.rs:236-244`)被两引擎共用(`lib.rs:120` 与 `lib.rs:135` 都构造 `CommitteeValidator`),即同一个 validator struct 可以在 `StaticCommitteeConfig.validators` 与 `TendermintConfig.validators` 中分别出现。协议层没有 *"validator identity 在两个引擎下必须不同"* 的不变量。
- **证据**:
  - `consensus/src/lib.rs:236-244`(`CommitteeValidator` 是 pub struct,两引擎共用);
  - `consensus/src/lib.rs:120` 与 `lib.rs:135`(两引擎的 `try_from` 都构造 `CommitteeValidator`);
  - `MEMPOOL_CONSENSUS F-09`(`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:50`)。
- **影响**:在协议层,*"validator V" 的身份是 (id, public_key)* 字符串对。同一字符串对在不同 engine 的 validator set 中出现 *协议层允许*。结合 `F-CONS-09` 的 64 字节 commitment 命名错位,与 `F-CONS-15` 的 *"public_key 是 hash-like 32 字节"*,整体看 *协议层的 validator 身份模型是模糊的*:同一 validator 在不同 engine 的不同 cert 里签的"signature" 在 verifier 层是 *可区分的*(域分离),但在 *协议层语义* 上,*"validator V 在 Tendermint 下签了 X"* 和 *"validator V 在 static committee 下签了 X"* 是 *两个独立的承诺* —— 这与 *"validator V 是同一个实体"* 的直觉不一致。
- **建议方向**:协议层应:
  - 在 `CommitteeValidator` doc 中声明 *"同一 (id, public_key) pair 在两引擎的 validator set 中独立有效,跨 engine 的 cert 不共享 validator identity 语义"*;
  - 或引入 `EngineId(ConsensusKind)` 作为 `CommitteeSignature` 的字段,显式区分 *"谁在哪个 engine 下签的"*,把 `F-CONS-07` 的 kind guard 升级为类型系统。

#### F-CONS-17 [LOW · 合理性]

- **观察**:`MyelinBlock.consensus_kind` 是 `ConsensusKind` 枚举(`lib.rs:27-32`),序列化时是 `as_str()` 返回的字符串(`lib.rs:189`)。协议层没有 *"consensus_kind 字符串必须稳定"* 的不变量 —— `as_str()` 当前返回 "static-closed-committee" / "tendermint",如果未来 enum 顺序变化、加入新 variant,字符串可能改变,block.hash() 跟着变。**协议层后果**:`MyelinBlock` 在 *协议升级* 时会失去 backward compatibility(老 block 的 hash 在新协议下重算结果不同)。
- **证据**:
  - `consensus/src/lib.rs:34-42`(`ConsensusKind::as_str` —— 当前字符串值,但没有 "frozen / canonical" 标注);
  - `consensus/src/lib.rs:189`(`self.consensus_kind.as_str().as_bytes()` 进入 hash);
  - `consensus/src/lib.rs:199-204`(`block.hash()` —— 把 consensus_kind 的字符串 binding 到 hash)。
- **影响**:协议升级(新增引擎、改名)会导致 *所有已有 finalised block 的 hash 失效* —— L1 court 上的 DA manifest / settlement intent 引用 block.hash(),hash 失效意味着 court bundle 必须重做。
- **建议方向**:协议层应在 `ConsensusKind::as_str` 上加 doc 注释 *"consensus_kind string is part of block hash and is a wire-stable identifier;changes to this string require a protocol version bump"*。或引入 `MyelinBlock.version: u32` 已经做了部分(看 line 160 —— `pub version: u32`),但 version 跟 consensus_kind 是独立字段,协议升级需要 *同时* bump 两个。

#### F-CONS-18 [MEDIUM · 安全性]

- **观察**:mempool 的 `try_replace_by_fee`(`cellpool.rs:268-313`)是 *递归* 调用 `add`,`MEMPOOL_CONSENSUS F-02` 已在实现层指出"递归 add 可导致 `MempoolError::TxExists` 返回或循环"。本 lane 在 *协议层* 延伸:协议层声称的 *"deterministic conflict resolution"* 建立在 *"每个 wtxid 只会进入池一次"* 之上,但 `try_replace_by_fee` 的递归路径可能 *多次进入同一 wtxid*。**协议层后果**:`try_replace_by_fee` 的行为不是纯函数 —— 它的输出取决于 *pool 的当前状态*(在递归过程中被 mutate)。两个完全相同的调用序列,在不同 *初始 pool state* 下,可能产生不同结果。这破坏了 *"deterministic conflict resolution"* 的协议层承诺。
- **证据**:
  - `mempool/src/cellpool.rs:268-313`(`try_replace_by_fee` 的完整实现);
  - `mempool/src/cellpool.rs:312`(`self.add(tx.clone(), fee, cycles)` —— 递归调用);
  - `mempool/src/cellpool.rs:36-49`(ConflictKey 的 doc comment 声明确定性);
  - `MEMPOOL_CONSENSUS F-02`(`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:43`)。
- **影响**:两个 mempool 实例,在不同初始状态下,收到同一串 add 调用后,可能 *进入不同最终状态*。这破坏了 *mempool → consensus* 的协议层不变量:同一 `ordered_cell_tx_commitments` 序列应该在所有 mempool 实例上得到同一列表。
- **建议方向**:协议层应在 doc 中说明 *"try_replace_by_fee 在递归路径下不是确定性函数;协议层 finality 应假设 mempool 已经把所有 RBF 处理完毕后才进入 consensus 路径,而不是假设 RBF 在 finality 阶段被重新触发"*。或实现层将 `try_replace_by_fee` 改为非递归(先计算 *理想* 池状态,再一次性 commit)。

## 3. 合理性评估

### 3.1 总体

模型的 *合理性*(assumption 是否现实)在三个层面上分别评估:

**A. "Closed validator" 的封闭性**:协议层 *不可强制* —— `F-CONS-05`。在 L2 业务边界上,这意味着 *"closed validator 是 trust assumption,不是 protocol invariant"* —— 跟 `MYELIN_SESSION_L2_PLAN.md` 的 *"session benchmarking and pressure testing"* 自我定位(`docs/MYELIN_ARCHITECTURE.md:43-48`)一致,但跟公开叙事里偶尔出现的 *"Myelin 是 L2"* 表述有距离。

**B. "Tendermint-style" 的语义匹配度**:公开命名 *Tendermint* 暗示 *完整 Tendermint 安全模型*,实际只实现了 *precommit verifier* —— `F-CONS-02`、`F-CONS-11`。文档内部有 *"stripped variant"* 的诚实承认,但外部阅读者很难从命名中分辨。

**C. 加权 vs 简单多数**:`CommitteeValidator.weight: u64`(`lib.rs:243`)在协议层 *实际是简单多数* —— 因为验证路径(`lib.rs:451-458`、`587-594`)只 sum weights 然后比较 `>= quorum_weight`。如果 weight 都是 1,等价于"≥ quorum_weight 个 validator 签"。weight 字段在 *协议层* 是 *冗余* 的 —— 它 *没有* 影响 quorum 的"加权"含义(没有"weight=100 的 validator 抵 100 个 weight=1 的 validator"这种语义,因为 quorum_weight 也是单一标量阈值)。**协议层后果**:`F-CONS-03` 揭示的"无最小阈值"在加权背景下被放大 —— 一个 weight=10^9 的 validator 即可独自 finalise,即使其余 validator weight 总和 = 1。

### 3.2 Findings

合理性维度的 finding 主要在 §2.2 中以 *严密度* / *安全性* 标记呈现(因为大多数合理性问题在协议层都体现为 *invariant 不闭环*)。这里补充两条只关合理性的:

#### F-CONS-19 [MEDIUM · 合理性]

- **维度**:合理性
- **观察**:`weight: u64`(`lib.rs:243`)与 `quorum_weight: u64`(`lib.rs:252`)的协议层组合方式,等价于 *简单多数*。如果 weight 字段被理解为 *"validator 的投票权"*,则攻击者可以让一个 validator 的 weight 占 total_weight 的 > quorum_weight 比例,独自 finalise。**协议层不变量缺失**:不强制 *"任何单个 validator 的 weight 必须 < quorum_weight"*(防止单 validator 独占)。
- **证据**:
  - `consensus/src/lib.rs:386-405`(`StaticClosedCommittee::new` —— 校验 `validator.weight != 0`、`total_weight` 不溢出、`quorum_weight ≤ total_weight`,但 *不* 检查 max(weight) < quorum_weight);
  - `consensus/src/lib.rs:500-520`(Tendermint 同上);
  - `consensus/src/lib.rs:451-458`、`587-594`(verify 路径只 sum,不做比例检查)。
- **影响**:在 fixture 边界内这是 *诚实的简化*;在 L2 业务边界上,*协议层允许* 一个 validator weight = 10^9、其余 weight = 1,quorum_weight = 10^9 —— 该 validator 独自 finalise,模型等同于"集中式"。
- **建议方向**:协议层应加 *"max(weight) < quorum_weight"* 的 invariant(或 `quorum_weight > max(weight)`),让"加权"在协议层 *真正* 体现为 *多 validator 共同*。

#### F-CONS-20 [LOW · 合理性]

- **维度**:合理性
- **观察**:协议层 *没有* 显式声明 *"Validator 必须是 honest-but-curious / Byzantine / crash-fault-only"* 假设。在 Tendermint 原论文里,模型假设 *"≤ 1/3 validator 是 Byzantine"* —— 这不是技术约束,是 *模型假设*。Myelin 的 Tendermint 引擎没在任何 doc 中复述这一假设,导致 L2 业务方在解读 *"weighted precommit"* 时可能误以为 *任何* quorum_weight 个 validator 签了就一定 safe。
- **证据**:
  - `consensus/src/lib.rs:484-489`(`Tendermint` struct doc —— 没说 Byzantine 容错假设);
  - `MYELIN_CONSENSUS_COMPLETENESS.md:1-247`(全文无 "Byzantine"、"fault tolerance"、"1/3"、"2/3" 等关键词的显式协议假设声明);
  - `MYELIN_SESSION_L2_PLAN.md:1-556`(无此声明)。
- **影响**:L2 业务方读 *"weighted precommit finality"* 时,会假设 *任意* quorum_weight 个 validator 签了就 final —— 但协议层只保证 *"那个 quorum_weight 个 validator 在那一刻签了"*,*不* 保证 *"没有其他 validator 在同一 (height, round) 签了不同 block_hash"*(见 `F-CONS-06`)。
- **建议方向**:协议层应在 `Tendermint` struct doc 与 `MYELIN_CONSENSUS_COMPLETENESS.md` 显式声明 *"本引擎在协议层不实施 Byzantine 容错;fault tolerance 假设由 TOML 配置 + 外部 trust anchor 提供"*。

## 4. 安全性评估

### 4.1 总体

安全性在三个攻击维度上评估:

**A. 单 validator 攻破**:`F-CONS-03` (无 quorum 下界) + `F-CONS-19` (无 max weight < quorum 约束) + `F-CONS-15` (public_key 无密码学承诺) —— 一个被攻破的 validator 在协议层可以 *独自* finalise 任意 block(配置允许的话)。

**B. Validator 串谋**:协议层 *不检测*(见 `F-CONS-06`、`MEMPOOL_CONSENSUS F-01`)。Quorum_weight 个 validator 串谋,可以:
  - 签两个互相冲突的 `FinalisedBlock`(同一 height, 不同 block_hash);
  - 签两个互相冲突的 `FinalisedTendermintBlock`(同一 height + round, 不同 block_hash);
  - 在 L1 court 投影的 *"finality gap"* 期间(见 `F-CONS-08`),重新签发覆盖 cert。

**C. Long-range / nothing-at-stake / grinding**:协议层 *不讨论* 这些攻击的处置 —— 因为模型是 *verifier-only* + *closed committee*,从设计上 *没有* long-range 风险路径(没有 validator set 演化,所以没有"老 validator set 复活"的攻击面)。nothing-at-stake 与 grinding 是 *状态机* 概念(节点在多个 fork 上同时签以增加命中率),而 Myelin 没有 fork choice —— 所以这些攻击在协议层 *不适用*,但 *也没有协议层论证说明为什么不适用*。

**D. L1 关系**:Myelin finality 是 *封闭委员会在那一刻的签名*,与 CKB finality *正交* —— `F-CONS-08`。一个 Myelin finalised block 在 L1 CKB 上 reorg 概率是 *独立* 事件,协议层没有 *checkpoint* 机制把两者绑定。

### 4.2 Findings

安全性维度的 finding 大多已在 §2.2 中呈现。这里补充两条专门安全性维度的:

#### F-CONS-21 [MEDIUM · 安全性] *(协议层延伸 `MEMPOOL_CONSENSUS F-01`)*

- **维度**:安全性
- **观察**:`MEMPOOL_CONSENSUS F-01` 已记 *"跨 (height, round) equivocation 不可检测"*。本 lane 在 *协议层* 进一步指出:模型 *没有"finality gap 内不允许签发 conflict"的协议层语义* —— 在 *commit 后到 L1 投影前* 的窗口内,被攻破的 validator 可以 *单机* 重签一个 `FinalisedBlock`(静态委员会模式,quorum_weight=1)或 *联合 quorum_weight 个 validator* 重签 `FinalisedTendermintBlock`(Tendermint 模式)。即使 L1 court 最终拒绝 conflict,L2 业务方在 finality gap 期间已经按 *旧* finalised block 做了不可逆动作(签了 DA anchor、settled intent)。
- **证据**:
  - `consensus/src/lib.rs:296-304`(`FinalisedBlock` —— 无 L1 anchor 字段);
  - `consensus/src/lib.rs:619-629`(`FinalisedTendermintBlock` 同样);
  - `MYELIN_SESSION_L2_PLAN.md:262-280`(settlement-intent 描述里没有 *"在 settlement 期间 finalised block 是否可覆盖"* 的协议规则);
  - `MEMPOOL_CONSENSUS F-01`、`F-CONS-06`。
- **影响**:**见 `F-CONS-08` 的影响分析** —— 这是 *协议层* 的 finality gap 攻击窗口,在 L2 业务上等同于 *"finality 不可信直到 L1 投影完成"*。如果 L1 投影永远不发生(因为 L1 court 还没实现,见 `MYELIN_SESSION_L2_PLAN.md:266-268`),则 Myelin 的 finality *永远是暂时的*。
- **建议方向**:协议层应引入 *"finality checkpoint"* 概念 —— `FinalisedBlock` 上加 `l1_anchor_id` 字段(可选,直到 L1 投影完成才填),checkpoint 完成后 finality 不可逆。或者在 model 层承认 *"finality 在 L1 投影前是 provisional,不是 absolute"*。

#### F-CONS-22 [LOW · 安全性]

- **维度**:安全性
- **观察**:`StaticClosedCommittee::validators: HashMap<String, CommitteeValidator>`(`lib.rs:372`)与 `Tendermint::validators: HashMap<String, CommitteeValidator>`(`lib.rs:487`)的迭代顺序 *非确定*(HashMap 在 Rust 里是 `RandomState` + 哈希种子),但 verify 路径(`lib.rs:439`、`575`)迭代的是 *certificate 的 signatures Vec*,不是 validator map,所以 verify 路径的输出是确定的。**协议层后果**:若未来某个 API 把 `validators` map 的迭代顺序暴露给外部(例如导出 JSON、生成 transcript),则 *同一配置 + 同一证书* 可能产生 *不同 transcript*,破坏 *"可重放验证"* 的协议层承诺。
- **证据**:
  - `consensus/src/lib.rs:13`(`use std::collections::{HashMap, HashSet}`);
  - `consensus/src/lib.rs:372`、`487`(`validators: HashMap<String, CommitteeValidator>`);
  - `consensus/src/lib.rs:439`(`for signature in &certificate.signatures` —— 迭代 cert,不是 map);
  - `consensus/src/lib.rs:575`(同样)。
- **影响**:协议层 *目前* 安全(因为 verify 路径不依赖 map 迭代顺序);但 *协议层不强制* 未来 API 不暴露 map 迭代 —— 这是一个 *lurking risk*。
- **建议方向**:协议层应在 doc 中说明 *"validator map 迭代顺序不稳定;任何暴露 validator 集合的 API 必须 sort by (id, public_key) 后输出"*。

## 5. 与已存在 audit 的关系

`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` 的 19 条 finding 中:

- **F-01 (CRITICAL, Tendermint cross-(height,round) equivocation)**:本 lane 的 `F-CONS-06` 在协议层延伸 —— 不仅是 *"不可检测"*,而是 *"协议层没有任何 evidence log / slashing / accountability 机制"*。
- **F-02 (HIGH, RBF recursive add)**:本 lane 的 `F-CONS-18` 在协议层延伸 —— 不仅是 *"实现层可能 panic 或返回错误"*,而是 *"协议层声称的 deterministic conflict resolution 在 RBF 路径下不成立"*。
- **F-03 (HIGH, fee_density f64 cast)**:本 lane 的 `F-CONS-14` 在协议层延伸 —— 不仅是 *"饱和行为 benign"*,而是 *"协议层不变量建立在 IEEE 754 字节级稳定之上,需要在 doc 明确"*。
- **F-04 (HIGH, get_sorted NaN panic)**:被 `F-CONS-14` 吸收 —— 协议层 *"排序不 panic"* 是隐含 invariant。
- **F-06 (MEDIUM, timestamp non-determinism)**:被 `F-CONS-04` 吸收并扩展到协议层(原 F-06 在实现层,本 lane 在协议层 —— *timestamp 来源未定义*)。
- **F-09 (MEDIUM, signature domain missing consensus_kind)**:被 `F-CONS-16` 吸收并扩展到协议层(原 F-09 在实现层指出 engine guard 只在 block 层,本 lane 指出 CommitteeValidator 在两引擎共用导致 validator identity 模糊)。
- **F-10 (MEDIUM, no session_id on config)**:被 `F-CONS-05` 吸收 —— 协议层 "closed validator" 是 config 层而非 protocol 层。
- **F-11 (LOW, timestamp_ms 在 block hash)**:被 `F-CONS-04` 吸收 —— 协议层不规定 timestamp 来源。
- **F-12 ~ F-15 (LOW/INFO)**:与本 lane 关注点正交(实现层细节)。
- **F-16 ~ F-18 (INFO)**:本 lane 不重复(hygiene / Cargo pin)。
- **F-19 (MEDIUM, CLI quorum_signers 顺序)**:本 lane 不重复 —— CLI 层,在协议层之外。

本 lane **新增** 的 finding(非延伸,独立 protocol-layer claim):`F-CONS-02`、`F-CONS-03`、`F-CONS-07`、`F-CONS-08`、`F-CONS-09`、`F-CONS-10`、`F-CONS-11`、`F-CONS-12`、`F-CONS-13`、`F-CONS-15`、`F-CONS-17`、`F-CONS-19`、`F-CONS-20`、`F-CONS-21`、`F-CONS-22`。

## 6. 已知缺陷的协议层影响

**F-PRIM-01**(`exec/src/celltx/types.rs:299-307, 316-324`,`compute_conflict_hash` / `compute_typed_data_hash` 长度前缀缺失,`(args="X", data="")` 与 `(args="", data="X")` 碰撞)在共识子系统的协议层表现如下:

- **共识层不直接调用 `compute_conflict_hash` / `compute_typed_data_hash`**(`consensus/src/lib.rs` 没有 `use myelin_exec` import,共识层只 hash `MyelinBlock` 与签名,不接触 cell identity)。
- **但** `MyelinBlock.ordered_cell_tx_commitments: Vec<Hash32>`(`lib.rs:174`)是 *外部输入* —— 这些 commitment 的字节级含义不在 `consensus` crate 内定义。如果 ordered_cell_tx_commitments 来自 `exec` 的 `compute_typed_data_hash`,且 `exec` 的 cellid hash 碰撞(F-PRIM-01),则 *两个不同 CellTx* 可能贡献同一 commitment,进入同一 `ordered_cell_tx_commitments` 列表,**block.hash() 不区分二者**。
- **协议层后果**:`MyelinBlock.hash()` 是 finality evidence 的核心(verifier 重算 block_hash),如果 hash 输入域(`ordered_cell_tx_commitments`)有 collision class(F-PRIM-01),则 finality evidence 不能区分 *两个不同 CellTx 集合* —— 攻击者可以构造 *collision pair*,让 L2 业务方误以为 *CellTx A 进 block* 但实际是 *CellTx B 进 block*。
- **本 lane 不重复 F-PRIM-01** —— `audits/swarm-wholerepo/LANE_PRIMITIVES.md:56` 与 `audits/swarm-all-dimensions-2026-06-27/LANE_CELLSCRIPT_COMPILER.md:453` 已记录。本 lane 在协议层延伸为:consensus 子系统的 *finality evidence 完整性* 取决于 *ordered_cell_tx_commitments 的 hash 完整性*;后者取决于 *exec crate 的 cellid hash 完整性*(F-PRIM-01 在那里)。**协议层耦合**:exec 层的 collision class 感染 consensus 层的 finality 强度。

## 7. 跨子系统影响(给 synthesis lane 用)

共识子系统的协议层不变量断裂,对其他子系统有以下协议层假设传染:

- **DA 子系统**(分层 DA + external receipt):consensus 的 finality evidence(`MyelinBlock.hash()` + committee certificate)是 DA manifest 锚定的核心。`F-CONS-08` 揭示的 finality gap 在 DA 上的含义是:DA manifest 锚定的 *"已 finalised block hash"* 在 L1 投影完成前 *可能* 被 consensus 引擎覆盖(quorum 串谋),DA 的 *"anchor once" 不变量* 在协议层与 consensus 的 *"anchor may re-anchor"* 不一致 —— **DA 不能假设 consensus finality 是 final**。这要求 DA 子系统的 protocol-level claim ladder 显式承认 *"DA 锚定的 Myelin block hash 在 L1 投影完成前是 provisional,不是 final"*。

- **Court 子系统**(chunk adjudication + dispute path):consensus 的 static committee 决议 + Tendermint 决议是 court dispute 的 *"初始事实"*。`F-CONS-06` 揭示的 equivocation 不可检测在 court 上的含义是:court *不能* 假设 committee 的决定是 *唯一的* —— court 必须能在 protocol level 上处理 *"同一 (height, round) 有两个互相冲突的 FinalisedBlock"* 的输入。`F-CONS-02` 揭示的 *"Tendermint 不是状态机"* 在 court 上的含义是:court 不能从 *"Tendermint precommit certificate"* 推断 *BFT safety* —— court 必须独立验证 block 的 state transition 正确性(`MYELIN_SESSION_L2_PLAN.md:182-184` 的 `verify-court-bundle` 已经在做),不能依赖 consensus 的 precommit 给出 safety claim。

- **Settlement 子系统**(authority cell + deployment evidence):consensus 的 finality 在 settlement 上的含义是 *finality 是 settlement 的输入*。`F-CONS-05` 揭示的 *"closed validator 是 config 层而非 protocol 层"* 在 settlement 上的含义是:settlement authority cell 必须 *不* 信任 committee config 作为唯一 trust anchor —— settlement 必须有 *自己的* authority cell 协议(`MYELIN_SESSION_L2_PLAN.md:271-292` 的 `settlement-package` 已经在做)。`F-CONS-21` 揭示的 finality gap 在 settlement 上的含义是:settlement intent 引用 block_hash 时,必须 *同时* 引用 *该 block_hash 的 finality checkpoint 状态*,否则 settlement intent 在 protocol level 上不能保证 *引用的是 committee 唯一决议的版本*。

总结一句话给 synthesis lane:共识子系统在协议层 *没有给出 BFT safety claim*(`F-CONS-02`、`F-CONS-06`、`F-CONS-21`)、*没有给出 cryptographic authentication claim*(`F-CONS-01`、`F-CONS-15`)、*没有给出 finality durability claim*(`F-CONS-08`、`F-CONS-04`)。其他三个子系统(DA、Court、Settlement)在协议层 *不能* 把 Myelin consensus 当作 *强 finality 锚点* —— 它们的 protocol-level claim ladder 必须显式声明 *"consensus finality 是 provisional,直至 L1 投影完成"*。