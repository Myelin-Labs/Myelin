# CellScript 侧 — 自述与对 CellFabric 的现有配合面

> **侧写范围**：从 CellScript 主仓库 `/Users/arthur/RustroverProjects/CellScript`（即
> 头指针 `0.21.0-rc.1`，即 `Cargo.toml:19`）视角，观察它如何理解"自己 vs CellFabric"、
> 现有 CellFabric 配合面的 CellScript 端实现细节，以及它与 Myelin 仓库的关系。
> 配套引用：Myelin 仓库将其 vendored CellScript 副本放在
> `/Users/arthur/RustroverProjects/Myelin/cellscript/`（头指针 `0.20.0-rc.2`，见
> `/Users/arthur/RustroverProjects/Myelin/cellscript/Cargo.toml:19`）。
>
> **本报告是 owner 内部综合材料**，并非给 CellScript 上游的 PR，所以所有结论按"现状"
> 叙述，不读"v1/v2"或"想要"等版本叙事。CellScript 还在 alpha（详见
> `/Users/arthur/RustroverProjects/CellScript/README.md:52-66` 中"alpha / stabilisation
> phase"措辞）。

---

## 1. CellScript 是什么

CellScript 是一个**面向 CKB 的 Cell 模型 DSL 与编译器**。它把 `.cell` 源码
lower 成 ckb-vm RISC-V 汇编或 ELF artifact，并把 Cell 效果、调度提示、schema、Source-hash、
verifier obligation 等作为机器可读的 metadata sidecar（`.elf.meta.json` / `.s.meta.json`）
一并发出。

- 它**不是 VM**，不会引入新的执行环境；目标执行体是 ckb-vm（README:25-29）。
- 它**不是 wallet / indexer / orderer / registry**，编译产物只到"合规的 metadata +
  artifact"为止；链上 acceptance 由外部节点证据承担（README:54-66；
  `docs/CELLSCRIPT_CKB_ADAPTER.md:13-45`）。
- 它**不实现"自动 transaction 生成"**：源代码里讲过 0.14 故意把 Action Builder /
  CellFabric / CCC integration 列为非范围（`docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184`）。
- 当前**唯一支持的 target profile 是 `ckb`**（`src/lib.rs:126-137`）。其它 profile 名
  fail closed（`src/lib.rs:248-251`： `"unsupported target profile '{}'; supported
  profile: ckb"`）。

Layer 一句话：编译器核心 + CKB profile + Type/Lock Script 两种 entry（`action` / `lock`）
+ 编译期 metadata sidecar + 一个**显式隔离的 CKB adapter crate**（用
  `ckb-sdk-rust` 5.x 把 compiler 出来的 semantic plan realize 成 packed CKB transaction）
+ **JSON-only CellFabric bridge**（`no-cell-fabric-rust-crate`）。

---

## 2. CellScript → CKB 的编译路径

### 2.1 五段编译流水线

`README.md:457-516` 与 `src/lib.rs:6-40`（module map）一致给出五段流水线：

```text
source (.cell) → lexer/ → parser/ → types/ + flow/ + proof_plan/ → ir/ + optimize/ → codegen/
```

- **Lex/Parser**：`src/lexer/`, `src/parser/`, `src/ast/`
- **类型 + 状态转换检查**：`src/types/`, `src/flow/`, `src/proof_plan/`
- **IR 降级**：`src/ir/`（动作 lowering 后输出 `IrConsume` / `IrCreate` / `IrReadRef` /
  `IrDestroy` 等显式 IR 指令）
- **RISC-V codegen**：`src/codegen/`（Tier-1 mnemonic 强制由 internal assembler 拥有，
  见 `AGENTS.md:170-173`），输出 `.s`（asm）或 `.elf`（默认 raw RV64）
- **Standalone 工具**：`lsp/`, `wasm/`, `cli/`, `repl.rs`, `mcp_main.rs`

每段都同时发射 metadata（`RuntimeMetadata` / `ConstraintsMetadata` / `CkbConstraintsMetadata`
等，定义见 `src/lib.rs:778-803`、`src/lib.rs:528-770`）。

### 2.2 artifact 边界

- `ArtifactFormat` 只两个：`RiscvAssembly` (`.s`) / `RiscvElf` (`.elf`)
  （`src/lib.rs:307-313`）。
- `TargetProfile::Ckb` 是**唯一实现**的 profile；其他 profile 名会立即报错
  （`src/lib.rs:248-251`）。
- CKB profile ABI contract 是显式结构化的，profile 元数据中的所有 ABI 字段
  （`witness_abi`, `lock_args_abi`, `source_encoding`, `spawn_ipc_abi`, `since_abi`,
  `cell_dep_abi`, `script_ref_abi`, `output_data_abi`, `capacity_floor_abi`,
  `type_id_abi`, `hash_type_policy`, `dep_group_manifest` 等）都有具体字符串字面量
  （`src/lib.rs:276-303`）。
- VM ABI trailer（`CSABITR0` magic + 版本）在 `src/lib.rs:863-909` 中实现，可被
  `strip_vm_abi_trailer` 剥除以直接送入 ckb-vm（`src/lib.rs:867-874`）。
- ELF 装载，`src/lib.rs:1066-1075`（`ckb_blake2b256` 用 CKB default personalization
  `b"ckb-default-hash"`）。
- ckb-vm SYS-ABI 通过 `src/codegen/runtime.rs`（codegen 内部）生成
  `ckb_load_cell_data` / `ckb_load_witness` / `ckb_load_header_by_field` /
  `ckb_load_input_by_field` / `secp256k1_verify` / `load_ecdsa_signature_hash`
  等 wrapper（README:509-516）。

### 2.3 metadata 怎么 emit

metadata 是**单一 JSON sidecar**——`*.elf.meta.json` / `*.s.meta.json`
（README:519-531）。顶层 schema：

- `metadata_schema_version = 44`（`src/lib.rs:214`，`src/main.rs`/runtime
  通过 `METADATA_SCHEMA_VERSION` 序列化时直接写出当前值）。
- 子版本：`source_metadata_schema_version`, `artifact_metadata_schema_version`,
  `constraints_metadata_schema_version` 都是 `1`（`src/lib.rs:215-217`）。这层 partition
  是 0.20 release 的关键（`docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md:67-79`）。
- `target_profile` 字段是**结构化 ABI contract**：`witness_abi`,
  `source_encoding`, `type_id_abi`, `spawn_ipc_abi`, `since_abi`, `cell_dep_abi`,
  `script_ref_abi`, `output_data_abi`, `capacity_floor_abi`, `lock_args_abi` 都
  有显式字符串值（`src/lib.rs:819-851`，`src/lib.rs:247-303`）。
- `constraints.ckb.*`（cycles, tx_size, occupied_capacity, hash_type_policy,
  declared_capacity_floors, declared_cell_deps）按表 `CkbConstraintsMetadata` 写出
  （`src/lib.rs:611-770`）。
- `runtime.*`：`ckb_runtime_required`, `ckb_runtime_accesses`, `verifier_obligations`,
  `proof_plan`, `builder_assumptions`, `pool_primitives`（`src/lib.rs:778-803`）。
- `metadata.runtime.vm_abi` 必须满足两个硬约束：`format = "molecule"` 且
  `version = 0x8001`（`src/lib.rs:949-960`，与 `MOLECULE_VM_ABI_VERSION` 同步）。

metadata validate 阶段会逐字段校验 target_profile ABI 与 compiler 实际 emit 的 ABI
一致，任何 drift 都会 fail closed（`src/lib.rs:1003-1050`，
`src/lib.rs:1003-1048`）。

### 2.4 metadata → CellDep / deployment 映射

- Compiler 在编译期不知道 live cell；它只在 `target_profile.script_references`
  里声明 Script Reference **义务**（`README.md:90-99`，`src/lib.rs:680-690`），
  并在 metadata 中暴露 `runtime_input_requirements`、`runtime_accesses`、
  `transactions_runtime_input_requirements`（`src/lib.rs:678-803`）。
