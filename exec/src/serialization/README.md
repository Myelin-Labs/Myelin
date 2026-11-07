# Myelin 序列化

执行层使用三个明确的编码边界：

- 共识哈希采用各自的 domain-separated 流式编码。
- 存储工具通过 `VersionedEnvelope<T>` 包装当前类型的显式 payload。
- 脚本可见对象使用 canonical Molecule，唯一 ABI 版本为 `VM_ABI_VERSION = 0x8001`。

## 存储格式

`VersionedSerializable` 的 `to_versioned_payload` 和 `from_versioned_payload`
都是必需方法；未提供编解码器的类型不能实现该 trait。`VersionedEnvelope`
只接受格式 `0x80` 和该类型的 `CURRENT_VERSION`，不提供历史格式导入或升级。

`OutPoint`、`Script`、`CellInput`、`CellOutput`、`CellDep`、`DepType` 和 `CellTx`
使用显式 Molecule payload。解析后的执行上下文类型没有占位存储实现。

```rust
use myelin_exec::{CellOutput, Script, VersionedEnvelope};

let output = CellOutput {
    lock: Script::new([0xAA; 32], 0, vec![]),
    type_: None,
    capacity: 1000,
};
let bytes = VersionedEnvelope::new(&output)?.to_bytes();
let restored = VersionedEnvelope::<CellOutput>::from_bytes(&bytes)?.parse()?;
assert_eq!(output, restored);
```

`serialize_to_bytes`、流式序列化和缓存使用同一个 envelope 编码。
批量数据统一使用 `serialize_streaming` / `deserialize_streaming`：条目数后直接
连接当前 envelope，不再支持额外套一层长度的批量格式。检查元数据直接使用
`VersionedEnvelope`；类型合法性以 `parse()` 为准，字节大小直接读取编码结果长度。
`SecureEnvelope` 也只接受其当前 Molecule table 布局。

## VM ABI

`VmSerializable` 只提供 `to_vm_bytes` 和 `from_vm_bytes`。`ResolvedHeader`
和 `ResolvedCell` 直接使用 Molecule 编解码器；运行时没有多版本协商器或旧 raw 编码路径。

`LOAD_SCRIPT`、`LOAD_INPUT`、`LOAD_CELL`、`LOAD_HEADER` 的完整对象读取使用
当前 Molecule 编码。不存在 ABI 选择器或协商接口；artifact trailer 只接受
`VM_ABI_VERSION`（`0x8001`），其余版本直接报错。

构建产物元数据可以声明 VM ABI。RISC-V ELF 可以附带
固定 ABI trailer；代码哈希覆盖完整 artifact，loader 在交给 CKB-VM 前验证并去掉
trailer。普通 CKB ELF 无需 Myelin trailer。非 ELF artifact 的 sidecar metadata
仍需单独认证，不能自动成为链上事实。

`VmSemantics::CkbStrict` 的 header 读取使用 provider 提供的 CKB `Header` 字节
和 epoch 字段。`molecule_compat` 同时提供 CKB 交易、脚本、WitnessArgs、Header、
签名和 TYPE_ID 编码工具；这些是当前 CKB 集成接口。

## 模块

| 模块 | 职责 |
| --- | --- |
| `mod.rs` | 当前 envelope、序列化 trait 和 ABI trailer |
| `molecule_compat.rs` | canonical Molecule 对象与 CKB 编码/验证 |
| `utils.rs` | 单条 envelope 的类型化编码和解码 |
| `streaming.rs` | 流式读写 |
| `cache.rs` | 序列化结果缓存 |
| `security.rs` | 有界安全 envelope |

Scheduler plans 是绑定原始交易的链下 sidecar，不从交易 witness 解码。
