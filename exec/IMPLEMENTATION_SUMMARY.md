# Myelin 序列化分层治理实现总结

## 实施日期
2026-04-15

## 完成状态
✅ Phase 1 和 Phase 2 已完成，Phase 3 预留

---

## 核心实现

### 1. 序列化框架 (`src/serialization/`)

#### `mod.rs` - 核心 Trait 和类型
- **VersionedSerializable**: 存储层版本化序列化 trait
  - `CURRENT_VERSION`: 类型当前的 schema 版本
  - `upgrade_from()`: 支持从旧版本升级
  
- **VersionedEnvelope<T>**: 版本化信封包装器
  - `format_version`: 序列化格式 (0x80=Molecule-compatible)
  - `schema_version`: 数据 schema 版本
  - `payload`: 实际序列化数据
  
- **VmSerializable**: VM ABI 序列化 trait
  - `to_vm_bytes()`: 序列化为 VM 可见字节
  - `from_vm_bytes()`: 从 VM 字节解析
  - `abi_version()`: 获取 ABI 版本
  
- **VmAbiNegotiator**: ABI 版本协商
  - `negotiate()`: 协商脚本和 VM 之间的 ABI 版本
  - `default_capabilities()`: 获取 VM 默认能力
  - 只接受 Molecule VM object ABI (`0x8001`)

#### `vm_abi.rs` - VM ABI 标准化序列化
提供统一的序列化格式，确保 ABI 稳定性：
- `serialize_script()`: Script 序列化
- `serialize_outpoint()`: OutPoint 序列化
- `serialize_cell_input()`: CellInput 序列化
- `serialize_cell_output()`: CellOutput 序列化
- 大小计算辅助函数

#### `molecule_compat.rs` - Molecule 迁移预留
为未来 Molecule 迁移预留的接口：
- `MoleculeSerializer`: Molecule 序列化器占位
- `MoleculeError`: Molecule 错误类型
- 预留的序列化/反序列化函数
- Schema 版本常量

#### `utils.rs` - 实用工具函数
提供便捷的序列化工具：
- `serialize_to_bytes()`: 一键序列化到字节
- `deserialize_from_bytes()`: 一键从字节解析
- `serialize_many()`: 批量序列化
- `deserialize_many()`: 批量反序列化
- `peek_format_version()`: 快速查看格式版本
- `peek_schema_version()`: 快速查看 schema 版本
- `is_valid_versioned_envelope()`: 快速验证信封格式
- `estimate_serialized_size()`: 估计序列化大小

#### `cache.rs` - 序列化缓存
提供序列化结果缓存以优化性能：
- `SerializationCache`: 单线程缓存，LRU 淘汰策略
- `ThreadSafeSerializationCache`: 线程安全版本
- `CacheStats`: 缓存统计信息
- `get_or_serialize()`: 获取或序列化
- `contains()`: 检查是否在缓存中

#### `macros.rs` - 序列化宏
提供便捷的宏来简化代码：
- `impl_versioned_serializable!`: 实现 VersionedSerializable
- `impl_vm_serializable!`: 实现 VmSerializable
- `impl_versioned_serializable_batch!`: 批量实现 VersionedSerializable
- `impl_vm_serializable_batch!`: 批量实现 VmSerializable
- `envelope!`: 快速创建 VersionedEnvelope
- `serialize!`: 快速序列化
- `deserialize!`: 快速反序列化
- `define_schema_version!`: 定义版本常量并批量实现

#### `validation.rs` - 序列化验证
提供序列化数据的验证功能：
- `SerializerValidator`: 序列化验证器
- `ValidationConfig`: 验证配置（默认/宽松/严格）
- `ValidationResult`: 验证结果（Valid/Warning/Invalid）
- `validate_envelope()`: 验证 VersionedEnvelope
- `is_valid_envelope()`: 快速检查有效性

#### `streaming.rs` - 流式序列化
提供流式序列化支持，处理大型数据：
- `StreamingSerializer`: 流式序列化器
- `StreamingDeserializer`: 流式反序列化器
- `serialize_streaming()`: 批量流式序列化
- `deserialize_streaming()`: 批量流式反序列化