- 真实 `CellDep` 的 `out_point`（`tx_hash`, `index`）由 **`cellscript-ckb-adapter`**
  或外部 runtime 在 deploy 后从 `DeploymentManifest` 取回
  （`crates/cellscript-ckb-adapter/src/lib.rs:18-25` —
  `ACTION_PLAN_POLICY = "cellscript-action-builder-plan-v1"`,
  `DEPLOYMENT_MANIFEST_SCHEMA = "cellscript-ckb-deployment-manifest-v0.19"`）。
- Adapter 是 `cellscript-ckb-adapter` 这一个 crate 单独做，文档
  `docs/CELLSCRIPT_CKB_ADAPTER.md:1-45` 给出层级表：
  - `cellc` 编译期只做 artifact + metadata + ABI + action build plan + entry witness +
    deploy plan，**不知道 SDK**
  - `cellscript-ckb-adapter` 拿编译器产物，做 materialize + sign + balance + accept
    + submit 桥
  - `ckb-sdk-rust` 5.x 持有 packed 类型 + RPC + Signer + CapacityBalancer
  - CKB node 提供 acceptance 证据

---

## 3. CellScript → CellFabric 的现有 bridge（CellScript 端）

### 3.1 入口：`cellc action build --fabric-intent`

CLI 命令：`cellc action build` 加 `--fabric-intent` flag（一次性切到 fabric envelope
输出而不是原始 `cellscript-action-builder-plan-v1` JSON）。

完整 CLI 路径：

1. **`ActionBuildArgs` 结构**定义在
   `/Users/arthur/RustroverProjects/CellScript/src/cli/commands.rs:444-453`，有一个
   `pub fabric_intent: bool` 字段。
2. **Clap 注册**：`src/cli/commands.rs:12394-12397` 注册 `--fabric-intent` flag，
   help 文本是 `"Emit a CellFabric intent envelope instead of the raw CellScript
   action plan"`。
3. **解析**：`src/cli/commands.rs:13415-13426`（`Some(("action", m))` 分支）把 flag
   透到 `ActionBuildArgs.fabric_intent`。
4. **dispatch + IO**：在 action-build handler 里（`src/cli/commands.rs:2898-2930`），
   当 `args.fabric_intent == true` 时调
   `cellfabric_intent_envelope_json(&result.metadata, action, &plan, &input_path,
   &metadata_hash)` 替换原来输出的 `plan`，否则输出原始 `plan`。
5. **envelope 生成器**：定义在 `src/cli/commands.rs:10140-10272`。

外部 README（`README.md:417`）明确写：

> "It is not a wallet UI, frontend kit, or CellFabric intent engine."

所以 CellScript 拒绝把 `cellc` 升级成 CellFabric 引擎；只把 envelope 充当 CellFabric
import 的输入。

### 3.2 envelope schema 关键字段

schema 字符串常量：`"cellscript-cellfabric-intent-envelope-v0.20"`
（`src/cli/commands.rs:10151`，test 断言在 `tests/cli.rs:7252`）。
`status: "requires-runtime-binding"`（`src/cli/commands.rs:10152`，runtime binding
由 CellFabric 端做；这是设计意图——不是为了 CellScript 后面还能 emit 一个 final
state）。

完整 envelope JSON 结构（`src/cli/commands.rs:10150-10271`）：

```jsonc
{
  "schema": "cellscript-cellfabric-intent-envelope-v0.20",
  "status": "requires-runtime-binding",
  "bridge_boundary": { /* 详细字段见下文 */ },
  "source": {
    "input":              "<absolute input path>",
    "module":             "<module name, e.g. cellscript::fungible_token>",
    "action":             "<action name, e.g. mint>",
    "target_profile":     "ckb",
    "compiler_version":   "<string, e.g. 0.21.0-rc.1>",
    "metadata_hash":      "<blake2b-256 of metadata.json>",
    "artifact_hash":      "<compiler-emitted artifact_hash>",
    "action_plan_hash":   "<blake2b-256 of embedded action_plan>"
  },
  "cellfabric_mapping": {
    "target":                  "CellFabric IntentBody template",
    "candidate_intent_action": "App",
    "payload_format":          "cellscript-action-plan-json-v1",
    "payload_hash_field":      "cellscript_action_plan_hash",
    "resource_binding":        "runtime-resolved-live-cells",
    "auth_binding":            "runtime-wallet-or-live-cell-context",
    "settlement_compiler":     "cellscript-ckb-adapter-or-generated-builder"
  },
  "cellfabric_intent_template": { /* 详细字段见下文 */ },
  "resource_access_template": {
    "hard_conflicts": {
      "status": "runtime-required",
      "consumed_cell_patterns":     "<action.consume_set, like {type:'Token',binding:'token'} >",
      "runtime_input_requirements": "<from action.transaction_runtime_input_requirements>",
      "note": "CellFabric OutPointRef conflicts must be filled from resolved live cells before submitting a SignedIntent."
    },
    "reads":  "<action.read_refs>",
    "writes": {
      "creates": "<action.create_set>",
      "mutates": "<action.mutate_set>"
    },
    "app_conflict_key_templates": "<from cellfabric_app_conflict_key_templates()>"
  },
  "required_runtime_evidence": [
    "author_lock_script_hash", "intent_nonce",
    "resolved_consumed_outpoints", "resolved_read_outpoints",
    "cellfabric_auth_signature", "deployment_identity",
    "live_cell_resolution", "capacity_fee_balance",
    "estimate_cycles", "tx_pool_acceptance", "l1_status_observation"
  ],
  "non_claims": [
    "does not create a CellFabric SignedIntent",
    "does not prove CellFabric orderer acceptance",
    "does not soft-confirm the action",
    "does not prove live-cell availability",
    "does not prove CKB tx-pool acceptance",
    "does not prove L1 finality"
  ],
  "action_plan": { /* embedded CellScript action build plan, policy = "cellscript-action-builder-plan-v1" */ }
}
```

设计要点：

- **`bridge_boundary.kind: "json-bridge"`**——明示这不是 in-process FFI。
- **`source.metadata_hash` / `source.artifact_hash` / `source.action_plan_hash`**
  三个 hash 一起把"编译器-side 已经是 deterministic"的事实交给 CellFabric 端验证，
  其中 `metadata_hash` 是 `compile_metadata` 的 blake2b-256（`src/commands.rs:2898`
  处的 `metadata_hash` 在外层用 `hash_json_value("metadata", &metadata)` 计算）；test
  断言三者在 `tests/cli.rs:7255-7258` 出现并对应字符串。
- **`cellfabric_intent_template`** 字段（`src/cli/commands.rs:10181-10234`）：
  - `domain.chain_id = "<target_profile name>" = "ckb"`（`src/cli/commands.rs:10184`）
  - `domain.app_namespace = "<module>"`（`src/cli/commands.rs:10185`）
  - `author.lock_script_hash = null`（`src/cli/commands.rs:10188`，由 runtime wallet
    提供）
  - `nonce = null`、`validity.valid_after_ms = null`、`validity.valid_until_ms = null`
    ——**完全交由 runtime 填**
  - `resources.consumes = []`、`resources.reads = []`（`src/cli/commands.rs:10197-10198`）——
    见 § 3.5 的 resource_access_template 与之分离
  - `resources.app_keys = [...]`（由 `cellfabric_app_conflict_key_templates()` 生成，
    见 §3.4 讨论）
  - `action.kind = "App"`、`action.action = "<name>"`、`action.payload_format =
    "cellscript-action-plan-json-v1"`、`action.payload_hash = <action_plan_hash>`
    ——这是 CellFabric 端 `IntentBody.payload_hash` 的 raw contract
  - `constraints.*` 把 `transaction_runtime_input_requirements` /
    `verifier_obligations` / `fail_closed_runtime_features` 同步过来（`src/cli/commands.rs:10208-10213`）
  - `dependencies.requires = []` 且 `dependencies.source = "service-supplied-cellfabric-intent-ids"`——
    CellFabric parent-level dependency 由 service 决定（`src/cli/commands.rs:10214-10217`）
  - `replacement.supersedes = []`、`replacement.rule = "service-policy"`
  - `fee.* = null`、`fee.source = "runtime-builder-policy"`
  - **`auth_mode = "CoSignConcreteTx"`**（`src/cli/commands.rs:10227`，仅此一种，
    直接写死字符串）
  - `metadata.cellscript_* = <三 hash>` —— 回填到 CellFabric intent metadata block

