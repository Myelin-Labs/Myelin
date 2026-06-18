//!
//! # Myelin Utilities
//!
//! General purpose utilities and various type extensions used across the Rusty Myelin codebase.
//!

pub mod any;
pub mod arc;
pub mod binary_heap;
pub mod channel;
pub mod expiring_cache;
pub mod hashmap;
pub mod hex;
pub mod iter;
pub mod mem_size;
pub mod networking;
pub mod refs;

pub mod as_slice;

/// # Examples
/// ## Implement serde::Serialize/serde::Deserialize for the Vec field of the struct
/// ```
/// #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// struct MyStructVec {
///     #[serde(with = "myelin_utils::serde_bytes")]
///     v: Vec<u8>,
/// }
/// let v = vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19];
/// let test_struct = MyStructVec { v: v.clone() };
///
/// let expected_str = r#"{"v":"000102030405060708090a0b0c0d0e0f10111213"}"#;
/// // Serialize using serde_json
/// let json = serde_json::to_string(&test_struct).unwrap();
/// assert_eq!(expected_str, json);
/// // Deserialize using serde_json
/// let from_json: MyStructVec = serde_json::from_str(&json).unwrap();
/// assert_eq!(test_struct, from_json);
/// ```
/// ## Implement serde::Serialize/serde::Deserialize for the SmallVec field of the struct
/// ```
/// use smallvec::{smallvec, SmallVec};
/// #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// struct MyStructSmallVec {  
///     #[serde(with = "myelin_utils::serde_bytes")]
///     v: SmallVec<[u8; 19]>,
/// }
/// let v = smallvec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19];
/// let test_struct = MyStructSmallVec { v: v.clone() };
///
/// let expected_str = r#"{"v":"000102030405060708090a0b0c0d0e0f10111213"}"#;
/// // Serialize using serde_json
/// let json = serde_json::to_string(&test_struct).unwrap();
/// assert_eq!(expected_str, json);
/// // Deserialize using serde_json
/// let from_json: MyStructSmallVec = serde_json::from_str(&json).unwrap();
/// assert_eq!(test_struct, from_json);
/// ```
pub mod serde_bytes;
pub mod serde_bytes_optional;

/// # Examples
///
/// ```
/// #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// struct TestStruct {
///     #[serde(with = "myelin_utils::serde_bytes_fixed")]
///     v: [u8; 20],
/// }
/// let test_struct = TestStruct { v: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19] };
///
/// let expected_str = r#"{"v":"000102030405060708090a0b0c0d0e0f10111213"}"#;
/// // Serialize using serde_json
/// let json = serde_json::to_string(&test_struct).unwrap();
/// assert_eq!(expected_str, json);
/// // Deserialize using serde_json
/// let from_json: TestStruct = serde_json::from_str(&json).unwrap();
/// assert_eq!(test_struct, from_json);
/// ```
pub mod serde_bytes_fixed;
/// # Examples
/// ## Implement serde::Serialize/serde::Deserialize using declarative macro
/// ```
/// #[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// struct MyStruct([u8; 20]);
///
/// impl AsRef<[u8; 20]> for MyStruct {
///     fn as_ref(&self) -> &[u8; 20] {
///         &self.0
///     }
/// }
/// impl myelin_utils::hex::FromHex for MyStruct {
///     type Error = faster_hex::Error;
///     fn from_hex(hex_str: &str) -> Result<Self, Self::Error> {
///         let mut bytes = [0u8; 20];
///         faster_hex::hex_decode(hex_str.as_bytes(), &mut bytes)?;
///         Ok(MyStruct(bytes))
///     }
/// }
/// impl From<[u8; 20]> for MyStruct {
///     fn from(value: [u8; 20]) -> Self {
///         MyStruct(value)
///     }
/// }
/// myelin_utils::serde_impl_ser_fixed_bytes_ref!(MyStruct, 20);
/// myelin_utils::serde_impl_deser_fixed_bytes_ref!(MyStruct, 20);
///
/// let test_struct = MyStruct([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]);
/// let expected_str = r#""000102030405060708090a0b0c0d0e0f10111213""#;
/// // Serialize using serde_json
/// let json = serde_json::to_string(&test_struct).unwrap();
/// assert_eq!(expected_str, json);
/// // Deserialize using serde_json
/// let from_json: MyStruct = serde_json::from_str(&json).unwrap();
/// assert_eq!(test_struct, from_json);
/// ```
///
/// ## Implement serde::Serialize/serde::Deserialize for the field of the struct
/// ```
/// #[derive(Debug, Copy, Clone, PartialEq, Eq)]
/// struct MyStruct([u8; 20]);
///
/// impl AsRef<[u8; 20]> for MyStruct {
///     fn as_ref(&self) -> &[u8; 20] {
///         &self.0
///     }
/// }
/// impl myelin_utils::hex::FromHex for MyStruct {
///     type Error = faster_hex::Error;
///     fn from_hex(hex_str: &str) -> Result<Self, Self::Error> {
///         let mut bytes = [0u8; 20];
///         faster_hex::hex_decode(hex_str.as_bytes(), &mut bytes)?;
///         Ok(MyStruct(bytes))
///     }
/// }
/// impl From<[u8; 20]> for MyStruct {
///     fn from(value: [u8; 20]) -> Self {
///         MyStruct(value)
///     }
/// }
///
/// #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
/// struct TestStruct {
///     #[serde(with = "myelin_utils::serde_bytes_fixed_ref")]
///     v: MyStruct,
/// }
///
/// let test_struct = TestStruct { v: MyStruct([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19]) };
///
/// let expected_str = r#"{"v":"000102030405060708090a0b0c0d0e0f10111213"}"#;
/// // Serialize using serde_json
/// let json = serde_json::to_string(&test_struct).unwrap();
/// assert_eq!(expected_str, json);
/// // Deserialize using serde_json
/// let from_json: TestStruct = serde_json::from_str(&json).unwrap();
/// assert_eq!(test_struct, from_json);
/// ```
pub mod serde_bytes_fixed_ref;
pub mod sim;
pub mod sync;
pub mod triggers;
pub mod vec;

pub mod git;

#[cfg(not(target_arch = "wasm32"))]
pub mod fd_budget;
#[cfg(not(target_arch = "wasm32"))]
pub mod sysinfo;
