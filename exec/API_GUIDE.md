# Spora 序列化 API 指南

## 快速开始

### 基本序列化

```rust
use spora_exec::{CellOutput, Script, serialize_to_bytes, deserialize_from_bytes};

let output = CellOutput {
    lock: Script::new([0xAA; 32], 0, vec![]),
    type_: None,
    capacity: 1000,
};

// 序列化
let bytes = serialize_to_bytes(&output)?;

// 反序列化
let restored: CellOutput = deserialize_from_bytes(&bytes)?;
```

### 使用宏简化

```rust
use spora_exec::{impl_versioned_serializable, envelope, serialize};
use borsh::{BorshSerialize, BorshDeserialize};

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
struct MyData { value: u64 }

impl_versioned_serializable!(MyData, 1);

let data = MyData { value: 42 };
let bytes = serialize!(data)?;
```

## 核心 Trait

### VersionedSerializable

用于存储层版本化序列化：

```rust
use spora_exec::VersionedSerializable;

impl VersionedSerializable for MyData {
    const CURRENT_VERSION: u8 = 1;
    
    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
        // 实现版本升级逻辑
        match version {
            1 => borsh::from_slice(bytes)
                .map_err(|e| SerializationError::DeserializationFailed(e.to_string())),
            _ => Err(SerializationError::UpgradePathNotAvailable { from: version, to: 1 }),
        }
    }
}
```

### VmSerializable

用于 VM ABI 序列化：

```rust
use spora_exec::VmSerializable;

impl VmSerializable for MyData {
    fn to_vm_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("serialization should not fail")
    }
    
    fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError> {
        borsh::from_slice(bytes)
            .map_err(|e| VmAbiError::DeserializationFailed(e.to_string()))
    }
    
    fn abi_version() -> u16 { 0x0001 }
}
```

## 高级功能

### 使用缓存

```rust
use spora_exec::SerializationCache;

let mut cache = SerializationCache::new(1000);

// 第一次会序列化
let bytes1 = cache.get_or_serialize(&data)?;

// 第二次直接返回缓存
let bytes2 = cache.get_or_serialize(&data)?;
assert_eq!(bytes1.as_ptr(), bytes2.as_ptr()); // 同一内存
```

### 流式处理

```rust
use spora_exec::{StreamingSerializer, StreamingDeserializer};

// 序列化到文件
let file = std::fs::File::create("data.bin")?;
let mut serializer = StreamingSerializer::new(file);
serializer.serialize(&data)?;

// 从文件反序列化
let file = std::fs::File::open("data.bin")?;
let mut deserializer = StreamingDeserializer::new(file);
let data: MyData = deserializer.deserialize()?;
```

### 安全信封

```rust
use spora_exec::{SecureEnvelope, serialize_with_integrity};

// 带完整性校验的序列化
let envelope = serialize_with_integrity(&data)?;

// 验证完整性
assert!(envelope.verify());

// 序列化为字节
let bytes = envelope.to_bytes();

// 解析并验证
let envelope = SecureEnvelope::from_bytes(&bytes)?;
let data: MyData = borsh::from_slice(&envelope.data)?;
```

### 压缩

```rust
use spora_exec::{CompressionConfig, CompressedEnvelope};

let config = CompressionConfig::default(); // Zstd 级别 3
let data = vec![0u8; 10000];

let envelope = CompressedEnvelope::compress(&data, &config)?;
println!("压缩率: {:.1}%", envelope.compression_ratio() * 100.0);

let restored = envelope.decompress()?;
```

### 验证

```rust
use spora_exec::{SerializerValidator, ValidationConfig};

let config = ValidationConfig::strict();
let validator = SerializerValidator::new(config);

let bytes = serialize_to_bytes(&data)?;
let result = validator.validate_envelope(&bytes);

match result {
    ValidationResult::Valid => println!("验证通过"),
    ValidationResult::Warning(msg) => println!("警告: {}", msg),
    ValidationResult::Invalid(msg) => println!("无效: {}", msg),
}
```

## 配置模式

### 安全配置

```rust
use spora_exec::SecurityConfig;

let config = SecurityConfig::default();  // 标准安全
let config = SecurityConfig::strict();   // 严格安全
let config = SecurityConfig::minimal();  // 最小安全
let config = SecurityConfig::none();     // 无安全（仅测试）
```

### 压缩配置

```rust
use spora_exec::CompressionConfig;

let config = CompressionConfig::default(); // Zstd 级别 3
let config = CompressionConfig::fast();    // LZ4
let config = CompressionConfig::best();    // Zstd 级别 19
let config = CompressionConfig::none();    // 无压缩
let config = CompressionConfig::auto();    // 自动选择
```

### 验证配置

```rust
use spora_exec::ValidationConfig;

let config = ValidationConfig::default();    // 标准验证
let config = ValidationConfig::permissive(); // 宽松验证
let config = ValidationConfig::strict();     // 严格验证
```

## 最佳实践

### 1. 选择合适的序列化方式

- **存储层**: 使用 `VersionedSerializable` + `VersionedEnvelope`
- **VM ABI**: 使用 `VmSerializable` + `vm_abi` 模块
- **网络传输**: 考虑使用 `SecureEnvelope` 或 `CompressedEnvelope`

### 2. 版本管理

```rust
// 定义 schema 版本常量
pub const MY_SCHEMA_VERSION: u8 = 1;

// 批量实现
impl_versioned_serializable_batch! {
    (TypeA, MY_SCHEMA_VERSION),
    (TypeB, MY_SCHEMA_VERSION),
}
```

### 3. 性能优化

```rust
// 使用缓存
let mut cache = SerializationCache::new(1000);

// 批量序列化
let bytes = serialize_many(&items)?;

// 流式处理大文件
let mut serializer = StreamingSerializer::new(file);
```

### 4. 错误处理

```rust
use spora_exec::SerializationError;

match result {
    Err(SerializationError::UnsupportedVersion(v)) => {
        eprintln!("不支持的版本: {}", v);
    }
    Err(SerializationError::UpgradePathNotAvailable { from, to }) => {
        eprintln!("无法从版本 {} 升级到 {}", from, to);
    }
    Err(e) => {
        eprintln!("序列化错误: {}", e);
    }
    Ok(data) => {
        // 处理数据
    }
}
```

## 完整示例

```rust
use spora_exec::*;
use borsh::{BorshSerialize, BorshDeserialize};

#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
struct Transaction {
    inputs: Vec<CellInput>,
    outputs: Vec<CellOutput>,
}

impl_versioned_serializable!(Transaction, 1);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 创建交易
    let tx = Transaction {
        inputs: vec![/* ... */],
        outputs: vec![/* ... */],
    };
    
    // 1. 基本序列化
    let bytes = serialize_to_bytes(&tx)?;
    
    // 2. 带完整性校验
    let secure = serialize_with_integrity(&tx)?;
    let bytes = secure.to_bytes();
    
    // 3. 带压缩
    let config = CompressionConfig::default();
    let compressed = CompressedEnvelope::compress(&bytes, &config)?;
    
    // 4. 验证
    let validator = SerializerValidator::default();
    assert!(validator.is_valid_envelope(&bytes));
    
    // 5. 使用缓存
    let mut cache = SerializationCache::new(100);
    let _ = cache.get_or_serialize(&tx)?;
    
    println!("序列化完成！");
    Ok(())
}
```
