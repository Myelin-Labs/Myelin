# Spora 序列化分层治理

本文档描述 Spora 执行层的序列化架构。

## 架构概述

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: VM/Script ABI 层 (Molecule v1 public + legacy v1)     │
│  - ResolvedHeader, ResolvedCell, Witness Payload                │
│  - 脚本可见的所有数据结构                                        │
│  - 需要: canonical, partial read, version兼容                  │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: 内部通信/存储层 (保持 Borsh + Version Envelope)       │
│  - P2P消息 (Protobuf 已覆盖)                                    │
│  - 钱包/节点 RPC (已有 JSON/Borsh 双轨)                         │
│  - RocksDB 存储 (加 version envelope)                          │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: 共识关键路径 (已绕过 Borsh，保持现状)                 │
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
- [x] Spora VM `ResolvedHeader` / `ResolvedCell` Molecule wire layout
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
use spora_exec::{VersionedSerializable, VersionedEnvelope};

// 类型自动实现 VersionedSerializable
let tx = CellTx::new(...);

// 包装到 VersionedEnvelope
let envelope = VersionedEnvelope::new(&tx)?;

// 存储到 RocksDB
db.put(key, borsh::to_vec(&envelope)?)?;

// 从 RocksDB 读取并解析
let envelope: VersionedEnvelope<CellTx> = borsh::from_slice(&bytes)?;
let tx = envelope.parse()?;
```

### VmSerializable

为 VM-facing 类型提供 public Molecule ABI 抽象。显式 legacy Borsh/custom v1 路径保留在 `VmAbiFormat::Legacy`
和旧 serializer helpers 中，不再是 `VmSerializable` 的默认语义：

```rust
use spora_exec::{VmSerializable, ResolvedHeader};

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
- `0x0001`: Borsh/custom ABI v1 (explicit legacy compatibility only)
- 使用 `VmAbiNegotiator` 协商版本
- 使用 `VmAbiFormat` 在 VM runtime/syscall 边界选择实际 wire format

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
- ✅ `VmSerializable` 的 public bytes 使用 Molecule；VM syscall 在 runtime 边界通过 `VmAbiFormat` 选择 Molecule 或显式 legacy
- ✅ `vm_abi` 模块标准化序列化格式

### Phase 2 (未来 3-6 个月)
- 完善 ABI 版本协商机制
- 支持脚本指定 ABI 版本
- 添加更多 VM-facing 类型的 `VmSerializable` 实现

### Phase 3 (当前推进)
- `molecule_compat` 已实现 canonical Molecule wire layout，不再是 NotImplemented 占位
- `LOAD_SCRIPT` / `LOAD_INPUT` / `LOAD_CELL` / `LOAD_HEADER` full-load 路径已支持 `VmAbiFormat::Molecule`
- `LoadScript` / `LoadInput` / `LoadCell` / `LoadHeader` constructors and `TransactionScriptVerifier` now default to Molecule
- CKB `Script` / `OutPoint` / `CellInput` / `CellOutput` / `CellDep` / `RawTransaction` / `Transaction` / `WitnessArgs` / `RawHeader` / `Header` / packed `EpochNumberWithFraction` helpers are available for CKB-profile byte and hash material, including zeroed-lock `SIGHASH_ALL`, header hash, Blake160 pubkey hashes, local CKB Blake160 recoverable-signature verification, and local zeroed-lock CKB sighash-all witness verification
- `VmSemantics::CkbStrict` uses provider-supplied CKB `Header` bytes for `LOAD_HEADER` and CKB epoch fields for `LOAD_HEADER_BY_FIELD`; it does not fall back to Spora `ResolvedHeader`
- CellScript metadata 通过 `runtime.vm_abi.version = 0x8001` 声明所需 VM object ABI
- CellScript scheduler witness 的 public admission 只接受 Molecule bytes；legacy Borsh witness decode 只保留为显式迁移/回归入口
- RISC-V ELF artifact 可以内嵌固定 ABI trailer；verifier/loader 在交给 CKB-VM 前 strip trailer，并据此选择 Molecule syscall 输出格式
- verifier caller 仍可以用 `with_abi_version(0x8001)` 将 artifact metadata 映射到 Molecule syscall 输出格式
- TransactionScriptVerifier 默认使用 Molecule；Borsh/custom ABI v1 必须通过 explicit legacy ABI 选择
- 非 ELF artifact 的 sidecar metadata 不是链上自动事实；仍需嵌入或认证 artifact ABI manifest
- 保持分层架构，不做全栈 Molecule 切换
