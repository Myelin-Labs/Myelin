# CellFabric 侧 — 自述与对 CellScript 的现有配合面

> 仓库：`/Users/arthur/RustroverProjects/CellFabric`
> Crate 名（按 `Cargo.toml`）：`cell-fabric`（version `0.1.1`，`rust-version = "1.92.0"`）
> 见 `/Users/arthur/RustroverProjects/CellFabric/Cargo.toml:2,9`
> 本报告的每条结论都附 file:line 证据，不做建议、不做版本叙事。

---

## 0. 阅读前发现的一处与任务说明书不一致

任务说明书第 8 项要求读 `src/cellscript_amm.rs`：

> 8. `/Users/arthur/RustroverProjects/CellFabric/src/cellscript_amm.rs`（AMM-specific import，跟通用 import 的差别）

但该文件在仓库里 **不存在**。`/Users/arthur/RustroverProjects/CellFabric/src/cellscript_bridge.rs`（lines 1, 643–759）同时承载通用 import 和 AMM-specific import（`import_cellscript_amm_swap_request`、`import_cellscript_amm_swap_response`、`import_cellscript_amm_swap_hex_*`、`import_cellscript_amm_swap_wire_*`）。本报告的"3.1 通用 import"与"3.4 AMM 路径"两节都引用 `cellscript_bridge.rs` 作为同一文件的两段。

---

## 1. CellFabric 是什么（一段话）

CellFabric（crate `cell-fabric`）是一个独立的 Rust crate，定位为 **CKB-settled 的 cell-intent ordering layer**（`README.md:3`）—— 它假设 CKB L1 独占 finality、上层做 intent DAG + 软排序 + 显式 conflict + 把不可变 bundle 编译到 CKB 交易，从而在 Phase 0/1 提供一个"可测试、可验证、可审计"的领域层（`README.md:135-150`、`docs/principles-tutorial.md:26-29`）。它不是 service binary：gateway / orderer / submitter / proof service 都构建在它之上（`README.md:3-5`）。当前可选 feature 只有 `http`（Axum dev server，`Cargo.toml:14`）和 `ckb-rpc-submit`（`ckb-jsonrpc-types` + `reqwest`，`Cargo.toml:15`），其余都是 crate-internal capability（`README.md:269-270`、`README.md:384-388`）。

---

## 2. CellFabric 核心模型

### 2.1 数据流（README 与 tutorial 自陈）

`docs/principles-tutorial.md:107-119` 给出官方 mermaid 图：

```
User signs SignedIntent
  → Gateway validation
  → Single-writer IntentEngine actor
  → IntentDag indexes cells, app keys, dependencies
  → Conflict records remain visible
  → Orderer selects conflict-free Bundle
  → Non-final BundleReceipt
  → SettlementCompiler uses immutable Bundle
  → SettlementPlan with concrete CKB tx bytes
  → CkbSubmitter sends to CKB RPC
  → SettlementTracker observes L1 status
  → Settled only after CKB commitment/finality
```

`docs/principles-tutorial.md:121-123` 明说："Only CKB L1 commitment is final. Bundle receipts and soft confirmations are explicitly non-final."

### 2.2 关键类型

> 完整 module map 在 `src/lib.rs:8-43`、`README.md:296-336`。

- `IntentBody`、`SignedIntent`（`docs/principles-tutorial.md:56-74`）
  - `SignedIntent::id` 仅由 `IntentBody` 派生；auth 字节不参与语义身份
- `OutPointRef`、`ScriptSpec`、`CellTemplate`、`ResourceAccess`、`IntentAction`（`Mint` / `Transfer` / `Swap` / `App`）、`ReplacementPolicy`、`IntentAuth`（`docs/principles-tutorial.md:67-73`、`README.md:128`）
- `IntentDag`：`src/dag.rs`，含 hard-conflict index、app-conflict index、conflict records、replacement state（`docs/principles-tutorial.md:194-195`）
- `Bundle`、`BundleReceipt`、`SignedBundleReceipt`（`src/bundle.rs`，`docs/principles-tutorial.md:218-237`）
- `SettlementDraft`、`SettlementPlan`、`SettlementPlanEvidence`、`SettlementFinalityEvidence`（`src/draft.rs`、`src/settlement.rs`、`src/tracker.rs`）
- `SoftConfirmationConfidence` —— 钱包侧摘要，"non_final" 永远为 true（`src/confidence.rs`、`docs/principles-tutorial.md:241-243`）
- `ConflictProof`、`DisputeProof`、`ExitClaim`、`ReceiptEquivocationProof`（`src/evidence.rs`、`docs/principles-tutorial.md:420-447`）

### 2.3 Invariant 分组

来源主要是 `docs/red-lines.md` 和 `README.md:86-97` + `README.md:226-289`。

#### 2.3.1 架构 / finality（`red-lines.md:10-58`、README.md:86-97）

- 软确认永不等于 L1 finality；L1 rejected 是 terminal state，不是 warning
- `BundleReceipt.non_final` 必须为 `true`
- 单 writer actor；raw actor command + 直接 settlement lifecycle mutation 是 crate-internal capability，外部必须走 gateway / orderer / tracker / worker 服务（`README.md:268-270`、`README.md:382-388`）

#### 2.3.2 安全 / auth（`red-lines.md:60-91`）

- `IntentAuth` binding 只证明"auth 附在这个 body 上"，不等价于"用户授权"
- 默认 `NoopAuthVerifier` 不是真实安全件；production 必须 strict registry
- `CkbSecp256k1Blake160Verifier` 严格要求外部提供 trusted lock-script-hash ↔ lock-arg 映射（`docs/principles-tutorial.md:95-98`）
- `GatewayConfig::assert_production_ready` / `OrdererConfig::assert_production_ready` 会在 dev-friendly 默认上 fail（`README.md:372-378`）

#### 2.3.3 Reentrancy / purity（`red-lines.md:122-148`、`README.md:120-132`）

- Settlement compilation 必须是 immutable bundle snapshot 上的纯函数；不允许读 live `IntentDag`、持 async lock 跨 RPC/DB/CKB、依赖 wall-clock / 随机 / unordered map
- 编译输出对同一 bundle + compiler 配置必须可复现（`red-lines.md:140-142`）

#### 2.3.4 Double-spend / conflict（`red-lines.md:93-119`）

- Admit（不隐式拒）→ Quarantine（`ConflictRecord`）→ Exclude（selector 不共选）→ Evidence（proof service 可重建）→ Invalidate / Exit（待 CKB script）（`README.md:99-118`）
- Hard conflict 由 consumed `OutPointRef` 索引；App conflict 由 `AppConflictKey` 索引（`docs/principles-tutorial.md:188-201`）
- `AppConflictKey` 必须从 signed `IntentBody` 推导；不允许"先 sign、再 patch app_keys"（`red-lines.md:99-104`）
- App compiler 不允许在签名的 `consumes` / `reads` 之外添加 inputs（`red-lines.md:101`）

