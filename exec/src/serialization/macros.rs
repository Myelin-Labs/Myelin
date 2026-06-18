// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
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
/// use myelin_exec::{impl_versioned_serializable, VersionedSerializable};
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
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

/// Disabled legacy helper for implementing `VmSerializable`.
///
/// This macro used to emit legacy VM bytes. Public VM ABI is now Molecule-only
/// by default, so new VM-visible types must implement `VmSerializable`
/// explicitly and return a Molecule ABI version.
#[macro_export]
macro_rules! impl_vm_serializable {
    ($($tokens:tt)*) => {
        compile_error!(
            "impl_vm_serializable! was removed because it emitted legacy VM bytes. \
             Implement VmSerializable explicitly with Molecule bytes instead."
        );
    };
}

/// 为多个类型批量实现 VersionedSerializable
///
/// # Example
///
/// ```rust
/// use myelin_exec::{impl_versioned_serializable_batch};
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
/// struct TypeA { value: u32 }
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
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

/// Disabled batch helper for legacy VM ABI implementations.
#[macro_export]
macro_rules! impl_vm_serializable_batch {
    ($($tokens:tt)*) => {
        compile_error!(
            "impl_vm_serializable_batch! was removed because it emitted legacy VM bytes. \
             Implement VmSerializable explicitly with Molecule bytes instead."
        );
    };
}

/// 快速创建 VersionedEnvelope
///
/// # Example
///
/// ```rust
/// use myelin_exec::{envelope, CellOutput, Script};
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
/// use myelin_exec::{serialize, CellOutput, Script};
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
/// use myelin_exec::{serialize, deserialize, CellOutput, Script};
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
/// use myelin_exec::{define_schema_version, VersionedSerializable};
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
/// struct TypeA { value: u32 }
///
/// #[derive(Clone, Debug, PartialEq, Eq)]
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
    use crate::serialization::{SerializationError, VersionedSerializable};

    #[derive(Clone, Debug, PartialEq, Eq, Hash)]
    struct TestData {
        value: u64,
    }

    impl_versioned_serializable!(TestData, 5);

    #[test]
    fn test_impl_versioned_serializable() {
        assert_eq!(TestData::CURRENT_VERSION, 5);

        let data = TestData { value: 42 };
        assert_eq!(data.version(), 5);
    }

    #[test]
    fn test_impl_versioned_serializable_batch() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct TypeA {
            value: u32,
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
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
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct EnvelopeData {
            value: u64,
        }
        impl VersionedSerializable for EnvelopeData {
            const CURRENT_VERSION: u8 = 1;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(self.value.to_le_bytes().to_vec())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                decode_u64_payload(version, bytes).map(|value| Self { value })
            }
        }

        let data = EnvelopeData { value: 42 };
        let envelope = envelope!(data).unwrap();
        assert_eq!(envelope.schema_version(), 1);
    }

    #[test]
    fn test_serialize_macro() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct SerializeData {
            value: u64,
        }
        impl VersionedSerializable for SerializeData {
            const CURRENT_VERSION: u8 = 1;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(self.value.to_le_bytes().to_vec())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                decode_u64_payload(version, bytes).map(|value| Self { value })
            }
        }

        let data = SerializeData { value: 42 };
        let bytes = serialize!(data).unwrap();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn test_deserialize_macro() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct DeserializeData {
            value: u64,
        }
        impl VersionedSerializable for DeserializeData {
            const CURRENT_VERSION: u8 = 1;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(self.value.to_le_bytes().to_vec())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                decode_u64_payload(version, bytes).map(|value| Self { value })
            }
        }

        let data = DeserializeData { value: 42 };
        let bytes = serialize!(data).unwrap();
        let restored: DeserializeData = deserialize!(&bytes).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_deserialize_macro_with_type() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct DeserializeTypedData {
            value: u64,
        }
        impl VersionedSerializable for DeserializeTypedData {
            const CURRENT_VERSION: u8 = 1;

            fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
                Ok(self.value.to_le_bytes().to_vec())
            }

            fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
                decode_u64_payload(version, bytes).map(|value| Self { value })
            }
        }

        let data = DeserializeTypedData { value: 42 };
        let bytes = serialize!(data).unwrap();
        let restored = deserialize!(&bytes, DeserializeTypedData).unwrap();
        assert_eq!(data, restored);
    }

    #[test]
    fn test_define_schema_version() {
        #[derive(Clone, Debug, PartialEq, Eq)]
        struct SchemaTypeA {
            value: u32,
        }

        #[derive(Clone, Debug, PartialEq, Eq)]
        struct SchemaTypeB {
            value: u64,
        }

        define_schema_version!(TEST_SCHEMA_VERSION = 3, SchemaTypeA, SchemaTypeB);

        assert_eq!(TEST_SCHEMA_VERSION, 3);
        assert_eq!(SchemaTypeA::CURRENT_VERSION, 3);
        assert_eq!(SchemaTypeB::CURRENT_VERSION, 3);
    }

    fn decode_u64_payload(version: u8, bytes: &[u8]) -> Result<u64, SerializationError> {
        if version != 1 {
            return Err(SerializationError::UpgradePathNotAvailable { from: version, to: 1 });
        }
        if bytes.len() != 8 {
            return Err(SerializationError::DeserializationFailed("expected u64 payload".to_string()));
        }
        Ok(u64::from_le_bytes(bytes.try_into().expect("slice length checked")))
    }
}