### 3.3 边界设计：为什么 `no-cell-fabric-rust-crate`

`bridge_boundary.cellscript_core_dependency = "no-cell-fabric-rust-crate"`
（`src/cli/commands.rs:10155`，test 断言在 `tests/cli.rs:7255`）。这是**硬编码的
设计约束**。

证据文档（`docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:5-7`）：
> "The CellFabric bridge is a JSON boundary between CellScript action planning
> and CellFabric intent ordering. **It does not add a Rust dependency from
> the CellScript compiler core to CellFabric.**"

旁证：`Cargo.toml:62-95` 完整列出了 `cellscript` 库的依赖，**没有任何 `cellfabric-*`
crate** 出现（该仓库根 `Cargo.toml` 是 virtual workspace，包含 4 个成员，4 个成员
的 Cargo.toml 也都不依赖 CellFabric）。同样可以从根 `Cargo.lock`（`Cargo.lock`
100 KB+）里搜无 `cellfabric` 包。

为什么这样选，文档和代码里都没有"为什么 JSON-only"的完整论述，但存在的几条理由
可以观察：

1. **编译侧不能碰 SDK / node / chain**：CellScript 编译期坚持"no-ckb-sdk-rust"
   （`docs/CELLSCRIPT_CKB_ADAPTER.md:28-32`、`crates/cellscript-ckb-adapter/src/lib.rs:433-435`）：
   adapter 解析阶段就硬要求 `adapter_contract.compiler_core_dependency ==
   "no-ckb-sdk-rust"`。同理，导入 CellFabric Rust crate 会把 CellFabric 的
   `IntentDag` / `Bundle` / `SoftConfirmation` / runtime builder / HTTP server
   一并带入 compiler 编译路径，破坏 offline / metadata-only / wasm-playground / browser
   playground 的边界（playground bundle size budget 600 KB gzip，
   `AGENTS.md:319-326`）。
2. **CellScript 编译器核心可以做 wasm / 浏览器 playground**：如果把 CellFabric crate
   引入，则 CellFabric 的 HTTP / Ed25519 / DashMap / tokio 等会用到的依赖会让
   `wasm32-unknown-unknown` target 不能 build（`src/lib.rs:11-15` 解释 wasm exclusion）。
3. **CellFabric-side 边界文档已经多次表达要"reciprocal"**：
   `docs/CELLSCRIPT_CKB_ADAPTER.md:212-214` 写出 adapter 端要求
   `compiler_core_dependency: "no-ckb-sdk-rust"`；对称地，
   `bridge_boundary.cellscript_core_dependency: "no-cell-fabric-rust-crate"`
   体现 CellFabric 端在编译期不依赖 CellScript crate。这一对"边界检查"互相咬合。
4. **CellFabric 端同样遵守**：根据 `docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:62-83`，
   parent-project CellFabric 提供 `cellscript_import` / `cellscript_flow` example
   + `POST /cellscript/import`（在 `http` feature 下）作为反向消费，
   **而不是把 CellScript lib 拉进 CellFabric 编译路径**——这是 JSON schema
   互操作而不是 FFI 互操作。
5. **`Cargo.toml:20-26` 工作区成员列表**：`members = [".", "crates/cellscript-ckb-adapter",
   "crates/cellscript-wasm", "examples/ckb-sdk-builder"]`——4 个成员里没有
   `cellscript-cellfabric-bridge` 或类似 crate，证明 bridge 故意不开新成员。

**结论**：`no-cell-fabric-rust-crate` 不是"工程小 bug 我们以后会修"——是**结构性的
单向导出**：CellScript 编译器核心不导 Rust API 给 CellFabric；CellFabric 通过
`cellscript_flow` 这种 sibling-example 方式反消费 envelope JSON。如果 owner 要
让 Myelin/CellFabric ↔ CellScript 之间**通过 cargo crate 互相 import，目前编译器
不提供这条路径**，必须走 JSON envelope 或 TypeScript builder。

### 3.4 `bridge_boundary` 三个 flag 的语义

源码里写的是**四个 boolean**（不是三个，但 README/文档没明说"三 flag"），都直接
带"not_a_*"否定前缀（`src/cli/commands.rs:10153-10161`）：

| Flag | 含义 |
|---|---|
| `cellscript_core_dependency: "no-cell-fabric-rust-crate"` | CellScript 编译器核心没有 Rust crate 依赖 CellFabric（设计契约，见 § 3.3） |
| `cellfabric_expected_role: "intent-ordering-soft-confirmation-and-settlement-tracking"` | CellFabric 端做什么（"intent ordering + soft confirmation + settlement tracking"） |
| `not_a_cellfabric_signed_intent: true` | 这玩意儿**不是**已签 SignedIntent（CellFabric 才造 SignedIntent） |
| `not_a_soft_confirmation: true` | **不是** orderer soft confirmation receipt |
| `not_l1_finality: true` | **不是** L1 finality claim（test 也有：在 `tests/cli.rs:7258`） |
| `compiler_must_not_infer_cellfabric_finality: true` | 编译器**永远不应该尝试推断** CellFabric finality——这条是"反编译期长臂"约束 |

> "三个 flag"的提法来自 task brief，可能指 `not_a_*` 三件套（"不是 SignedIntent"
> / "不是 soft confirmation" / "不是 L1 final"）；但实际源码里这块有 6 个字段，
> 见上表。补一下避免"3 vs 6"对不上。

任务 brief 也提到 `cellscript_core_dependency`——它确实是核心字段，但更精确的语义
是"编译器核心没有 Rust-level 依赖到 CellFabric crate"（`no-cell-fabric-rust-crate`），
而不是泛泛的"无依赖"。

### 3.5 `cellfabric_app_conflict_key_templates` 包含什么

定义在 `src/cli/commands.rs:10274-10298`。它从 action metadata 衍生 app-level
conflict key，把下面三类输入 unique 一遍到 `BTreeSet<(key_type, key)>`：

1. `action.touches_shared` → `(cellscript-shared-resource, <shared-name>)`
2. `action.mutate_set` 每项 → `(cellscript-mutate-binding, "<ty>:<binding>")`
3. `action.pool_primitives` 每项 → `(cellscript-pool-primitive, <serde_json>)`

每个 key 输出为：

```json
{
  "namespace":     "<module>",
  "key_type":      "...",
  "key":           "...",
  "key_encoding":  "utf8",
  "key_bytes_hex": "<hex of key bytes>"
}
```

放在 `cellfabric_intent_template.resources.app_keys` 和
`resource_access_template.hard_conflicts.app_conflict_key_templates` 两处——
两处用同一个 helper，所以**完全一致**（`src/cli/commands.rs:10149` 调用一次，
`src/cli/commands.rs:10247` 再调用一次）。

### 3.6 0.14 release 怎么 audit 这条边界

`scripts/cellscript_0_14_scope_audit.sh` 是**离 cellfabric 无关**的 audit；它做
0.14 的 7 个 `examples/language/v0_14_*.cell` 的强制 metadata 校验，要求：

- `Action Builder, CellFabric, CCC integration, or automatic transaction` 这串字
  必须在 `docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md` 里找得到（line 52）。
- README.md 必须 link 到 0.14 release notes（line 54）。
- 每个示例 emit 的 metadata 必须填齐 `source_encoding`, `witness_abi`,
  `spawn_ipc_abi`, `outputs_data ABI`, `type_id_abi` 等（line 124-129）。

由此得到的**结构性 fallback**：`0.14 release notes` 显式把 `Action Builder,
CellFabric, CCC integration, automatic transaction generation` 列为非范围
（`docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184`）。"audit boundary missing"
如果该字符串找不到则 exit 1，反过来"release notes does not mention CellFabric by name"
也会 fail。"cellfabric 必须显式否认" 这是 0.14 release 的一个结构性 audit，
由 `require_doc_boundary` 守护（脚本 line 25-32）。

注意：`docs/CELLSCRIPT_GATE_POLICY.md:65-69` 显式指出：