---

## 3. CellFabric 现有 CellScript 配合面

> 单一来源文件：`src/cellscript_bridge.rs`（1118 行；常量在 lines 15-16）。
> Bridge docs：`docs/cellscript-bridge.md`。
> 顶部导出表：`src/lib.rs:67-82`。

入口常量：

- `CELLSCRIPT_INTENT_ENVELOPE_SCHEMA = "cellscript-cellfabric-intent-envelope-v0.20"`（`cellscript_bridge.rs:15`）
- `CELLSCRIPT_INTENT_ENVELOPE_STATUS = "requires-runtime-binding"`（`cellscript_bridge.rs:16`）

### 3.1 通用 import：`import_cellscript_intent_envelope`

实现位置：`src/cellscript_bridge.rs:478-634`。

签名：

```rust
pub fn import_cellscript_intent_envelope(
    envelope: &Value,
    binding: CellScriptIntentBinding,
) -> Result<CellScriptIntentImport>
```

`binding` 结构 `src/cellscript_bridge.rs:18-29` 包含：`author`、`nonce`、`consumes`、`reads`、`validity`、`constraints`、`dependencies`、`replacement`、`fee`。

Envelope 入口校验（顺序：`cellscript_bridge.rs:482-541`）：

1. `schema` 必须是 schema 常量（line 482）
2. `status` 必须是 status 常量（line 483）
3. `bridge_boundary.not_a_cellfabric_signed_intent = true`（line 484-488）
4. `bridge_boundary.not_a_soft_confirmation = true`（line 489-493）
5. `bridge_boundary.not_l1_finality = true`（line 494）
6. `bridge_boundary.cellscript_core_dependency = "no-cell-fabric-rust-crate"`（line 495-499）—— 这条与 `cellscript-bridge.md:30-37` 一致：CellScript 故意不依赖 `cell-fabric` crate，bridge 走 JSON schema（`bridle_boundary.kind` 在 `examples/cellscript_amm_flow.rs:249` 为 `"json-bridge"`）
7. `cellfabric_intent_template.domain.{chain_id,app_namespace}` 必须与 `source.target_profile` / `source.module` 一致（lines 507-516）
8. `cellfabric_intent_template.action.kind = "App"`、`action.action` 等于 `source.action`、`payload_format = "cellscript-action-plan-json-v1"`、`payload_hash` 等于 `source.action_plan_hash`、`auth_mode = "CoSignConcreteTx"`（lines 517-541）
9. 内嵌 `action_plan` JSON 重新 blake2b-256，必须等于 `source.action_plan_hash`（lines 543-552）

冲突键导入：`parse_app_conflict_keys`（`cellscript_bridge.rs:950-966`）要求 `cellfabric_intent_template.resources.app_keys` 和 `resource_access_template.app_conflict_key_templates` 完全一致；同时要求每个 key 的 `namespace` 等于 `source.module`（`cellscript_bridge.rs:554-562`）。

返回的 `IntentBody` 形态（`cellscript_bridge.rs:602-626`）：

- `version = 1`
- `domain.chain_id = source.target_profile`，`domain.app_namespace = source.module`
- `action = IntentAction::App { action: source.action, payload: action_plan_json_bytes }` —— 这里是 **完整 CellScript action-plan JSON bytes**，不只是 hash（`cellscript-bridge.md:54-57`）
- `auth_mode = CoSignConcreteTx`
- `metadata` 里写入 `cellscript_schema` / `cellscript_status` / `cellscript_action` / `cellscript_action_plan_hash` / `cellscript_payload_format` + 4 个可选 source metadata（`input` / `metadata_hash` / `artifact_hash` / `compiler_version`）复制（`cellscript_bridge.rs:564-600`）

注意：`metadata.cellscript_action_plan_hash` 与 `payload` 是双向绑定 —— 任何后续修改 payload 都会被 `validate_cellscript_intent_payload` 拒绝（`cellscript_bridge.rs:450-476`，下面 §3.2 详述）。这让外部 runtime builder 可以"保留 provenance，但替换 payload"（即 AMM 路径，§3.4）。

### 3.2 App conflict policy：`CellScriptAppConflictPolicy`

实现位置：`src/cellscript_bridge.rs:362-448`。

核心结构（`cellscript_bridge.rs:362-448`）：

```rust
pub struct CellScriptAppConflictPolicy {
    namespace: String,
    keys_by_action_plan_hash: BTreeMap<[u8; 32], BTreeSet<AppConflictKey>>,
}
```

`with_import` / `register_import`（`cellscript_bridge.rs:412-422`）把每次 import 的 `action_plan_hash → declared app_keys` 存起来。

`AppConflictPolicy::conflict_keys` 的实现（`cellscript_bridge.rs:434-447`）：

1. 调 `validate_cellscript_intent_payload(intent)` 重算 payload 的 blake2b-256，跟 `metadata.cellscript_action_plan_hash` 比对
2. 用重算出来的 hash 在 policy map 里查 declared app_keys
3. 把 stored app keys 作为 conflict keys 返回（selector 因此能拒绝同 action-plan hash 但不同 app-keys 声明的 attempt）

要点：

- Policy 是 namespace-scoped（`cellscript_bridge.rs:401`），所以同样的 scheme 可以挂在不同 namespace 上
- 它不做"自动派生 app keys"，必须显式 register_import，failure mode 是 `unknown CellScript action_plan_hash`（`cellscript_bridge.rs:438-444`）—— 这意味着 service 必须在 import 时间就把 hash 喂给 policy；selector 阶段不能"重新派生"，避免非确定性

Gateway 推荐配置（`docs/cellscript-bridge.md:100-109`）：

```rust
GatewayConfig::new("ckb-dev").with_required_app_policy_for_app_actions(true)
```

+ register `CellScriptAppConflictPolicy`。

### 3.3 Settlement 缺口的形状：`CellScriptRuntimeBuilderCompiler`

实现位置：`src/cellscript_bridge.rs:362-398`。

```rust
impl AppSettlementCompiler for CellScriptRuntimeBuilderCompiler {
    fn namespace(&self) -> &str { ... }
    fn compile_intent(&self, intent: &SignedIntent) -> Result<AppSettlementFragment> {
        let action_plan_hash = validate_cellscript_intent_payload(intent)?;
        Err(IntentError::UnsupportedSettlementAction(format!(
            "CellScript intent {} action_plan_hash 0x{} requires an external \
             CellScript runtime builder or adapter; CellFabric core does not \
             materialize CellScript action plans into CKB transactions",
            intent.id, hex::encode(action_plan_hash)
        )))
    }
}
```

来源：`cellscript_bridge.rs:385-398`。

要点：