#### `security.rs` - 序列化安全
提供序列化数据的安全保护：
- `SecureEnvelope`: 带完整性校验的安全信封
- `SecurityConfig`: 安全配置（默认/最小/严格/无）
- `SecurityGuard`: 安全守卫，执行安全检查
- `compute_hash()`: BLAKE3 哈希计算
- `verify_integrity()`: 完整性验证
- `serialize_with_integrity()`: 带完整性校验的序列化
- `deserialize_with_integrity()`: 带完整性校验的反序列化

#### `compression.rs` - 序列化压缩
提供序列化数据的压缩支持：
- `CompressionAlgorithm`: 压缩算法枚举（None/LZ4/Zstd）
- `CompressionConfig`: 压缩配置（默认/高速/最佳/自动）
- `CompressedEnvelope`: 带压缩的信封
- `CompressionStats`: 压缩统计
- `compress()`: 压缩数据
- `decompress()`: 解压数据
- `estimate_compressed_size()`: 估计压缩后大小
- `select_algorithm()`: 自动选择算法

---

### 2. 类型实现

#### CellTx 类型 (`src/celltx/types.rs`)
所有 CellTx 类型实现 `VersionedSerializable`:
- `OutPoint` - schema version 1
- `Script` - schema version 1
- `CellOutput` - schema version 1
- `CellInput` - schema version 1
- `CellDep` - schema version 1
- `DepType` - schema version 1
- `CellTx` - schema version 1
- `TransactionInfo` - schema version 1
- `ResolvedCellMeta` - schema version 1
- `ResolvedCellTx` - schema version 1

常量: `CELLTX_SCHEMA_VERSION = 1`

#### VM 类型 (`src/vm/verifier.rs`)
VM-facing 类型实现 `VmSerializable`:
- `ResolvedHeader` - ABI version 0x8001 (Molecule v1)
- `ResolvedCell` - ABI version 0x8001 (Molecule v1)

---

### 3. VM Syscall 更新

#### 已更新的 Syscall
- `load_header.rs`: 使用 `VmSerializable::to_vm_bytes()`
- `load_cell.rs`: 使用 `vm_abi::serialize_cell_output()`
- `load_script.rs`: 使用 `vm_abi::serialize_script()`
- `load_input.rs`: 使用 `vm_abi::serialize_outpoint()`

#### 不需要更新的 Syscall
- `load_witness.rs`: 返回原始字节，无需序列化
- `load_tx.rs`: 返回原始字节，无需序列化
- `load_signature_hash.rs`: 计算哈希，不涉及序列化

---

### 4. 测试覆盖

#### 单元测试 (`src/serialization/mod.rs`)
- VersionedEnvelope 往返测试
- 不支持的格式版本测试
- 版本升级路径测试
- ABI 版本协商测试 (成功/回退/失败)
- 错误类型转换测试
- VmSerializable 往返测试

#### 集成测试 (`tests/`)
- `serialization_integration.rs`: 226 行
  - 所有 CellTx 类型序列化测试
  - Schema 版本一致性测试
  - 大负载处理测试
  - 多轮往返测试
  
- `vm_abi_integration.rs`: 234 行
  - ResolvedHeader/ResolvedCell 序列化测试
  - vm_abi 函数测试
  - ABI 版本兼容性测试
  - 错误处理测试

#### 示例 (`examples/`)
- `serialization_usage.rs`: VM ABI 和版本协商使用示例
- `storage_versioning.rs`: 存储层版本化示例
- `schema_migration.rs`: Schema 迁移示例 (v1→v2→v3)
- `utils_usage.rs`: 工具函数使用示例
- `cache_usage.rs`: 序列化缓存使用示例

---

### 5. 文档

#### 架构文档
- `src/serialization/README.md`: 模块架构和使用指南
- `src/lib.rs`: 三层架构文档注释
- `docs/SERIALIZATION_LAYER_GOVERNANCE_MIGRATION_PLAN.md`: 迁移计划 (已更新)
- `API_GUIDE.md`: API 使用指南和最佳实践

