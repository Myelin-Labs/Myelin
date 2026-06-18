// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Macros
//
//! # 序列化宏
//!
//! 本模块提供便捷的宏来简化序列化代码的编写。
//!
//! ## 声明宏 (Declarative Macros)
//!
//! 这些宏在编译时展开，无需单独的 proc-macro crate。

/// 为类型实现 VersionedSerializable trait
///
/// # Example
///
/// ```rust
/// use spora_exec::{impl_versioned_serializable, VersionedSerializable};
/// use borsh::{BorshSerialize, BorshDeserialize};
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct MyData {
///     value: u64,
/// }
///
/// impl_versioned_serializable!(MyData, 1);
///
/// assert_eq!(MyData::CURRENT_VERSION, 1);
/// ```
#[macro_export]
macro_rules! impl_versioned_serializable {
    ($type:ty, $version:expr) => {
        impl $crate::serialization::VersionedSerializable for $type {
            const CURRENT_VERSION: u8 = $version;
        }
    };
}

/// 为类型实现 VmSerializable trait
///
/// # Example
///
/// ```rust
/// use spora_exec::{impl_vm_serializable, VmSerializable};
/// use borsh::{BorshSerialize, BorshDeserialize};
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct MyVmData {
///     value: u64,
/// }
///
/// impl_vm_serializable!(MyVmData, 0x0001);
///
/// assert_eq!(MyVmData::abi_version(), 0x0001);
/// ```
#[macro_export]
macro_rules! impl_vm_serializable {
    ($type:ty, $abi_version:expr) => {
        impl $crate::serialization::VmSerializable for $type {
            fn to_vm_bytes(&self) -> Vec<u8> {
                // Borsh 序列化理论上不应失败，但如果失败，返回空 Vec 而不是 panic
                // 调用者应检查返回的 Vec 是否为空
                borsh::to_vec(self).unwrap_or_default()
            }

            fn from_vm_bytes(bytes: &[u8]) -> Result<Self, $crate::serialization::VmAbiError> {
                use borsh::BorshDeserialize;
                BorshDeserialize::try_from_slice(bytes)
                    .map_err(|e| $crate::serialization::VmAbiError::DeserializationFailed(e.to_string()))
            }

            fn abi_version() -> u16 {
                $abi_version
            }
        }
    };
}

/// 为多个类型批量实现 VersionedSerializable
///
/// # Example
///
/// ```rust
/// use spora_exec::{impl_versioned_serializable_batch};
/// use borsh::{BorshSerialize, BorshDeserialize};
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeA { value: u32 }
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeB { value: u64 }
///
/// impl_versioned_serializable_batch! {
///     (TypeA, 1),
///     (TypeB, 1)
/// }
/// ```
#[macro_export]
macro_rules! impl_versioned_serializable_batch {
    ($(($type:ty, $version:expr)),+ $(,)?) => {
        $(
            $crate::impl_versioned_serializable!($type, $version);
        )+
    };
}

/// 为多个类型批量实现 VmSerializable
///
/// # Example
///
/// ```rust
/// use spora_exec::{impl_vm_serializable_batch};
/// use borsh::{BorshSerialize, BorshDeserialize};
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeA { value: u32 }
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeB { value: u64 }
///
/// impl_vm_serializable_batch! {
///     (TypeA, 0x0001),
///     (TypeB, 0x0001)
/// }
/// ```
#[macro_export]
macro_rules! impl_vm_serializable_batch {
    ($(($type:ty, $abi_version:expr)),+ $(,)?) => {
        $(
            $crate::impl_vm_serializable!($type, $abi_version);
        )+
    };
}

/// 快速创建 VersionedEnvelope
///
/// # Example
///
/// ```rust
/// use spora_exec::{envelope, CellOutput, Script};
///
/// let output = CellOutput {
///     lock: Script::new([0xAA; 32], 0, vec![]),
///     type_: None,
///     capacity: 1000,
/// };
///
/// let envelope = envelope!(output).unwrap();
/// ```
#[macro_export]
macro_rules! envelope {
    ($value:expr) => {
        $crate::serialization::VersionedEnvelope::new(&$value)
    };
}

/// 快速序列化到字节
///
/// # Example
///
/// ```rust
/// use spora_exec::{serialize, CellOutput, Script};
///
/// let output = CellOutput {
///     lock: Script::new([0xAA; 32], 0, vec![]),
///     type_: None,
///     capacity: 1000,
/// };
///
/// let bytes = serialize!(output).unwrap();
/// ```
#[macro_export]
macro_rules! serialize {
    ($value:expr) => {
        $crate::serialization::utils::serialize_to_bytes(&$value)
    };
}

/// 快速从字节反序列化
///
/// # Example
///
/// ```rust
/// use spora_exec::{serialize, deserialize, CellOutput, Script};
///
/// # let output = CellOutput {
/// #     lock: Script::new([0xAA; 32], 0, vec![]),
/// #     type_: None,
/// #     capacity: 1000,
/// # };
/// # let bytes = serialize!(output).unwrap();
/// let restored: CellOutput = deserialize!(&bytes).unwrap();
/// ```
#[macro_export]
macro_rules! deserialize {
    ($bytes:expr, $type:ty) => {
        $crate::serialization::utils::deserialize_from_bytes::<$type>($bytes)
    };
    ($bytes:expr) => {
        $crate::serialization::utils::deserialize_from_bytes($bytes)
    };
}

/// 定义版本常量并批量实现 VersionedSerializable
///
/// # Example
///
/// ```rust
/// use spora_exec::{define_schema_version, VersionedSerializable};
/// use borsh::{BorshSerialize, BorshDeserialize};
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeA { value: u32 }
///
/// #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
/// struct TypeB { value: u64 }
///
/// define_schema_version!(MY_SCHEMA_VERSION = 1, TypeA, TypeB);
///
/// assert_eq!(MY_SCHEMA_VERSION, 1);
/// assert_eq!(<TypeA as VersionedSerializable>::CURRENT_VERSION, 1);
/// assert_eq!(<TypeB as VersionedSerializable>::CURRENT_VERSION, 1);
/// ```
#[macro_export]
macro_rules! define_schema_version {
    ($const_name:ident = $version:expr, $($type:ty),+ $(,)?) => {
        pub const $const_name: u8 = $version;

        $(
            $crate::impl_versioned_serializable!($type, $version);
        )+
    };
}

#[cfg(test)]
mod tests {
    use crate::serialization::{VersionedSerializable, VmSerializable};
    use borsh::{BorshDeserialize, BorshSerialize};

    #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq, Hash)]
    struct TestData {
        value: u64,
    }

    #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
    struct TestVmData {
        value: u64,
    }

    impl_versioned_serializable!(TestData, 5);
    impl_vm_serializable!(TestVmData, 0x1234);

    #[test]
    fn test_impl_versioned_serializable() {
        assert_eq!(TestData::CURRENT_VERSION, 5);

        let data = TestData { value: 42 };
        assert_eq!(data.version(), 5);
    }

    #[test]
    fn test_impl_vm_serializable() {
        assert_eq!(TestVmData::abi_version(), 0x1234);

        let data = TestVmData { value: 42 };
        let bytes = data.to_vm_bytes();
        let restored = TestVmData::from_vm_bytes(&bytes).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_impl_versioned_serializable_batch() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct TypeA {
            value: u32,
        }

        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct TypeB {
            value: u64,
        }

        impl_versioned_serializable_batch! {
            (TypeA, 1),
            (TypeB, 2)
        }

        assert_eq!(TypeA::CURRENT_VERSION, 1);
        assert_eq!(TypeB::CURRENT_VERSION, 2);
    }

    #[test]
    fn test_envelope_macro() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct EnvelopeData {
            value: u64,
        }
        impl_versioned_serializable!(EnvelopeData, 1);

        let data = EnvelopeData { value: 42 };
        let envelope = envelope!(data).unwrap();
        assert_eq!(envelope.schema_version(), 1);
    }

    #[test]
    fn test_serialize_macro() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct SerializeData {
            value: u64,
        }
        impl_versioned_serializable!(SerializeData, 1);

        let data = SerializeData { value: 42 };
        let bytes = serialize!(data).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_deserialize_macro() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct DeserializeData {
            value: u64,
        }
        impl_versioned_serializable!(DeserializeData, 1);

        let data = DeserializeData { value: 42 };
        let bytes = serialize!(data).unwrap();
        let restored: DeserializeData = deserialize!(&bytes).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_deserialize_macro_with_type() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct DeserializeTypedData {
            value: u64,
        }
        impl_versioned_serializable!(DeserializeTypedData, 1);

        let data = DeserializeTypedData { value: 42 };
        let bytes = serialize!(data).unwrap();
        let restored = deserialize!(&bytes, DeserializeTypedData).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_define_schema_version() {
        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct SchemaTypeA {
            value: u32,
        }

        #[derive(BorshSerialize, BorshDeserialize, Clone, Debug, PartialEq, Eq)]
        struct SchemaTypeB {
            value: u64,
        }

        define_schema_version!(TEST_SCHEMA_VERSION = 3, SchemaTypeA, SchemaTypeB);

        assert_eq!(TEST_SCHEMA_VERSION, 3);
        assert_eq!(SchemaTypeA::CURRENT_VERSION, 3);
        assert_eq!(SchemaTypeB::CURRENT_VERSION, 3);
    }
}