- 这是一个 **故意 fail-closed** 的 `AppSettlementCompiler` 实现。它不返回 fragment，而是返回 `IntentError::UnsupportedSettlementAction`（`src/error.rs:162-163`）
- `compile_intents`（batch hook）默认从 per-intent 推导，所以这条也意味着 batch 编译落入同一条 error —— CellScript 通用路径连 batch 都不放过
- 注释和 docs 自陈这是一个 **boundary adapter**：CellFabric core 不把 CellScript action plan 物质化成 CKB 交易；service 必须接一个"外部 CellScript runtime builder"（`docs/cellscript-bridge.md:113-119`、`docs/cellscript-bridge.md:282-291` step 8）

为什么这是关键缺口（用 CellFabric 自己的语言）：

- `docs/principles-tutorial.md:259-271` 自陈 "Swap and App actions require (a) a registered app conflict policy for the namespace, (b) a registered app settlement compiler" —— 缺一不可
- `README.md:249-254` 与 `red-lines.md:111-112` 复述同一规则
- CellScript 通用 import 解决了 (a)（§3.2），**不**解决 (b)。`CellScriptRuntimeBuilderCompiler` 是 (b) 的占位——它的全部价值是"明确告诉上层：这条 namespace 没有 compiler"
- 这正是 generic CellScript 路径"只有 import + 没有 compile"的具体技术解释

注意：直接 path 命中"missing compiler"时返回的是 `MissingAppSettlementCompiler(namespace)`（`src/error.rs:101-102`）；但配了 `CellScriptRuntimeBuilderCompiler` 之后，错误类型是 `UnsupportedSettlementAction(...)`。两个错误 family 都 fail closed，但语义不同：前者是"你没注册 compiler"，后者是"你注册了 compiler，它说它干不了"。

### 3.4 AMM 路径：完整闭环的实现

实现位置：
- AMM-specific import 仍在 `src/cellscript_bridge.rs:643-759`
- AMM 域类型在 `src/amm.rs`

#### 3.4.1 AMM import path（`cellscript_bridge.rs:643-729`）

`import_cellscript_amm_swap_request`（line 643）的额外校验：

1. 必须 `source.module = "amm"`、`source.action = "swap"`（lines 647-658）—— 常量在 `src/amm.rs:11,12`
2. 用 `AmmSwapRequest::exact_in(...)` 验证 amounts 是非零 LE u128（`src/amm.rs:58-74`、`amount_to_le_bytes` `src/amm.rs:600-607`）；payout lock 必填（`src/amm.rs:113-115`）；`receive_capacity_shannons` 不能是 zero（`src/amm.rs:116-120`）
3. `template_keys == expected_app_keys` 必须严格相等（`cellscript_bridge.rs:894-900`）
4. 拒绝 binding consumes 命中 pool cell（`cellscript_bridge.rs:675-679`）
5. 强制 `reads = binding.reads ∪ {pool_cell}`（`cellscript_bridge.rs:680-682`）

返回的 `IntentBody`（`cellscript_bridge.rs:694-718`）关键改动：

- `action = App { action: "swap", payload: <cellfabric-amm-swap-request-json-v1 bytes> }` —— payload 是 **AMM-native swap request**，不是 CellScript action plan
- `metadata.cellscript_action_plan_hash` 保留原 hash 作 provenance（line 689-692）
- `metadata.cellscript_source_payload_format = "cellscript-action-plan-json-v1"`、`cellscript_payload_format = "cellfabric-amm-swap-request-json-v1"`（line 921-926 在 `cellscript_metadata`，lines 689-692 覆写 source 字段）

这是 bridge docs 自陈的"两 shape"区别（`docs/cellscript-bridge.md:12-19`）：

> - generic CellScript import, which keeps the CellScript action plan JSON as the app payload and requires an external CellScript runtime builder at settlement;
> - AMM swap import, which keeps the CellScript action-plan hash as provenance but converts the app payload into a `cellfabric-amm-swap-request-json-v1` request that can be validated by `AmmSwapPolicy` and compiled by `AmmPoolBatchCompiler`.

#### 3.4.2 `AmmSwapPolicy`：namespace `"amm"` 的 conflict policy（`src/amm.rs:532-590`）

关键校验（`src/amm.rs:555-585`）：

- `intent.body.domain.app_namespace == self.namespace`（line 556-561）
- action 必须为 App 且 `action == "swap"`（line 563-570）
- `AmmSwapRequest::from_payload` 校验（同 §3.4.1）
- intent.resources.reads 必须包含 pool cell（line 573-577）—— 跟 `cellscript_bridge.rs:680-682` 对齐
- intent.resources.consumes 不能包含 pool cell（line 578-582）
- 输出的 `conflict_keys = { amm:pool:<pool_id> }`（line 584）

`allows_app_key_co_selection`（`src/amm.rs:587-589`）返回 `namespace == "amm" && key_type == "pool"` —— 把 AMM pool key 显式标成 batchable，让 `select_conflict_free_bundle_with_app_policies` 把多个同-pool swap 共选进一个 bundle（README.md:236、tutorial §App Policy Examples）。

#### 3.4.3 `AmmPoolBatchCompiler`：namespace `"amm"` 的 settlement compiler（`src/amm.rs:328-523`）

注册形态：`registry.with_compiler(AmmPoolBatchCompiler::new([pool_state])?)`（`examples/cellscript_amm_flow.rs:144`）。

构造（`src/amm.rs:336-373`）：接受显式 `AmmPoolState`（来自 `CellTemplate` + `fee_bps`，`from_cell_template` `src/amm.rs:147-167`），或 `from_pool_templates_with_output_capacity` 同时记录 input/output capacity。

`AmmPoolCellData` schema = `"cell-fabric-amm-pool-v1"`（`src/amm.rs:14,232-277`）；`AmmSwapReceiptData` schema = `"cell-fabric-amm-swap-receipt-v1"`（`src/amm.rs:15,279-325`）。

Batch 编译逻辑（`src/amm.rs:439-522`，`compile_intents`）：

1. 每个 intent 跑 `compile_request`（`src/amm.rs:394-427`）：
   - 校验 namespace + App + action == "swap"
   - `AmmSwapRequest::from_payload`
   - `intent.resources.reads` 必须包含 pool cell，`consumes` 不能包含
   - `intent.resources.app_keys` 必须包含 `amm:pool:<pool_id>`
2. 按 pool cell 分组（`src/amm.rs:443-454`）
3. 每组顺序跑 exact-in 报价 `quote_exact_in`（`src/amm.rs:643-678`）：先扣 fee bps（`fee_denominator - u128::from(fee_bps)`）→ `reserve_out * amount_in_after_fee / (reserve_in + amount_in_after_fee)`；过 `min_receive` 检查
4. 累计 reserve 应用每笔 swap（`src/amm.rs:483-488`，`checked_add` / `checked_sub`）
5. 给每个 swap 生成 receipt 输出 `swap_receipt_output`（`src/amm.rs:698-717`）：
   - `AmmSwapReceiptData` schema = `cell-fabric-amm-swap-receipt-v1`
   - `CellTemplate { capacity_shannons, lock: receive_lock, type_: None, data: Inline(receipt_bytes) }`
   - `template.validate_occupied_capacity()`（`src/amm.rs:716`）