> "`./scripts/cellscript_0_14_scope_audit.sh` is a historical standalone audit from
> the 0.14 release line. It is not invoked by any current gate mode and is retained
> for manual 0.14-compat debugging only; it is not part of the 0.21 release-evidence
> boundary."

这意味着这条 audit 在 0.21 release-evidence 里**已经不被使用**。但 0.20 把 0.14
的"否认 Action Builder/CellFabric/CCC integration"作为 0.14 的关卡延续下来

### 3.7 0.20 release 怎么引入 bridge

`docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md` 没有专门的"CellFabric 集成"段，
但 `docs/CELLSCRIPT_0_20_ROADMAP.md:106-129` 即"Forteenth slice"+ "Fifteenth slice"：
> "Fourteenth slice: `cellc action build --fabric-intent` now emits a
> `cellscript-cellfabric-intent-envelope-v0.20` JSON bridge for parent-project
> CellFabric services."
>
> "Fifteenth slice: `scripts/cellscript_cellfabric_bridge_smoke.sh` now
> performs a bounded cross-repo smoke check against sibling CellFabric. ...
> without making CellScript depend on the CellFabric Rust crate."

这是 roadmap 层的明确记录；roadmap §P2 CellFabric Exploration (Frozen Except Bridge)
（`docs/CELLSCRIPT_0_20_ROADMAP.md:513-528`）把 CellFabric 状态锁定为
**"frozen for the 0.20 acceptance pass beyond the bounded `cellc action build
--fabric-intent` JSON bridge"**。所以 0.20 完成了"Bridge envelope"以及
"`cellscript_cellfabric_bridge_smoke.sh`"两步；剩下的 intent-DAG composition 与
runtime builder 集成显式地 forward 到 0.20 之后。

> 矛盾点：`docs/CELLSCRIPT_0_20_ROADMAP.md:529-550` 的 §Non-Goals 写：
> "Do not claim cross-protocol CellFabric intent composition in the per-action
> builder release. Do not treat frozen CellFabric exploration as a 0.20
> acceptance blocker."
>
> 但 `docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md`（被官方称为"final 0.20 notes"）
> 没有"CellFabric bridge"小节——release note 的事实是"已发布的 0.20 里 envelope
> 实际 ship 了（`src/cli/commands.rs:10150-10272`，`tests/cli.rs:7252-7284`）"，但
> release notes 文件本身（`docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md:1-229`）
> **没有命名这条 bridge**——只在"ELF entry ABI gate"+"exact artifact build
> reports"+"cell_data_codec_manifest"+"metadata partitioning"+"multi-file
> project boundary"+"critical example coverage"+"browser playground"+"final
> polish"这几节里。**roadmap 上有，release notes 上没有**——这是 CellScript 自己
> 文件层的一个文档空白，不影响代码层（代码确实 emit `v0.20` envelope）。

---

## 4. CellScript 的 scope 边界（什么它做、什么它不做）

### 4.1 它做什么

| 责任 | 证据 |
|---|---|
| 解析 `.cell` → AST + types + state checks | `README.md:457-516`, `src/lib.rs:6-40` |
| Lowering 到 RISC-V IR | `src/ir/`, `src/codegen/mod.rs` |
| Emit `.s` / `.elf` artifact | `ArtifactFormat` (src/lib.rs:307-313) |
| Emit structured metadata sidecar | `src/lib.rs:381-420`, `METADATA_SCHEMA_VERSION = 44` |
| Emit CKB profile ABI contract | `src/lib.rs:820-851`, `src/lib.rs:247-303` |
| 生成 TypeScript action-builder 包 | `cellc gen-builder --target typescript` (README:760, src/cli/commands.rs:12404-12446) |
| 接受 ckb-std ABI 作为参考 | `docs/CELLSCRIPT_CKB_STD_COMPAT.md` |

### 4.2 它不做什么

显式列在源码 / 文档里的"拒做"清单：

| 它拒绝的事情 | 文件:line 证据 |
|---|---|
| 不实现自己的 VM；目标仅 ckb-vm | `README.md:25-29` |
| 不实现 Action Builder 来"自动生成完整 CKB tx"（只到 action plan 为止） | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184` |
| 不实现 CellFabric intent engine / orderer / soft confirmation | `README.md:417`, `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184` |
| 不实现 CCC integration | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184` |
| 不做 wallet UI / frontend kit | `docs/CELLSCRIPT_CKB_STD_COMPAT.md:214`, `docs/CELLSCRIPT_0_19_ROADMAP.md:446-447` |
| 不在编译期导入 ckb-sdk-rust | `docs/CELLSCRIPT_CKB_ADAPTER.md:28-32`, `crates/cellscript-ckb-adapter/src/lib.rs:433-435` |
| 不在编译期导入 CellFabric crate | `bridge_boundary.cellscript_core_dependency = "no-cell-fabric-rust-crate"` (`src/cli/commands.rs:10155`, `docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:5-7`) |
| 不维护 portable target profile（除了 ckb） | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:184-186` |
| 不实现 first-class verified signer 值 / 隐式 signer derivation | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:186-187` |
| 不实现 first-class `max_cycles` spawn parameter（0.14 scope 拒） | `scripts/cellscript_0_14_scope_audit.sh:50` |
| 不实现 hidden signer/地址 derivation | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:187-188` |
| 不实现 arbitrary byte-slice Blake2b beyond `hash_blake2b(input: Hash) -> Hash` | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:190-191` |
| 不实现 full generic maps | `docs/CELLSCRIPT_0_14_RELEASE_NOTES.md:191-192` |
| WASM playground 不做 server compile / uploaded archive / server-owned state | `docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md:138-148` |
| 不在 `--fabric-intent` 路径上"创造 signed intent / soft confirmation / L1 finality" | `src/cli/commands.rs:10262-10269` |

**这套"不做什么"的 audit 都是结构性的**——每次 release notes 都把它列一遍（`0.14`、`0.19`、`0.20`，
非-goals 节），并由 `cellscript_0_14_scope_audit.sh` 的 `require_doc_boundary` 把
"非范围"字面量 grep 出来守门。

### 4.3 不做的另一面：caller 责任

`docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:99-110` 直接列出 envelope 上**没有**做的事，
必须由 caller (CellFabric 侧 service) 在外面做：

- wallet author binding
- nonce
- resolved live outpoints
- fee policy
- CellFabric auth signature
- deployment identity
- dry-run 证据
- L1 status 观测

这就是 `required_runtime_evidence` 数组里 11 个字符串的来源
（`src/cli/commands.rs:10249-10261`）。

---

## 5. CellScript 的对外公共面

### 5.1 CellScript 工作区 crate（4 个）

来自 `/Users/arthur/RustroverProjects/CellScript/Cargo.toml:2-7`：

| Member | 路径 | 用途 |
|---|---|---|
| `cellscript` (root) | `.` | 编译器 + CLI (`cellc`) + LSP (`tower-lsp`) + WASM playground + REPL + MCP |
| `cellscript-ckb-adapter` | `crates/cellscript-ckb-adapter/` | 用 `ckb-sdk-rust` 5.x 把编译器产物 realize 成 packed CKB tx |
| `cellscript-wasm` | `crates/cellscript-wasm/` | 浏览器编译时 playground（`wasm-bindgen` 装） |
| `cellscript-ckb-sdk-builder-example` | `examples/ckb-sdk-builder/` | cookbook 包，依赖 `cellscript-ckb-adapter` |

`crates/cellscript-ckb-adapter/Cargo.toml:14-22` 列出该 crate 的依赖：
- `ckb-jsonrpc-types = "1.0.0"`
- `ckb-hash = "1.0.0"`
- `ckb-sdk = { path = "../../../ckb-sdk-rust" }`（sibling）
- `ckb-types = "1.0.0"`
- `clap`, `anyhow`, `hex`, `serde`, `serde_json`

注意：**`Cargo.toml` 里没有 `cellfabric`、`cellscript-cellfabric` 之类名字**——
编译器核心 + adapter 都没声明 CellFabric 依赖。

### 5.2 CellScript binary

来自 `Cargo.toml:52-60`：

| Binary | 路径 | 说明 |
|---|---|---|
| `cellc` | `src/main.rs` | 主 CLI；含 `action build`, `action build --fabric-intent`, `gen-builder`, `metadata`, `tx solve`, `deploy plan`, `audit-bundle` 等 |
| `cellscript-mcp` | `src/mcp_main.rs` | MCP wrapper（bound to `cellc` 以做语义 metadata 报告） |

另有一个 `cellscript-ckb-tx-measure` 二进制（`src/bin/ckb_tx_measure.rs`，
`AGENTS.md:151-152`），依靠 sibling `../ckb` checkout，不属于 cellc 主体。

`crates/cellscript-ckb-adapter/src/bin/cellscript-deploy.rs` 是 `cellscript-deploy`
二进制（`crates/cellscript-ckb-adapter/Cargo.toml:10-12`）。

### 5.3 CellScript 已发布的 JSON schema / policy 字符串

来自代码、test、文档交叉处的"字符串字面量"——这些是 CellScript 把"边界"写在文档外面
留下的指针：

| Schema / policy 字符串 | 出处 |
|---|---|
| `cellscript-cellfabric-intent-envelope-v0.20` | `src/cli/commands.rs:10151`, `tests/cli.rs:7252` |
| `cellscript-entry-witness-v1` (ABI magic `CSARGv1\0`) | `src/lib.rs:219-220` |
| `cellscript-action-builder-plan-v1` | `crates/cellscript-ckb-adapter/src/lib.rs:18` (imports) |
| `cellscript-ckb-adapter-contract-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:19` |
| `cellscript-ckb-action-acceptance-report-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:20` |
| `cellscript-action-scan-selectors-v0.21` | `crates/cellscript-ckb-adapter/src/lib.rs:21` |
| `cellscript-ckb-script-evidence-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:22` |
| `cellscript-ckb-script-ref-evidence-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:23` |
| `cellscript-ckb-script-code-dep-evidence-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:24` |
| `cellscript-ckb-deployment-manifest-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:25` |
| `cellscript-ckb-deploy-evidence-v0.19` | `crates/cellscript-ckb-adapter/src/lib.rs:26` |
| `cellscript-action-preview-v1` | `crates/cellscript-ckb-adapter/src/lib.rs:1203` |
| `cellscript-compile-receipt-v1` | `docs/releases/CELLSCRIPT_0_21_RELEASE_NOTES.md:43` |
| `cellscript-cellfabric-intent-envelope-v0.20` 中内嵌的 action-plan schema 字段：`policy = "cellscript-action-builder-plan-v1"`，`adapter_contract.schema = "cellscript-ckb-adapter-contract-v0.19"`，`transaction_draft.state = "ActionPlan"` | `tests/cli.rs:7282-7283`（对 envelope 里 embedded action_plan 的断言） |
| metadata `metadata_schema_version = 44`, `source_metadata_schema_version = 1`, `artifact_metadata_schema_version = 1`, `constraints_metadata_schema_version = 1` | `src/lib.rs:214-217` |
| `MOLECULE_VM_ABI_VERSION = 0x8001`，VM ABI trailer magic `CSABITR0` | `src/lib.rs:863-865` |
| Adapter contract `compiler_core_dependency: "no-ckb-sdk-rust"` | `crates/cellscript-ckb-adapter/src/lib.rs:433-435`（enforced） |

`auth_mode` 当前只有 `"CoSignConcreteTx"` 这一个 string literal
（`src/cli/commands.rs:10227`），即 CellFabric 端必须用 cosign 模式消费。

### 5.4 CellScript 与 `proposals/`（submodules）

主仓 `.gitmodules`（见 `/Users/arthur/RustroverProjects/CellScript/.gitmodules`，
未在本次 grep 中显示主仓 `.gitmodules` 但 vendored 仓
`/Users/arthur/RustroverProjects/Myelin/cellscript/.gitmodules:1-20` 与主仓同源）
声明 6 个 submodule：`website`, `editors/vscode-cellscript`, `tests/benchmarks`,
`proposals/novaseal`, `proposals/evolving-dob/evolving-dob-profile-v1`，
proposals 不参与主 workspace（`Cargo.toml:8-14` exclude 列表）。CellScript 把
proposal-level work（NovaSeal, DOB evolution, iCKB 差分证据）作为单独 submodule，
workspace 主路径只 accept "CellScript compiler + ckb adapter + wasm + sdk-builder
cookbook"四件套。

---

## 6. CellScript 跟 Myelin 的关系

### 6.1 Myelin vendors CellScript

`/Users/arthur/RustroverProjects/Myelin/Cargo.toml:3`：
> `exclude = ["cellscript"]`

即 Myelin 主 workspace（`cli`, `consensus`, `core-utils`, `crypto/hashes`, `exec`,
`math`, `mempool`, `crypto/muhash`, `state`）**不**包含 `cellscript`，
但 `cellscript/` 子目录就是一个 vendored 完整 CellScript repo，作为"外部组件"使用。

`/Users/arthur/RustroverProjects/Myelin/cellscript/Cargo.toml:19` 是 vendored 仓内
`version = "0.20.0-rc.2"`（vendored 仓版本）, 与主仓 `0.21.0-rc.1`（`/Users/arthur/RustroverProjects/CellScript/Cargo.toml:19`）相差 1 个 release candidate。Myelin 的 `check_cellscript_parent_parity.py`
（`/Users/arthur/RustroverProjects/Myelin/scripts/check_cellscript_parent_parity.py:1-100`）
用于"对比 parent checkout 与 vendored copy 并对部分允许的差异（Myelin 适配）记录了
SHA-256 哈希白名单"——即 Myelin 维护 vendored 仓与 parent 主仓的源级一致性，由这个
脚本守门。

vendored 仓与 parent 仓 diff 的"被 Myelin 允许的差异"包含（仅 snippet）：
- `.gitignore` (`64003…`) — Playwright ignores + 展平的 proposal paths 保留
- `crates/cellscript-ckb-adapter/Cargo.toml` (`8562c3…`) — sibling ckb-sdk-rust
  path 因 vendoring 深度而不同
- `scripts/cellscript_ckb_release_gate.sh` (`8abde4…`)、`scripts/cellscript_gate.sh` (`903bc1…`)
- `tools/ckb-tx-measure/Cargo.toml` (`e07c69…`) — sibling CKB 路径差异

### 6.2 Myelin 用 CellScript 编译产物的具体路径

`/Users/arthur/RustroverProjects/Myelin/scripts/myelin_ckb_devnet_smoke.sh:113-173` 给出
Myelin-side 的"调用 `cellc` 编译自己的 .cell 源"实例：

```bash
cp "$ROOT/cellscript/examples/myelin/da-anchor-carrier.cell"     "$WORKDIR/myelin/"
# ... 4 个 .cell: da-anchor-carrier / settlement-carrier / da-anchor-final / settlement-final
cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
  "$WORKDIR/myelin/da-anchor-carrier.cell" \
  -t riscv64-elf --target-profile typed-cell --primitive-compat 0.18 \
  --entry-action verify_da_anchor_carrier \
  -o "$WORKDIR/myelin/da-anchor-carrier.typed-cell.elf"