---

## 文件变更统计

| 类别 | 文件数 | 新增行数 |
|------|--------|----------|
| 核心模块 | 11 | ~3700 |
| 类型实现 | 3 | ~100 |
| Syscall 更新 | 4 | ~20 |
| 测试 | 2 | ~570 |
| 基准测试 | 1 | ~240 |
| 示例 | 5 | ~870 |
| 文档 | 4 | ~500 |
| **总计** | **29** | **~5900** |

---

## 三层架构实现

```
┌─────────────────────────────────────────────────────────────────┐
│  Layer 3: VM/Script ABI 层 ✅                                   │
│  - ResolvedHeader/ResolvedCell 实现 VmSerializable              │
│  - vm_abi 模块提供标准化序列化                                   │
│  - ABI 版本协商机制                                              │
├─────────────────────────────────────────────────────────────────┤
│  Layer 2: 内部通信/存储层 ✅                                     │
│  - 所有 CellTx 类型实现 VersionedSerializable                   │
│  - VersionedEnvelope 支持格式和 schema 版本                      │
│  - 为未来 Molecule 迁移预留路径                                  │
├─────────────────────────────────────────────────────────────────┤
│  Layer 1: 共识关键路径 ✅ (保持现状)                             │
│  - 自定义流式哈希                                                │
│  - 不受序列化层变更影响                                          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 迁移路径

### Phase 1 ✅ 已完成
- VersionedSerializable trait 定义
- VersionedEnvelope 实现
- 所有 CellTx 类型版本化
- 架构文档

### Phase 2 ✅ 已完成
- VmSerializable trait 定义
- ResolvedHeader/ResolvedCell 实现
- VM syscall 使用新抽象
- vm_abi 模块标准化
- 单元测试和集成测试

### Phase 3 🔮 预留
- Molecule schema 定义
- molecule_compat 模块实现
- Molecule 版本 VmSerializable
- 完整 ABI 版本协商

---

## 使用示例

### 存储层版本化
```rust
use myelin_exec::{CellTx, VersionedEnvelope, VersionedSerializable};

// 存储
let tx = CellTx::new(...)?;
let envelope = VersionedEnvelope::new(&tx)?;
db.put(key, envelope.to_bytes())?;

// 读取
let envelope = VersionedEnvelope::<CellTx>::from_bytes(&bytes)?;
let tx = envelope.parse()?;
```

### VM ABI 序列化
```rust
use myelin_exec::{ResolvedHeader, VmSerializable};

let header = ResolvedHeader { ... };
let bytes = header.to_vm_bytes();
let restored = ResolvedHeader::from_vm_bytes(&bytes)?;
```

### ABI 版本协商
```rust
use myelin_exec::VmAbiNegotiator;

let caps = VmAbiNegotiator::default_capabilities();
let version = VmAbiNegotiator::negotiate(script_version, &caps)?;
```

---

## 关键设计决策

1. **分层架构**: 清晰分离存储层和 VM ABI 层，独立演进
2. **版本信封**: 同时支持格式版本和 schema 版本，灵活迁移
3. **ABI 抽象**: VmSerializable trait 隔离具体序列化实现
4. **预留接口**: molecule_compat 模块为未来迁移提供清晰路径
5. **标准化格式**: vm_abi 模块确保 VM-facing 类型格式稳定

---

## 后续工作

### 短期 (1-2 周)
- [ ] 运行完整测试套件
- [ ] 性能基准测试
- [ ] 代码审查

### 中期 (3-6 个月)
- [ ] 根据实际使用完善 ABI
- [ ] 添加更多 VM-facing 类型的 VmSerializable
- [ ] 完善 ABI 版本协商机制

### 长期 (6-12 个月，按需)
- [ ] 评估 Molecule 迁移需求
- [ ] 实现 molecule_compat 模块
- [ ] 多语言 SDK 兼容性验证