6. 生成 pool 状态输出 `pool_state_output`（`src/amm.rs:680-686` → `to_cell_template_with_reserves` `src/amm.rs:186-201`），再 `validate_occupied_capacity`（`src/amm.rs:199`）
7. 容量守恒：所有 output capacity 之和（pool_output + 所有 receipt）必须 ≤ `pool.input_capacity_shannons`（`src/amm.rs:507-514`）—— 不允许 mint capacity，receipt 必须 fit in spare capacity from consumed pool input
8. Pool cell 出现一次到 inputs，每个 swap 的 receipt 输出一次到 outputs（`src/amm.rs:516-519`）
9. `Ok(AppSettlementFragment::with_inputs(inputs, outputs))`（line 521）

容量守恒的兜底在 `examples/cellscript_amm_flow.rs:158-162`：调用方提供 `TransactionBuildContext::with_input_capacity(pool_cell, known_pool_input_capacity)` 然后 `draft.validate_known_capacity_balance(&build_context)` —— direct compiler 也能拦住 missing input capacity / capacity-imbalanced drafts（README.md:357-362）。

为什么 AMM 是"完整闭环"、通用 CellScript 是"import + 不 compile"：

| 维度 | AMM 路径 | 通用 CellScript 路径 |
| --- | --- | --- |
| Conflict policy | `AmmSwapPolicy`（`amm.rs:532-590`），namespace `"amm"` | `CellScriptAppConflictPolicy`（`cellscript_bridge.rs:362-448`），可挂在任意 namespace |
| Settlement compiler | `AmmPoolBatchCompiler`（`amm.rs:328-523`），namespace `"amm"`，实打实把 swap request 编译成 pool+receipt outputs | `CellScriptRuntimeBuilderCompiler`（`cellscript_bridge.rs:362-398`），固定返回 `UnsupportedSettlementAction` |
| App payload | AMM-native `cellfabric-amm-swap-request-json-v1` bytes | 完整 CellScript `cellscript-action-plan-json-v1` bytes |
| Batch-friendly? | yes（pool key 标 batchable，`allows_app_key_co_selection` `amm.rs:587-589`） | no（policy 不标任何 key 为 batchable） |
| Capacity conservation | yes（input vs output capacity 检查 + occupied-capacity validation） | n/a（compile never runs） |
| 失败模式 | `ConstraintViolation` / `UnsupportedSettlementAction` from compiler | `UnsupportedSettlementAction` from `CellScriptRuntimeBuilderCompiler` |

例：smoke `examples/cellscript_amm_flow.rs` 与 dev HTTP `scripts/cellscript_amm_flow_smoke.sh`（5 行 sh 包裹 `cargo run --quiet --example cellscript_amm_flow`）。

### 3.5 HTTP endpoints（`http` feature）

源码：`src/http.rs`。

```
/cellscript/import              POST  import_cellscript_intent         http.rs:202,265-269
/cellscript/import/amm-swap     POST  import_cellscript_amm_swap       http.rs:204-206,271-275
/intents                        POST  submit_intent                    http.rs:207
/intents/{intent_id}            GET   get_intent                       http.rs:208
/intents/{intent_id}/status     GET   get_intent_status                http.rs:209
/intents/{intent_id}/conflicts  GET   get_conflicts                    http.rs:210
/ledger                         GET   get_ledger                       http.rs:211
/orderer/bundles                POST  build_bundle                     http.rs:212
/orderer/bundles/{bundle_id}/receipts        GET   get_signed_receipts_for_bundle    http.rs:214-216
/orderer/bundles/{bundle_id}/availability     GET   get_bundle_data_availability      http.rs:217-220
/orderer/bundles/{bundle_id}/confidence      GET   get_soft_confirmation_confidence  http.rs:221-224
/orderer/orderers/{orderer_id}/receipts      GET   get_signed_receipts_for_orderer   http.rs:225-228
/orderer/soft-confirm                  POST   soft_confirm                          http.rs:229
/orderer/soft-confirm-signed           POST   soft_confirm_signed                   http.rs:231-233
/orderer/build-and-soft-confirm        POST   build_and_soft_confirm                http.rs:234-237
/orderer/build-and-sign-soft-confirm   POST   build_and_sign_soft_confirm           http.rs:238-241
/proofs/conflict                  POST  build_conflict_proof                http.rs:242
/proofs/dispute                   POST  build_dispute_proof                 http.rs:243
/proofs/receipt-equivocation      POST  build_receipt_equivocation_proofs   http.rs:244-247
/proofs/exit                      POST  build_exit_claim                     http.rs:248
/settlement/submit                POST  submit_settlement_plan               http.rs:249
/settlement/submit-with-recovery  POST  submit_settlement_plan_with_recovery http.rs:250-253
/settlement/record-observation    POST  record_settlement_observation       http.rs:254-257
/health                           GET   health                                http.rs:201,261-263
```

对比 README.md 247-249 self-claim 表里列了 `/health` + 部分 gateway/orderer routes，但 cellscript 的两个 import 路由只在 `src/http.rs:202-206` 与 `docs/cellscript-bridge.md:125-127,162-164` 提到 —— `docs/principles-tutorial.md:352-369` 的 routes 表完全省略了 `/cellscript/*` 的两行。这是文档遗漏（不算自相矛盾），建议综合时记得加进 routes 总账。

CellScript import 路由签名（`src/http.rs:265-275`）：

```rust
async fn import_cellscript_intent(Json(req): Json<CellScriptIntentImportWireRequest>)
    -> HttpResult<CellScriptIntentImportResponse>
async fn import_cellscript_amm_swap(Json(req): Json<CellScriptAmmSwapImportWireRequest>)
    -> HttpResult<CellScriptIntentImportResponse>
```

Wire 形接受 typed 与 hex 两种 envelope（`cellscript_bridge.rs:206-236,82-122`）；hex 形态把 `tx_hash_hex` / `code_hash_hex` / `args_hex` 转成 typed `OutPointRef` / `ScriptSpec`。HTTP 错误通过 `HttpIntentError` → status code mapping（`src/http.rs:165-191`）：`AppConflict` / `DuplicateAppConflictPolicy` / `BundleIncludesSupersededIntent` 走 409，其他走 400。

---

## 4. CellFabric 的 scope 边界（red lines）

完整源：`docs/red-lines.md`（223 行）。6 条 + 6 个 merge-check question。摘要：

