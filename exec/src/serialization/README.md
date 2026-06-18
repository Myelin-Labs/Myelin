# Myelin 序列化分层治理

本文档描述 Myelin 执行层的序列化架构。

## 架构概述

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: VM/Script ABI 层 (Molecule v1 public)                 │
│  - ResolvedHeader, ResolvedCell, Witness Payload                │
│  - 脚本可见的所有数据结构                                        │
│  - 需要: canonical, partial read, version兼容                  │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: 内部工具/存储层 (Molecule-compatible envelopes)       │
│  - P2P消息 (Protobuf 已覆盖)                                    │
│  - VersionedEnvelope uses Molecule-compatible table bytes        │
│  - Active records use explicit non-derived payload codecs         │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: 共识关键路径 (CKB/Molecule-shaped bytes)               │
│  - Block Hash, TxID, SigHash (自定义流式哈希)                   │
│  - 完全不受影响，继续用 domain-separated Blake3                 │
└─────────────────────────────────────────────────────────────────┘
```

## 实现状态

### Phase 1: 存储层治理 ✅ 已完成

- [x] `VersionedSerializable` trait 定义
- [x] `VersionedEnvelope<T>` 实现
- [x] 所有 CellTx 类型实现 `VersionedSerializable`
- [x] 架构声明文档

### Phase 2: VM ABI 治理 ✅ 已完成

- [x] `VmSerializable` trait 定义
- [x] `VmAbiNegotiator` 版本协商
- [x] `ResolvedHeader` 实现 `VmSerializable`
- [x] `ResolvedCell` 实现 `VmSerializable`
- [x] VM syscall 使用新抽象
- [x] `vm_abi` 模块提供标准化序列化

### Phase 3: Molecule ABI ✅ 已部分完成

- [x] CKB-style `Script` / `OutPoint` / `CellInput` / `CellOutput` Molecule wire layout
- [x] Myelin VM `ResolvedHeader` / `ResolvedCell` Molecule wire layout
- [x] Molecule ABI version `0x8001` exposed through `VmAbiNegotiator`
- [x] CellScript artifact metadata declares VM object ABI `0x8001`
- [x] Exec verifier can select syscall output format from artifact ABI version
- [x] RISC-V ELF artifact ABI trailer can be stripped before CKB-VM loading
- [x] Roundtrip and known-layout tests
- [ ] Generated `.mol` bindings via `moleculec`
- [ ] Embedded/authenticated ABI manifest for non-ELF artifacts
- [ ] Multi-language SDK compatibility validation

## 核心组件

### VersionedSerializable

为存储层类型提供版本化序列化支持：

```rust
use myelin_exec::{VersionedSerializable, VersionedEnvelope};

// 类型自动实现 VersionedSerializable
let tx = CellTx::new(...);

// 包装到 VersionedEnvelope
let envelope = VersionedEnvelope::new(&tx)?;

// 存储到 RocksDB
db.put(key, envelope.to_bytes())?;

// 从 RocksDB 读取并解析
let envelope = VersionedEnvelope::<CellTx>::from_bytes(&bytes)?;
let tx = envelope.parse()?;
```

### VmSerializable

为 VM-facing 类型提供 public Molecule ABI 抽象。Non-Molecule VM object ABI versions are rejected:

```rust
use myelin_exec::{VmSerializable, ResolvedHeader};

// 序列化传递给 VM
let header = ResolvedHeader { ... };
let bytes = header.to_vm_bytes();