```

四对编译各跑两遍——一次 `--target-profile typed-cell`、一次 `--target-profile ckb`，
然后从 CKB-profile ELF 算 `code_hash`、从 carrier ELF 算出 SHA-256，用作 cell_dep
和 code cell 部署（line 175-200）。

`/Users/arthur/RustroverProjects/Myelin/MYELIN_PRODUCTION_GATE.md:306-312`：

> "The smoke copies `cellscript/examples/myelin/da-anchor-carrier.cell`,
> `cellscript/examples/myelin/settlement-carrier.cell`,
> `cellscript/examples/myelin/da-anchor-final.cell`, and
> `cellscript/examples/myelin/settlement-final.cell` into the throw-away workdir,
> compiles all four checked-in CellScript sources under the `typed-cell` profile
> before compiling their CKB-profile ELFs, and records the typed-cell ELF plus
> metadata sidecar paths in `carrier_verifiers.*` and `final_script_verifiers.*`."

同样地，`/Users/arthur/RustroverProjects/Myelin/scripts/myelin_public_testnet_rehearsal_prepare.sh:80-83`
也用同样的 `cellscript/examples/myelin/*.cell` 4 个路径。

> **矛盾点（请 owner 留意）**：本次 review 时
> `ls /Users/arthur/RustroverProjects/Myelin/cellscript/examples/myelin/` 返回
> `No such file or directory`——**Myelin vendored 仓 `cellscript/examples/` 目录里
> 当前没有 `myelin/` 子目录、没有 `da-anchor-carrier.cell` 等 4 个文件**。脚本
> 的 shell cp 会直接 `cp: ...: No such file or directory`，意味着最近的 smoke 重跑
> 会失败；Myelin production gate / public-testnet rehearsal 也会失败。CellScript 编译
> 路径本身没有消失——`cellscript/examples/` 下仍然有 `token`, `nft`, `amm_pool` 等
> 主仓 7 个生产例子 + `language/` 子目录 + 各个 `examples/<protocol>/Cell.toml`
> 工作区——只是 Myelin 自己的"myelin 子 examples 目录"目前不存在于 vendored 仓中。
>
> `MYELIN_SWARM_AUDIT_WHOLEREPO.md:407-437` 引用的 `cellscript/examples/myelin/*.cell:1-56`
> 这些 `file:line` 也是基于"myelin 子目录存在"的假设（"F-DOC-01"、"F-DOC-05"、
> "F-DOC-20"、"F-DOC-31"等 4 处 finding 都点名 `cellscript/examples/myelin/*.cell:4`,
> `cellscript/examples/myelin/da-anchor-final.cell:13`, `cellscript/examples/myelin/settlement-final.cell:39`
> 等等）。所以 Myelin-side 的 swarm audit 也建立在这些文件存在的前提下。它们
> 在 Myelin vendored 仓和主仓都不存在（既有 `ls` 证据），**这是 Myelin 自身还没
> 解决的 drift**。

### 6.3 Myelin 假定的 `--target-profile typed-cell`

Myelin 脚本使用 `--target-profile typed-cell`（`myelin_ckb_devnet_smoke.sh:121,
128, 135, 142, 149, 156, 163, 170`），并把 carrier ELF / metadata 报告为
`typed_cell_profile_checked = true`（`MYELIN_PRODUCTION_GATE.md:374-375`）。

> **另另一处矛盾（更严重）**：CellScript 编译器本身**只接受 `ckb` profile 名**——

> - `src/lib.rs:127-128`：`CellScript now supports CKB as its only target profile`
> - `src/lib.rs:241-243`：`enum TargetProfile { Ckb }`
> - `src/lib.rs:248-251`：`TargetProfile::from_name` for `"ckb"` → Ok,
>   other → `Err("unsupported target profile '{}'; supported profile: ckb")`
> - 同样在 vendored 仓 `Myelin/cellscript/src/cli/commands.rs:139-451`（target_profile
>   都接受任意字符串但 `TargetProfile::from_name` 只允许 `ckb`）
> - 同样在 vendored 仓 `Myelin/cellscript/src/lib.rs`（同形）

测试断言也是 `target_profile = "ckb"` (`tests/cli.rs:7263`: `assert_eq!(envelope["source"]["target_profile"], "ckb");`)。

所以 Myelin-side 的 `--target-profile typed-cell` 一旦传给 vendored 的 `cellc`，
**会被 fail-closed 拒绝**。MYELIN_PRODUCTION_GATE.md / MYELIN_SWARM_AUDIT_WHOLEREPO.md
则宣称"4 fixtures compile cleanly under `--target-profile ckb` and
`--target-profile typed-cell`"（`MYELIN_SWARM_AUDIT_WHOLEREPO.md:437`）——这是
Myelin-side documentation 与 vendored CellScript 编译器的实际接口 surface 之间的
drift。

MYELIN_CKB_SEMANTIC_DEVIATIONS.md 全文搜了 `typed-cell` 主要出现在
`D-03`, `D-10`, `D-05`, `D-23` 等条目，但只在 `exec/src/celltx/types.rs::TypedCellDecl`、
`compute_typed_data_hash` / `Script::hash_v1` 这一侧提到 typed-cell ——都是 Myelin-side
**自己定义的 typed-cell 概念**，并非 CellScript 编译器的 `target profile typed-cell`。

`MYELIN_SESSION_L2_PLAN.md:448-456` 暗示：

> "CellScript now includes focused typed-cell package-commitment regressions:
> `v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper` ...
> They compile `PackageCommitment` under the local `typed-cell` profile"

——这是 Myelin-side 在自己的 v0_18 test suite 里**期望** CellScript 接受
`typed-cell` profile，但 vendored CellScript 实际不接受。

> **对综合任务而言这是一个事实层面矛盾**：Myelin-side 的脚本、reports、audits 都
> 把 `--target-profile typed-cell` 当作 CellScript 编译器已经 ship 的功能记录下来，
> 但 CellScript（0.21.0-rc.1 主仓 / 0.20.0-rc.2 vendored 仓）实际只支持 `ckb`。
> Myelin-side 要么是 patch CellScript 但 patch 没进 vendored copy（main 与 vendored
> diff 除白名单外不允许），要么是这个 `typed-cell` profile 是 Myelin roadmap 的
> 设计目标而非现状。本次 review 不下结论；只把现状摆在这里供 owner 综合。

### 6.4 Myelin 与 CellFabric 的关系

`/Users/arthur/RustroverProjects/Myelin/MYELIN_*.md` 文件全部 grep `CellFabric |
cellfabric | fabric-intent | fabric_intent`，**没有任何匹配**（仅做大小写不敏感 grep
`Myelin/cellfabric|Myelin/CellFabric|Myelin/fabric-intent|Myelin/fabric_intent`
亦为 0 匹配）。CellFabric 端对 Myelin 也没提及（依据 task brief cellfabric-side 的
确认；本次 review 不重复验证）。

即：**Myelin 仓库与 CellFabric 仓库**当前**没有任何文本层交叉**。Myelin 的所有
CellScript-bridged L2 工作是通过 `cellscript/examples/myelin/*.cell`（编译 + cellc
直接产出 .elf）走，并没有通过 `cellc action build --fabric-intent` 把任何东西
emit 成 `cellscript-cellfabric-intent-envelope-v0.20` envelope JSON。

> 这与本次 task 第 4 点的 CellScript→CKB 编译路径一致：Myelin 在做的是
> **typed-cell compiler profile + CKB cellc adapter + ckb-testtool + CKB devnet RPC**，
> **不是 CellFabric 集成**。

### 6.5 Myelin 与 CellScript 的其它接触面

`MYELIN_CKB_SEMANTIC_DEVIATIONS.md:17` (`D-03`)：

> "Myelin scheduler witnesses (CellScript typed-cell scheduler metadata) are not
> part of the CKB Molecule transaction layout."
> 写在 `exec/src/celltx/types.rs::CellScriptSchedulerWitness` / `push_cellscript_scheduler_witness`。

即 Myelin 自己也把"CellScript scheduler metadata" 当作一种 typed-cell witness，
**但**这层语义是 Myelin-side 自己的 layer —— 既不是 CellScript 编译器合约层的
输出（CellScript 没有 "scheduler witness" 这个 schema），也不是 CellFabric 的合约。
`MYELIN_CKB_SEMANTIC_DEVIATIONS.md:17` 把这条标为 `ProjectionWarning::SchedulerWitnessPresent`
"future sweep" 才 surface——也就是 Myelin 当前并**没有实现** scheduler witness
到 CKB Molecule table 的实际编码。

`MYELIN_PRODUCTION_GATE.md:286-330` 进一步说：

> "It uses the parent `../ckb` checkout, starts a throw-away devnet, mines a
> spendable always-success funding cell, deploys separate compact CellScript
> DA-anchor and settlement carrier verifiers, commits a 160-byte Myelin DA-anchor
> carrier payload into CKB output data guarded by the DA `data2` type script,
> and binds carrier type args to `ckb_data_hash(carrier_payload) ||
> carrier_identity_hash`, where the identity hash is the DA manifest hash or
> settlement intent hash."

所以 Myelin-on-CKB 的实际工作路径是：**CellScript 编译产物 → cellc → 4 个 ELF +
metadata sidecar → cellc adapter (cellscript-ckb-adapter) → 用 sibling `ckb` binary
跑本地 devnet → CKB RPC `get_live_cell` / `send_transaction`**。CellFabric 不在
这个回路。

---

## 7. 配合面观察（从 CellScript 视角看 CellFabric — 给 owner 综合用）

1. **bridge 是单向 JSON envelope，没有 Rust crate 互导**。
   `bridge_boundary.cellscript_core_dependency = "no-cell-fabric-rust-crate"`
   （`src/cli/commands.rs:10155`、`docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:5-7`）把编译器
   核心锁死。如果 CellFabric 想复用 CellScript 的解析/降级/codegen，只能走：
   - `cellc` 作为外部 binary 调（shell），或
   - 走 `cellc gen-builder --target typescript` 生成 TypeScript 包，TypeScript side
     内部用 `cellscript-builder-manifest` 而不是 CellFabric crate
   - 走 `cellscript-ckb-adapter` crate 用 RISC-V artifact 之外，是**编译期+后端**之间的
     边界
   没有任何路径让 CellFabric 直接 `use cellscript` 把编译结果 import 进来。

2. **envelope schema 把"已知的事"给 CellFabric，把"未知的事"留给 runtime**。
   CellScript 编译器是 deterministic producer：metadata hash、artifact hash、
   action_plan hash 三个 hash 都进了 envelope（`src/cli/commands.rs:10162-10171`）
   且 `tests/cli.rs:7261` 断言它们是 64-hex-char；CellFabric 端可以 bind 下来。
   但 `author.lock_script_hash`、`nonce`、`validity.*`、`fee.*`、`consumes`、
   `reads` 等都设为 `null` 或 `[]`——CellScript 编译器不能填，让 CellFabric runtime
   填。这是"deterministic compiler + non-deterministic runtime" split 的标准做法，
   实际意图在文档 `docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:112-130` 写明：
   > "CellScript compiler metadata can describe resource roles, runtime input
   > requirements, create sets, mutate sets, verifier obligations, and app conflict
   > key templates. It cannot know the final CKB OutPointRef values until runtime
   > live-cell resolution."

3. **bridge 的 `app_conflict_key_templates` 仅靠 schema 推**。
   `src/cli/commands.rs:10274-10298` 用 action metadata 的 `touches_shared` /
   `mutate_set` / `pool_primitives` 衍生 app conflict key，
   每条都是 `{namespace, key_type, key, key_encoding="utf8", key_bytes_hex}`。
   这意味着 CellFabric 端必须信任 CellScript 的"key 命名稳定"——
   任何 schema 改动 / 类型重命名 / pool primitive 改写都会让历史 key 失配。
   `tests/cli.rs:7270-7274` 仅断言 `app_keys.status` 与 `app_conflict_key_templates`
   存在，**不**做 key 集合 exact-match 断言，所以测试并不守"key 在编译期稳不稳定"。

4. **`auth_mode = "CoSignConcreteTx"` 是 CellScript 视角唯一允许的合同模式**。
   `src/cli/commands.rs:10227` 直接 hardcode 字符串 `"CoSignConcreteTx"`，
   CellFabric 端必须构造 CoSignConcreteTx intent body。如果 CellFabric 上层有
   `AuthMode::KeyOnly` / `AuthMode::MultiSig` / `AuthMode::None` 等其它 auth 模式，
   CellScript 这一端目前不会替你适配——`required_runtime_evidence` 列表里
   `"cellfabric_auth_signature"` 只要求一个 signature 位，没有"必须用某 mode"的强约束。

5. **CellScript 把 settlement 拒绝留在 CPU 外**。
   `cellfabric_intent_template.settlement_compiler` 字符串值是
   `"cellscript-ckb-adapter-or-generated-builder"`（`src/cli/commands.rs:10179`）。
   这是给"caller 自己挑"——CellScript 既不扮演 settlement compiler，也不替 caller
   选。需要 CellFabric 端在 gateway + ordering + soft confirmation 之后再调
   `cellscript-ckb-adapter::build_action_transaction` 或 generated TypeScript builder 的
   `buildTransaction(...)` 来出 packed CKB tx。这与 bridge 文档
   `docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:131-145` 的 `Recommended Execution Path`
   一致：CellFabric → service → runtime builder / adapter → CKB RPC → tracker。

6. **`bridge_boundary.compiler_must_not_infer_cellfabric_finality` 这一条约束**。
   `src/cli/commands.rs:10160` 既是给 CellFabric 端的保证、也是给未来 CellScript 编译器的
   "如果某天编译器想 infer orderer 状态——立即拒绝"。这条 flag 是 CellScript 自己
   的"long-term invariant"，目前没有 enforcement 代码——它只是 JSON 字段声明，对
   failure-mode 的事后追溯起作用。

7. **`--fabric-intent` 路径在 release-evidence 里已经被冻结**。
   `docs/CELLSCRIPT_GATE_POLICY.md:65-69` 强调 CellFabric smoke 不是任何 gate mode
   的一部分；0.20 roadmap § P2 把 CellFabric exploration 明文"frozen for the 0.20
   acceptance pass beyond the bounded `cellc action build --fabric-intent` JSON
   bridge"（`docs/CELLSCRIPT_0_20_ROADMAP.md:513-528`），roadmap 的 non-goals 节明确
   "Do not claim cross-protocol CellFabric intent composition in the per-action
   builder release"。所以：

   - 编译器仍 **emit** envelope，但
   - envelope 之外的 CellFabric 集成（intent DAG、bundle selection、orderer、soft
     confirmation、settlement finalize、tracker L1 status observations）**都是
     0.20 frozen — 后续 release 才再看**。

8. **CellScript 的 release-evidence 不包含 CellFabric**——除非 bridge smoke 跑过。
   CellScript 的 release 路径按 `scripts/cellscript_gate.sh` 的 `dev` / `ci` /
   `backend` / `release` / `release-quick` 走，没有 CellFabric integration 的 CI
   模式。CellFabric smoke 是 sibling-checkout manual run，靠 `cellscript_cellfabric_bridge_smoke.sh`
   在 `../CellFabric` 路径假设存在时才跑得通。

9. **Myelin 与 CellFabric 现在**不**互通**——Myelin 的 vendored CellScript 仓里
   `cellscript/examples/myelin/*.cell` 子目录缺失，且 `--target-profile typed-cell`
   不是 CellScript 编译器支持的 profile 名。这些事实使 Myelin 同时**实际上也没有走**
   `cellc action build --fabric-intent` emit envelope 的路径——Myelin-side 工作是
   直接 type-cell carrier + ckb-sdk-rust，没有把任何东西 emit 成
   `cellscript-cellfabric-intent-envelope-v0.20` envelope JSON。

---

## 附录 A：本次 review 涉及的 file:line 速查

- `/Users/arthur/RustroverProjects/CellScript/README.md:25-29, 52-66, 127-137, 207-209, 417, 457-516, 760`
- `/Users/arthur/RustroverProjects/CellScript/AGENTS.md:91-92, 110-124, 170-178, 282, 311, 319-326`
- `/Users/arthur/RustroverProjects/CellScript/Cargo.toml:2-7, 19, 52-60`
- `/Users/arthur/RustroverProjects/CellScript/src/lib.rs:11-15, 127-128, 211-217, 219-220, 247-303, 307-313, 381-420, 528-770, 778-803, 820-851, 863-909`
- `/Users/arthur/RustroverProjects/CellScript/src/cli/commands.rs:444-453, 12394-12397, 13415-13426, 2898-2930, 10140-10272, 10274-10298`
- `/Users/arthur/RustroverProjects/CellScript/crates/cellscript-ckb-adapter/Cargo.toml:10-22`
- `/Users/arthur/RustroverProjects/CellScript/crates/cellscript-ckb-adapter/src/lib.rs:18-26, 433-435, 515-588, 1203, 1091-1152`
- `/Users/arthur/RustroverProjects/CellScript/docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:1-148`
- `/Users/arthur/RustroverProjects/CellScript/docs/CELLSCRIPT_CKB_ADAPTER.md:13-50, 119-160, 200-273`
- `/Users/arthur/RustroverProjects/CellScript/docs/CELLSCRIPT_GATE_POLICY.md:60-85`
- `/Users/arthur/RustroverProjects/CellScript/docs/CELLSCRIPT_0_20_ROADMAP.md:106-130, 513-550, 552-569`
- `/Users/arthur/RustroverProjects/CellScript/docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-188`
- `/Users/arthur/RustroverProjects/CellScript/docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md:67-79, 119-148`
- `/Users/arthur/RustroverProjects/CellScript/docs/releases/CELLSCRIPT_0_21_RELEASE_NOTES.md:43`
- `/Users/arthur/RustroverProjects/CellScript/scripts/cellscript_cellfabric_bridge_smoke.sh:1-151`
- `/Users/arthur/RustroverProjects/CellScript/scripts/cellscript_0_14_scope_audit.sh:50, 52, 54, 124-129`
- `/Users/arthur/RustroverProjects/CellScript/tests/cli.rs:7205-7284`
- `/Users/arthur/RustroverProjects/CellScript/examples/token.cell:1-48`

- `/Users/arthur/RustroverProjects/Myelin/Cargo.toml:3`
- `/Users/arthur/RustroverProjects/Myelin/cellscript/Cargo.toml:19`
- `/Users/arthur/RustroverProjects/Myelin/cellscript/.gitmodules:1-20`
- `/Users/arthur/RustroverProjects/Myelin/scripts/check_cellscript_parent_parity.py:1-100`
- `/Users/arthur/RustroverProjects/Myelin/scripts/myelin_ckb_devnet_smoke.sh:80-200, 318-380`
- `/Users/arthur/RustroverProjects/Myelin/scripts/myelin_public_testnet_rehearsal_prepare.sh:80-83`
- `/Users/arthur/RustroverProjects/Myelin/MYELIN_PRODUCTION_GATE.md:280-379`
- `/Users/arthur/RustroverProjects/Myelin/MYELIN_SESSION_L2_PLAN.md:388-456`
- `/Users/arthur/RustroverProjects/Myelin/MYELIN_SWARM_AUDIT_WHOLEREPO.md:407-437`
- `/Users/arthur/RustroverProjects/Myelin/MYELIN_CKB_SEMANTIC_DEVIATIONS.md:17, 24`

---

## 附录 B：本次 review 已确认的 7 项发现

| task brief 要求 | 本报告对应 | 关键 file:line |
|---|---|---|
| CellScript 是什么（一段话） | § 1 | README:14-19、src/lib.rs:1-2、src/lib.rs:127-128 |
| CellScript → CKB 编译路径 | § 2 | README:457-516、src/lib.rs:6-40、src/lib.rs:307-313、src/lib.rs:611-770 |
| CellScript → CellFabric 现有 bridge | § 3 | src/cli/commands.rs:444-453、10140-10298 |
| ── `cellc action build --fabric-intent` CLI 路径 | § 3.1 | src/cli/commands.rs:444-453, 12394-12397, 13415-13426, 2898-2930 |
| ── envelope schema `cellscript-cellfabric-intent-envelope-v0.20` | § 3.2 | src/cli/commands.rs:10151, 10181-10234 |
| ── `bridge_boundary` flag 语义 | § 3.4 | src/cli/commands.rs:10153-10161 |
| ── `cellfabric_intent_template` 包含什么 | § 3.2 + § 3.5 | src/cli/commands.rs:10181-10234, 10274-10298 |
| ── `cellscript_core_dependency: no-cell-fabric-rust-crate` 是设计 | § 3.3 | src/cli/commands.rs:10155、docs/CELLSCRIPT_CELLFABRIC_BRIDGE.md:5-7、Cargo.toml:62-95 |
| CellScript 自己怎么界定 scope 不越界 | § 4 | docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md:181-184、scripts/cellscript_0_14_scope_audit.sh:50-55 |
| CellScript 端 runtime / settlement 路径 | § 2.4 + § 7.5 | crates/cellscript-ckb-adapter/src/lib.rs:433-435、docs/CELLSCRIPT_CKB_ADAPTER.md:13-45 |
| Myelin 关系（vendors + 用法 + fabric-intent） | § 6 | Myelin/Cargo.toml:3、Myelin/cellscript/Cargo.toml:19、myelin_ckb_devnet_smoke.sh:113-200、MYELIN_PRODUCTION_GATE.md:306-330、MELIN-side drift on typed-cell profile |

---

## 附录 C：本报告 flag 给 owner 复核的内部矛盾

### C-1 release notes 与代码的"CellFabric bridge"差距

`docs/releases/CELLSCRIPT_0_20_RELEASE_NOTES.md`（"final 0.20 notes"，文件头 `Updated:
2026-06-28`）全文里**没有"CellFabric"小节**——但 `docs/CELLSCRIPT_0_20_ROADMAP.md:106-129`
第 14 / 15 slice 明文宣称 0.20 已经 ship 了 `cellc action build --fabric-intent` +
`scripts/cellscript_cellfabric_bridge_smoke.sh`，代码里 envelope schema
`v0.20` 也确实存在（`src/cli/commands.rs:10151`）。roadmap 写、release notes 没写——
**roadmap 提了 release 但 release notes 没记入**。这是 CellScript 自己的文档层
inconsistency，与本次综合任务无关但值得 CellScript 上游注意。

### C-2 Myelin-side 引用了 CellScript 没有的目录和 profile

1. `cellscript/examples/myelin/da-anchor-carrier.cell` 等 4 个 `.cell` 在 vendored 仓
   `Myelin/cellscript/examples/` 下不存在（ls 验证）。
2. `--target-profile typed-cell` 在 vendored 仓 `Myelin/cellscript/src/cli/commands.rs`
   与主仓 `CellScript/src/cli/commands.rs` 都是 `target_profile: Option<String>` 字段
   在 `ActionBuildArgs`/etc.（直接接受任意字符串），但**真正解析**是在
   `CellScript/src/lib.rs:248-251` `TargetProfile::from_name`——只识别 `ckb`，其
   余 fail closed。所以即使 vendored 仓我看的 typed-cell 入口入口字段都是
   `target_profile: Some("typed-cell")` 也最终 reject。

> owner 综合 task 不应把这当作"Myelin 已修"或"Myelin 已实现 typed-cell profile"——
> 现状是 Myelin 的报告/脚本假设 CellScript 已经支持 typed-cell profile，且 vendored
> 仓与脚本期望不一致。本次 review 写明现状，不下"需要修"或"已经在修"的判断。

### C-3 任务 brief 的"3 个 flag" vs 源码的"6 个字段"

任务 brief 里 `bridge_boundary` "三个 flag"——但源码 `src/cli/commands.rs:10153-10161`
里写有 6 个字段（详见 §3.4 表格）。两者对不上不是 CellScript 错、也不是 brief 错
（"3"可能指 `not_a_signed_intent` / `not_a_soft_confirmation` / `not_l1_finality`
三件套），只是规范上的 lexical 选择问题。本报告按源码实际 6 字段记录。

---

## 附录 D：当前 CellScript alpha 状态汇总（按现状写、不做"想要"叙述）

CellScript 当前在 `0.21.0-rc.1`（主仓、`Cargo.toml:19`）/`0.20.0-rc.2`（vendored 仓）、
Rust `1.92.0`、CKB VM v2（包括 Spawn/IPC 2601-2608 syscalls）。新版本计划写在
`docs/CELLSCRIPT_0_21_ROADMAP.md`（含 semantic closure、authenticated compiler
receipts、cyclic graph views、type-level TemplateLayout、deferred template Merkleisation）。

`docs/releases/CELLSCRIPT_0_21_RELEASE_NOTES.md:1-20` 开篇即是 release candidate 措辞
（"0.21.0-rc.1 ... 2026-07-03"），并明文写"This is not a production CKB release claim
by itself"。CellScript 自己的 metadata 即把 release-claim 与 gate-pass 分离。

`Cargo.lock` 在 vendored 仓与主仓是 binary diff（上文 §6.1），cellfabric 字面量 grep
两边都是 0 匹配，与 `Cargo.toml` 的"4 个 members、none of them depends on cellfabric"
一致。