| Red line | 限制 | 关系到 bridge 的具体含义 |
| --- | --- | --- |
| 1 — soft confirmation is never finality | orderer 可以软确认但不拥有 finality；`non_final=true` 强制 | BundleReceipt / soft confirmation 永远非 final；bridge 给出 unsigned body，client 必须自签后 `POST /intents`（`docs/cellscript-bridge.md:216-217`） |
| 2 — auth binding ≠ production signature verification | 必须 strict `AuthVerifierRegistry`；`NoopAuthVerifier` 不是安全件 | bridge 默认走 `auth_mode = CoSignConcreteTx`（`cellscript_bridge.rs:624`）—— 留出给 signed-intent 阶段走真正的 secp256k1 verifier |
| 3 — conflict domains 必须在 signed IntentBody 中显式存在 | 不允许 post-hoc patch；不允许编译器 emit signed resources 之外的 inputs | import 期就强制把 `app_keys` 拷进 `ResourceAccess.app_keys`（`cellscript_bridge.rs:614`），并要求 envelope 的 `cellfabric_intent_template.resources.app_keys == resource_access_template.app_conflict_key_templates`（`cellscript_bridge.rs:951-965`） |
| 4 — 编译器不读 live mutable DAG | immutable bundle snapshot 上的纯函数 | `CellScriptRuntimeBuilderCompiler::compile_intent`（`cellscript_bridge.rs:390-397`）除了 `validate_cellscript_intent_payload` 外不接触任何 mutable 状态；如果 replace 它，需要继续满足这规则 |
| 5 — proof skeletons 不是 on-chain enforceability | ConflictProof/DisputeProof/ExitClaim 当前只是 off-chain evidence | bridge 不参与 dispute/exit，但 smoke flow 不会假装存在 on-chain enforcement |
| 6 — 不能把"结构性 DAG 启发"变成 protocol identity | CKB 独占 finality，orderer 不能成 consensus actor | bridge 不会接 soft-confirmation-of-Cellscript-action（envelope `bridge_boundary.not_a_soft_confirmation = true` 强校验，`cellscript_bridge.rs:489-493`） |

合并前 6 问（`red-lines.md:214-222`）是 `(non-final-as-final / unauthorized-auth / implicit-conflict / live-mutation / off-chain-as-on-chain / orderer-as-consensus)` —— 比对 CellScript bridge：

- (1) bridge `not_l1_finality: true` —— ✓
- (2) bridge 用 `CoSignConcreteTx` auth_mode，把真签交给后续 `/intents` 流程 —— ✓
- (3) bridge import 期就冻结构、要求两份 app_keys 一致 —— ✓
- (4) bridge 编译器实现是 pure （现在 fail-closed 占位）—— ✓
- (5) bridge 不声称自己出 on-chain enforcement —— ✓
- (6) bridge 不让 CellScript action 直接做软确认 —— ✓

> CellFabric 把 bridge 当成 **JSON schema contract**，不参与 CellScript 的"apps are 1st-class intent sources"叙事。

跟 CellScript 0.14 scope audit 的可比性：CellScript 0.14 用 "边界设计"（registry / Tooling release 之外不做）；CellFabric red-lines 用 "merge blocker"，在动机层面同源，但 CellFabric 把"违反就视为 protocol safety regression"（`red-lines.md:5-9`）明说成合并门，比 CellScript "本 release 不在 scope"更硬。

---

## 5. CellFabric 跟 Obyte 的关系

直接来源：`README.md:61-84`、`docs/principles-tutorial.md:43-52`、`docs/development-roadmap.md:113-119`。

**CellFabric 借鉴 Obyte 的**：

- "Block-free fast-path organization"：upper-layer activity 作为 transaction/intent 的图而非 mini-blocks（`README.md:62-66`）
- "Graph observability"：frontier / lineage / conflicts / confidence 不隐藏 settlement pending 状态（`README.md:72-73`、`docs/development-roadmap.md:114-118`）
- 一个公开 DAG 让用户/服务能查 ancestry、references、later approvals、ordering landmarks（`docs/development-roadmap.md:113-118`）

**CellFabric 故意不借鉴 Obyte 的**：

- 把 DAG 当成独立 ledger 的 worldview（`README.md:75-84`）
- Orderer 不能成 consensus actor
- Soft confirmation 不能变成 social / de facto finality
- "generic DAG elegance 覆盖 CKB cell 纪律"是禁止的（`README.md:80-83`）

**对比 CellScript → CellFabric 借鉴 CellScript 编译产物**：

不是同一种"借鉴不依赖"。CellFabric → CellScript 是 JSON schema bridge（`cellscript_core_dependency = no-cell-fabric-rust-crate`，`cellscript_bridge.rs:495-499`）：借用结构 + envelope 字段语义，不绑 Rust crate 版本。同样地，CellFabric → Obyte 是"组织启发"：借用图结构，不绑任何 Obyte 代码或协议 ID。两者都是 CellFabric 自我约束成"只 borrow 结构、不 borrow 协议身份"的同一个 red line（red-lines.md:181-205）的具体表现。

注意一个容易混淆的点：CellFabric README §"External Reference Boundary: Obyte"（`README.md:61-84`）里 Obyte 名字出现，docs 里 RGB++ 是另一对照表（`docs/development-roadmap.md:380-393`）。Obyte 是 "fast path 启发"，RGB++ 是 "Bitcoin↔CKB 双层 asset binding 替代方案"——属于不同决策维，没冲突。

---

## 6. CellFabric 跟 Myelin 的关系

> 任务说明要求"在 Myelin 仓库和 CellFabric 仓库两边都搜 Myelin"——结果如下

**CellFabric 这一边**（`rg -l "Myelin"` / `rg -i "myelin"` 跨整个 repo）：

- 命中：**0**（所有扩展名、所有分支）
- CellFabric 的 `README.md`、`docs/*.md`、`src/**/*.rs`、`examples/**.rs`、`scripts/**.sh`、`Cargo.toml`、`AGENTS.md` 都没有"Myelin"字样
- AGENTS.md 仅 32 行，是上游的通用 CKB 工作守则（与 CellFabric 项目无关），无 Myelin 引用（`AGENTS.md:1-32`）

**Myelin 这一边**（`rg -l "Myelin"` 跨整个 Myelin repo）：