// VM 内反序列化
let header = ResolvedHeader::from_vm_bytes(&bytes)?;
```

## 版本策略

### Schema 版本 (VersionedSerializable)

- 每个类型有 `CURRENT_VERSION` 常量
- 版本变更时实现 `upgrade_from` 方法
- 支持从旧版本平滑升级

### ABI 版本 (VmSerializable)

- `0x8001`: Molecule-based ABI v1 (launch/public VM ABI)
- 使用 `VmAbiNegotiator` 协商版本
- VM runtime/syscall 边界使用 Molecule wire format

## 模块结构

```
serialization/
├── mod.rs              # 核心 trait 和类型
├── vm_abi.rs           # VM ABI 序列化辅助函数
├── molecule_compat.rs  # Molecule canonical VM ABI 编码/解码
└── README.md           # 本文档
```

## 迁移路径

### Phase 1 (当前) ✅
- ✅ 所有 CellTx 类型实现 `VersionedSerializable`
- ✅ `ResolvedHeader` / `ResolvedCell` 实现 `VmSerializable`
- ✅ `VmSerializable` 的 public bytes 使用 Molecule；VM syscall 在 runtime 边界使用 Molecule
- ✅ `vm_abi` 模块标准化序列化格式

### Phase 2 (未来 3-6 个月)
- 完善 ABI 版本协商机制
- 支持脚本指定 ABI 版本
- 添加更多 VM-facing 类型的 `VmSerializable` 实现

### Phase 3 (当前推进)
- `molecule_compat` 已实现 canonical Molecule wire layout，不再是 NotImplemented 占位
- `LOAD_SCRIPT` / `LOAD_INPUT` / `LOAD_CELL` / `LOAD_HEADER` full-load 路径已支持 `VmAbiFormat::Molecule`
- `LoadScript` / `LoadInput` / `LoadCell` / `LoadHeader` constructors and `TransactionScriptVerifier` now default to Molecule
- `VersionedEnvelope<T>` emits a Molecule-compatible table by default; `serialize_to_bytes`, `serialize_many`, streaming serialization, and serialization cache entries now use `VersionedEnvelope::to_bytes()`
- `VersionedSerializable` has no derive-based default codec; implementors must provide an explicit payload codec
- Core CellTx-family `VersionedSerializable` implementations use CKB Molecule payloads for `OutPoint`, `Script`, `CellInput`, `CellOutput`, `CellDep`, `DepType`, and `CellTx`
- `SecureEnvelope` now emits a Molecule-compatible table, and integrity helpers use the versioned envelope utilities
- Typed-cell metadata has an explicit Molecule-compatible codec; core transaction and typed-cell metadata structs use explicit codecs
- Native `myelin-exec` has no direct or transitive legacy serializer dependency,
  and no legacy serializer API usage in execution, CellTx, typed metadata,
  scheduler-witness, VM ABI, or serialization code; `myelin-hashes`,
  `myelin-math`, and `myelin-utils` no longer carry that legacy serializer for
  native builds
- CKB `Script` / `OutPoint` / `CellInput` / `CellOutput` / `CellDep` / `RawTransaction` / `Transaction` / `WitnessArgs` / `RawHeader` / `Header` / packed `EpochNumberWithFraction` helpers are available for CKB-profile byte and hash material, including zeroed-lock `SIGHASH_ALL`, header hash, Blake160 pubkey hashes, local CKB Blake160 recoverable-signature verification, and local zeroed-lock CKB sighash-all witness verification
- `VmSemantics::CkbStrict` uses provider-supplied CKB `Header` bytes for `LOAD_HEADER` and CKB epoch fields for `LOAD_HEADER_BY_FIELD`; it does not fall back to Myelin `ResolvedHeader`
- CellScript metadata 通过 `runtime.vm_abi.version = 0x8001` 声明所需 VM object ABI
- CellScript scheduler witness 的 public admission 只接受 Molecule bytes；legacy witness decode path 已移除
- RISC-V ELF artifact 可以内嵌固定 ABI trailer；verifier/loader 在交给 CKB-VM 前 strip trailer，并据此选择 Molecule syscall 输出格式
- verifier caller 仍可以用 `with_abi_version(0x8001)` 将 artifact metadata 映射到 Molecule syscall 输出格式
- TransactionScriptVerifier 使用 Molecule；Non-Molecule VM object ABI versions are rejected
- 非 ELF artifact 的 sidecar metadata 不是链上自动事实；仍需嵌入或认证 artifact ABI manifest