- 命中：`MYELIN_PRODUCTION_REHEARSAL_REPORT.md`、`MYELIN_CONSENSUS_COMPLETENESS.md`、`MYELIN_SWARM_AUDIT_WHOLEREPO.md`、`MYELIN_CKB_SEMANTIC_DEVIATIONS.md`、`MYELIN_WEBSITE_AUDIT.md`、`mempool/src/cellpool.rs`、`mempool/src/scorer.rs`、`mempool/src/lib.rs`、`mempool/README.md`、`MYELIN_SESSION_L2_PLAN.md` 等（`Myelin` 字面匹配 ≥ 10 个文件）
- **注意**：Myelin 仓库内嵌一个 `cellscript/` 子目录（不是 CellFabric），里面有大量提到 `CellFabric` 的内容（`Myelin/cellscript/docs/README.md:117`、`Myelin/cellscript/scripts/cellscript_ckb_release_gate.sh:67,99,104,391`、`Myelin/cellscript/tests/cli.rs:5917,5963-5982`、`Myelin/cellscript/docs/releases/CELLSCRIPT_0_19_RELEASE_NOTES.internal.md:23,103` 等）—— 这些是 CellScript 项目自己对 CellFabric 的引用，**不是 Myelin 跟 CellFabric 的直接连线**

**结论（按现状）**：

- CellFabric 跟 Myelin **没有现存交叉**。零直接引用、零共享 schema、零共享 crate dependency（CellFabric `Cargo.toml:26-37` 的 dependencies：`ckb-crypto 1.1.0`、`ckb-hash 1.1.0`、`ckb-types 1.1.0`、`hex 0.4`、`serde 1.x`、`tokio 1.35`、`thiserror 1.x` + 两个 optional；无 Myelin / 无 mempool / 无 consensus）
- Myelin 仓库内嵌 CellScript（vendored 或 nested project），CellScript 走 JSON bridge 接触 CellFabric —— 这条线 **绕开了 Myelin**
- 这意味着 Myelin 若要跟 CellFabric 配合，要么继续走 JSON bridge（与 CellScript 同形状），要么需自己实现 `AppSettlementCompiler`（§3.3 那个缺口的形状），要么两条都不走、把 Myelin 留在 CellFabric 视野外

CellFabric docs 没有"我们应该跟 Myelin 协作"的任何表述。也没有任何"待办 / 关注 / 已知问题"段落提到 Myelin。

---

## 7. CellFabric 的对外公共面

### 7.1 Crate

| 字段 | 值 | 来源 |
| --- | --- | --- |
| name | `cell-fabric` | `Cargo.toml:2` |
| version | `0.1.1` | `Cargo.toml:3` |
| description | "CellFabric CKB-settled cell intent core types and deterministic ordering primitives" | `Cargo.toml:7` |
| edition | 2024 | `Cargo.toml:5` |
| rust-version | 1.92.0 | `Cargo.toml:10` |
| default features | （无） | `Cargo.toml:13` |
| optional features | `http`、`ckb-rpc-submit` | `Cargo.toml:14-15` |
| deps | `ckb-crypto 1.1.0`、`ckb-hash 1.1.0` (no-default)、`ckb-types 1.1.0`、`hex 0.4`、`serde 1.x`、`serde_json 1.0`、`thiserror 1.0.22`、`tokio 1.35` (`rt/sync/time`)、`async-trait 0.1` | `Cargo.toml:26-38` |
| optional deps | `axum 0.8` (http)、`ckb-jsonrpc-types 1.1.1` + `reqwest 0.12 { json }` (ckb-rpc-submit) | `Cargo.toml:28,29,34` |

### 7.2 Binary

| Binary | path | feature gate | 来源 |
| --- | --- | --- | --- |
| `cell-fabric` | `src/bin/cell-fabric.rs` （927 行） | default | `Cargo.toml:23-24`、`README.md:470-497` |
| `cell-fabric-http` | `src/bin/cell-fabric-http.rs`（238 行） | `http` | `Cargo.toml:18-20`、`src/bin/cell-fabric-http.rs` |

CLI `cell-fabric` 提供 `submit-transfer`、`submit-launchpad-allocation`、`order-once`、`settle-bundle`、`observe-bundle`、`status`、`bundle`、`conflicts`、`graph`（`README.md:472-497`、`src/bin/cell-fabric.rs:34-40`）。

`cell-fabric-http` 默认 127.0.0.1:8118、MemoryIntentStore + DryRunCkbSubmitter（`src/bin/cell-fabric-http.rs:12-46`、README.md:481-505）；可通过环境变量切到 `FileIntentStore` / `JournalIntentStore`，并配置 orderer signer（README.md:486-500）。

### 7.3 Library 的公开类型（顶层 re-export）

`src/lib.rs:45-171` 给出了稳定的对外接口表。摘录与 bridge / upper-layer 关注点相关的：

- bridge：`cellscript_bridge::` 全集（`lib.rs:67-82`）—— 含 `CellScriptIntentImportRequest`、`CellScriptAmmSwapImportRequest`、`CellScriptAppConflictPolicy`、`CellScriptRuntimeBuilderCompiler`、所有 `import_cellscript_*` 函数、所有 wire / hex / typed 形
- AMM：`amm::` 全集（`lib.rs:46-51`）—— `AmmPoolBatchCompiler`、`AmmPoolState`、`AmmSwapPolicy`、`AmmSwapRequest`、`AmmPoolCellData`、`AmmSwapReceiptData`、常量 `AMM_NAMESPACE`、`AMM_SWAP_ACTION`、`AMM_POOL_KEY_TYPE`、`AMM_POOL_STATE_SCHEMA`、`AMM_SWAP_RECEIPT_SCHEMA`、`AMM_SWAP_REQUEST_PAYLOAD_FORMAT`、`AMM_DEFAULT_RECEIVE_CAPACITY_SHANNONS`
- launchpad：`launchpad::`（`lib.rs:111-114`）—— `LaunchpadAllocationPolicy`、`LaunchpadAllocationRequest`、`LAUNCHPAD_NAMESPACE`、`LAUNCHPAD_ALLOCATE_ACTION`（**注意**：launchpad 只有 policy，没有 compiler，与 CellScript 通用路径同位置属于 "import + 不 compile"，但被 README.md:160-175 / 209-210 自陈为"intentionally a conflict-policy example, not a settlement compiler"）
- App compiler 侧：`AppSettlementCompiler` / `AppSettlementCompilerRegistry` / `AppSettlementFragment`（`lib.rs:52-54`）
- Engine / actor：`IntentEngine`、`ActorIntentGateway`、`ActorIntentOrderer`、`IntentEngineHandle`、`spawn_intent_engine_actor`（`lib.rs:90,101-104,117-120,45`）
- 顶层类型在 `lib.rs:160`：`pub use types::*;`（含 `IntentBody`、`SignedIntent`、`IntentAction`、`IntentDomain`、`ResourceAccess`、`AppConflictKey`、`OutPointRef`、`ScriptSpec`、`CellTemplate`、`IntentAuth`、`SettlementAuthMode`…）

### 7.4 HTTP routes（来自 §3.5）

共 24 条（`src/http.rs:196-259`）。两条 CellScript import 是其中特殊两条；其他都是 gateway / orderer / proofs / settlement / health 的常规 ops。

### 7.5 JSON schema 常量（write-site）

| Schema / format 常量 | 值 | 来源 |
| --- | --- | --- |
| Bridge envelope schema | `cellscript-cellfabric-intent-envelope-v0.20` | `cellscript_bridge.rs:15` |
| Bridge envelope status | `requires-runtime-binding` | `cellscript_bridge.rs:16` |
| Bridge action plan format | `cellscript-action-plan-json-v1` | `cellscript_bridge.rs:530,580`; defaults to this in summary (`cellscript_bridge.rs:339`) |
| Bridge auth mode | `CoSignConcreteTx` | `cellscript_bridge.rs:540,891` |
| Bridge "core dependency" | `no-cell-fabric-rust-crate` | `cellscript_bridge.rs:497-499,822-824` |
| AMM swap request payload format | `cellfabric-amm-swap-request-json-v1` | `src/amm.rs:16`; bridge reads at `cellscript_bridge.rs:686,687` |
| AMM pool state schema | `cell-fabric-amm-pool-v1` | `src/amm.rs:14` |
| AMM swap receipt schema | `cell-fabric-amm-swap-receipt-v1` | `src/amm.rs:15` |
| AMM namespace | `amm` | `src/amm.rs:11` |
| AMM swap action | `swap` | `src/amm.rs:12` |
| AMM pool key type | `pool` | `src/amm.rs:13` |
| AMM default receive capacity | `30_000_000_000` shannons | `src/amm.rs:17` |
| Launchpad namespace / action | `launchpad` / `allocate` | `src/launchpad.rs:7-8` |
| Receipt signing kind | secp256k1 | `src/receipt_auth.rs` (README.md:181) |

### 7.6 Smoke flow example（dev-only 边界检查）

| 命令 | 验证什么 | 来源 |
| --- | --- | --- |
| `cargo run --example cellscript_import -- <envelope.json> <lock-hash> <nonce>` | 一次 generic import，输出 `summary` 或 full response，**不签、不提交** | `examples/cellscript_import.rs:1-61`、`docs/cellscript-bridge.md:222-231` |
| `cargo run --example cellscript_import -- --summary-only ...` | CI-friendly 单行 summary | `docs/cellscript-bridge.md:236-242`、`examples/cellscript_import.rs:18-21` |
| `cargo run --example cellscript_flow -- ...` | import + dummy sign + 提交 gateway + `ActorIntentOrderer::build_and_soft_confirm` + 验证 `DirectSettlementCompiler::compile_bundle` 命中 `UnsupportedSettlementAction` | `examples/cellscript_flow.rs:50-150`、`docs/cellscript-bridge.md:245-258` |
| `cargo run --example cellscript_amm_flow -- [--summary-only]` | import 两笔同-pool swap + 通过 `AmmPoolBatchCompiler` 生成 pool output + receipt outputs + 走 `DirectSettlementCompiler::compile_bundle` | `examples/cellscript_amm_flow.rs`、`docs/cellscript-bridge.md:264-280` |
| `scripts/cellscript_amm_flow_smoke.sh` | 上面那条的 sh wrapper | `scripts/cellscript_amm_flow_smoke.sh:1-5` |
| 跨仓库：`cellscript/scripts/cellscript_cellfabric_bridge_smoke.sh` | CellScript 端生成 envelope 并跑 flow example | `docs/cellscript-bridge.md:260-263` |
| `cargo test -p cell-fabric` | 默认 crate 集成测试（含 `src/tests.rs` 10+ bridge 测试：lines 666, 736, 776, 819, 888, 940, 985, 1020, 1291, 1313, 1334） | `README.md:448-452`、`src/tests.rs` |
| `cargo test --features ckb-rpc-submit --test devnet_smoke` | 接 devnet CKB 的 smoke（仅在 `CELL_FABRIC_DEVNET_RPC` 已设置时跑） | `docs/principles-tutorial.md:393-400` |

---

## 8. 配合面观察（从 CellFabric 视角看 CellScript — 给 owner 综合用）

> 仅记录现状，**不**写"v1/v2/改进版"叙事，**不**给 CellFabric 上游提建议。
> 每条都附带 file:line 证据。

### 8.1 CellFabric 对 CellScript 的依赖边界是"schema + provenance"，不是代码

- CellFabric 不在 `Cargo.toml` 引 CellScript 任何 crate
- bridge schema 是字符串（`cellscript-cellfabric-intent-envelope-v0.20`，`cellscript_bridge.rs:15`）
- 故意保留 `cellscript_core_dependency = "no-cell-fabric-rust-crate"`（`cellscript_bridge.rs:497-499`）反方向也是同一选择：CellScript 不绑 CellFabric 版本

### 8.2 CellFabric 把"已实现什么 / 没实现什么"放在 namespace 边界里

- AMM namespace 配齐 policy + compiler + batch hook + 容量守恒 + 占位容量校验（§3.4）
- 通用 CellScript namespace 只配齐 import + policy；compiler 是 `UnsupportedSettlementAction` 占位（§3.3）
- launchpad namespace 在 README 自陈为"intentionally a conflict-policy example, not a settlement compiler"（README.md:160-175）。这跟 CellScript 通用路径是同一种"配齐一半"形态

### 8.3 从 CellFabric 角度看 CellScript 现在的 actor flow

`examples/cellscript_flow.rs:45-151` 已经把"完整 closed loop（非 final）"的 actor chain 跑通了：import → app policy 注册 → gateway (with `with_required_app_policy_for_app_actions(true)`) submit → orderer (with `with_required_bundle_validation(true)`) build_and_soft_confirm → `DirectSettlementCompiler` 跑批 → 命中 `UnsupportedSettlementAction` → 报回"`requires_external_cellscript_runtime_builder: true`"。这是 cellfabric-side **已经具备的**协议边界验证。

具体证据：

- `examples/cellscript_flow.rs:65-67` gateway 走 `with_required_app_policy_for_app_actions(true)` + `with_app_policies(...)`
- `examples/cellscript_flow.rs:71-74` orderer 走 `with_required_bundle_validation(true)` + `with_app_policies(...)`
- `examples/cellscript_flow.rs:58-59` 把 `CellScriptRuntimeBuilderCompiler::from_import(&imported)` 注册到 compiler registry
- `examples/cellscript_flow.rs:103-112` 跑 `direct_compiler.compile_bundle(&confirmed.bundle)`，然后 mapping 到 `requires_external_cellscript_runtime_builder`

这条 flow 是 **CellFabric 自己就具备的**自我边界验证，并不是协议上的开放合约。

### 8.4 从 CellFabric 角度看 CellScript 的关键不一致 / 文档遗漏

- `docs/principles-tutorial.md:352-369` routes 表里没列 `/cellscript/import` 和 `/cellscript/import/amm-swap`，但 `src/http.rs:202-206` 和 `docs/cellscript-bridge.md:125-127,162-164` 都明确有

- 任务说明书里说 `src/cellscript_amm.rs` 存在 —— **不存在**；AMM-specific import 在 `src/cellscript_bridge.rs:643-759`，与通用 import 同文件。综合任务里如果引用了"CellFabric 的 AMM-specific 文件"，需要换成 `cellscript_bridge.rs` 的 import_cellscript_amm_swap_* 段。

- CellFabric `examples/` 同时存在 `cellscript_flow.rs`（import-and-stop-at-runtime-boundary）与 `cellscript_amm_flow.rs`（完成 pool output + receipt outputs）。两条 flow 显示 CellFabric 已经在 dev-only 层面把"完整闭环 vs 卡在 boundary"的差别演示出来了。

### 8.5 给 owner 综合时 CellFabric 提供的"形状"

1. 一组不可绕过的入参 schema（`cellscript_bridge.rs:482-552`）
2. 一个 explicit-fail-closed 的 compiler 占位（`cellscript_bridge.rs:385-398` + §3.3 表格）
3. 一套 namespace-scoped policy + compiler registry（`src/policy.rs`、`src/app_compiler.rs`）
4. 一份 owner 可以照抄的注册样例（`examples/cellscript_flow.rs`、`examples/cellscript_amm_flow.rs`）
5. 4 组 red-line（`docs/red-lines.md`）+ 21 条 inventory invariant（`README.md:228-289`）+ 21 条安全 invariant（`docs/principles-tutorial.md:508-531`）—— CellFabric 自陈的可 merge 行为边界
6. 一个 commit-versioned schema（`cellscript-cellfabric-intent-envelope-v0.20`，`cellscript_bridge.rs:15`）—— 这是 CellScript 必须严格对齐的常量；任何不匹配都会在 bridge 入口就 fail

### 8.6 几条值得 owner 留意的"小信号"

- CellFabric `IntentEngine::with_store_and_app_policies(...)` 在 `examples/cellscript_amm_flow.rs:87-89` 被直接构造；`examples/cellscript_flow.rs:62` 则默认使用 `IntentEngine::in_memory()`。两条 flow 的 engine 构造方式不同 —— flow example 走 `in_memory`，AMM flow 显式塞 `MemoryIntentStore + app policies`，目的是让 `compile_intents` 拿到 policy。CellFabric 没把这两种用法合并成单一 helper，service-side 如果想跑同样的逻辑需要自己写类似 `examples/cellscript_amm_flow.rs` 的 setup。

- `examples/cellscript_amm_flow.rs:131-132` 用 `AMM_DEFAULT_RECEIVE_CAPACITY_SHANNONS * 3 + 1_000` 算"known input capacity" —— 这是 dev smoke 选择的一个具体数字，并不是协议级别的占位常量；生产 AMM service 需要在 live CKB 读出真实 input capacity（README.md:357-362 + docs/development-roadmap.md "Production work still needs live CKB pool reads"）。

- `cellscript_bridge.rs:692` `cellscript_source_payload_format = "cellscript-action-plan-json-v1"` 加上 `metadata.cellscript_payload_format` 在 `cellscript_metadata` 中由调用方写入的具体格式（AMM 路径下就是 `cellfabric-amm-swap-request-json-v1`，`cellscript_bridge.rs:686`）。两者并存，是为了让审计者既能看"原始 source 是什么"也能看"bridge 把什么带过去了"。

- 任务说明里给的 `cellscript_action_plan_hash` 用 32-byte hex string 表达（`cellscript_bridge.rs:454-459`），而 `metadata.cellscript_payload_format` 用 string 表达 —— 两个常量的 wire 形态在 envelope 里都是 `string`（来自 `examples/cellscript_amm_flow.rs:263,278`）。`source.action_plan_hash` 是 hex string，不是 `[u8;32]` 字节。综合时记得 hex 边界。

---

### 引用一致性自检

| 引用 | 文件 | 行 | 备注 |
| --- | --- | --- | --- |
| CellFabric 的 `Cargo.toml` | `Cargo.toml` | 1-38 | crate name `cell-fabric`、version 0.1.1 |
| `cellscript_bridge.rs` 顶部常量 | `src/cellscript_bridge.rs` | 15-16 | schema + status |
| `import_cellscript_intent_envelope` | `src/cellscript_bridge.rs` | 478-634 | 含 9 段校验 |
| `CellScriptAppConflictPolicy` | `src/cellscript_bridge.rs` | 362-448 | policy registry 结构 |
| `CellScriptRuntimeBuilderCompiler` | `src/cellscript_bridge.rs` | 385-398 | fail-closed 占位 |
| `import_cellscript_amm_swap_request` | `src/cellscript_bridge.rs` | 643-729 | AMM-specific shape |
| `AmmSwapPolicy` | `src/amm.rs` | 532-590 | conflict policy |
| `AmmPoolBatchCompiler` | `src/amm.rs` | 328-523 | batch compiler |
| `AmmPoolBatchCompiler::compile_intents` | `src/amm.rs` | 439-522 | batch 编译主逻辑 |
| HTTP routes 主表 | `src/http.rs` | 196-259 | 24 条 route |
| `import_cellscript_intent` HTTP handler | `src/http.rs` | 265-269 | typed/hex wire 两种 |
| `import_cellscript_amm_swap` HTTP handler | `src/http.rs` | 271-275 | typed/hex wire 两种 |
| Red lines 列表 | `docs/red-lines.md` | 1-223 | 6 条 + 6 question |
| Safety invariants (inventory) | `README.md` | 226-289 | 30 条 |
| Safety invariants (tutorial 表) | `docs/principles-tutorial.md` | 506-531 | 21 条 |
| Smoke flow `cellscript_flow` | `examples/cellscript_flow.rs` | 45-151 | boundary demo |
| Smoke flow `cellscript_amm_flow` | `examples/cellscript_amm_flow.rs` | 43-222 | complete loop demo |
| Smoke shell wrapper | `scripts/cellscript_amm_flow_smoke.sh` | 1-5 | sh 跑 example |
| 跨 repo Myelin 检查 | `CellFabric/`、`Myelin/` | n/a | CellFabric 0 命中；Myelin 命中在自有 `cellscript/` 子树 + mempool/`*` |

---

> 以下小节仅作"综合视角"提示，没有对 CellFabric 的评价：
> - CellFabric 把"CellScript bridge 的 settlement path 是 placeholder"明写进了源码（`cellscript_bridge.rs:390-397`）+ docs（`docs/cellscript-bridge.md:111-119`）+ smoke（`examples/cellscript_flow.rs:103-112`）。**完整闭环的实现在 AMM 那一支**，并明确说"通用 CellScript 路径不可能在 CellFabric core 内有 compiler"（`red-lines.md` RL3 + `tutorial.md:259-271`）。
> - 这条 placeholder 是 CellFabric 自报的"scope 边界"，不是 owner 加上去的限制。
