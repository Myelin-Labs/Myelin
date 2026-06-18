// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Streaming Serialization
//
//! # 流式序列化
//!
//! 本模块提供流式序列化支持，用于处理大型数据而不需要一次性加载到内存。
//!
//! ## 使用场景
//!
//! - 大型 CellTx 的序列化/反序列化
//! - 网络流中的数据处理
//! - 磁盘文件的流式读写

use crate::serialization::{SerializationError, VersionedSerializable};
use std::io::{Read, Write};

/// 流式序列化器
///
/// 用于将数据流式写入输出。
pub struct StreamingSerializer<W: Write> {
    writer: W,
    bytes_written: usize,
}

impl<W: Write> StreamingSerializer<W> {
    /// 创建新的流式序列化器
    pub fn new(writer: W) -> Self {
        Self { writer, bytes_written: 0 }
    }

    /// 序列化单个值
    pub fn serialize<T: VersionedSerializable>(&mut self, value: &T) -> Result<(), SerializationError> {
        let envelope = crate::serialization::VersionedEnvelope::new(value)?;
        let bytes = borsh::to_vec(&envelope).map_err(|e| SerializationError::IoError(e.to_string()))?;

        self.writer.write_all(&bytes).map_err(|e| SerializationError::IoError(e.to_string()))?;

        self.bytes_written += bytes.len();
        Ok(())
    }

    /// 写入原始字节
    pub fn write_raw(&mut self, bytes: &[u8]) -> Result<(), SerializationError> {
        self.writer.write_all(bytes).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_written += bytes.len();
        Ok(())
    }

    /// 写入长度前缀
    pub fn write_length(&mut self, len: u32) -> Result<(), SerializationError> {
        self.writer.write_all(&len.to_le_bytes()).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_written += 4;
        Ok(())
    }

    /// 刷新写入器
    pub fn flush(&mut self) -> Result<(), SerializationError> {
        self.writer.flush().map_err(|e| SerializationError::IoError(e.to_string()))
    }

    /// 获取已写入的字节数
    pub fn bytes_written(&self) -> usize {
        self.bytes_written
    }

    /// 获取底层写入器的引用
    pub fn inner(&self) -> &W {
        &self.writer
    }

    /// 获取底层写入器的可变引用
    pub fn inner_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// 消费序列化器并返回底层写入器
    pub fn into_inner(self) -> W {
        self.writer
    }
}

/// 流式反序列化器
///
/// 用于从输入流式读取数据。
pub struct StreamingDeserializer<R: Read> {
    reader: R,
    bytes_read: usize,
}

impl<R: Read> StreamingDeserializer<R> {
    /// 创建新的流式反序列化器
    pub fn new(reader: R) -> Self {
        Self { reader, bytes_read: 0 }
    }

    /// 反序列化单个值
    pub fn deserialize<T: VersionedSerializable>(&mut self) -> Result<T, SerializationError> {
        // Read format version
        let mut format_version = [0u8; 1];
        self.reader.read_exact(&mut format_version).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_read += 1;

        // Read schema version
        let mut schema_version = [0u8; 1];
        self.reader.read_exact(&mut schema_version).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_read += 1;

        // Read payload length (u32)
        let mut len_bytes = [0u8; 4];
        self.reader.read_exact(&mut len_bytes).map_err(|e| SerializationError::IoError(e.to_string()))?;
        let payload_len = u32::from_le_bytes(len_bytes) as usize;
        self.bytes_read += 4;

        // Read payload
        let mut payload = vec![0u8; payload_len];
        self.reader.read_exact(&mut payload).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_read += payload_len;

        // Create envelope and parse
        let envelope = crate::serialization::VersionedEnvelope::<T> {
            format_version: format_version[0],
            schema_version: schema_version[0],
            payload,
            _phantom: std::marker::PhantomData,
        };

        envelope.parse()
    }

    /// 读取原始字节
    pub fn read_raw(&mut self, buf: &mut [u8]) -> Result<(), SerializationError> {
        self.reader.read_exact(buf).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_read += buf.len();
        Ok(())
    }

    /// 读取长度前缀
    pub fn read_length(&mut self) -> Result<u32, SerializationError> {
        let mut len_bytes = [0u8; 4];
        self.reader.read_exact(&mut len_bytes).map_err(|e| SerializationError::IoError(e.to_string()))?;
        self.bytes_read += 4;
        Ok(u32::from_le_bytes(len_bytes))
    }

    /// 获取已读取的字节数
    pub fn bytes_read(&self) -> usize {
        self.bytes_read
    }

    /// 获取底层读取器的引用
    pub fn inner(&self) -> &R {
        &self.reader
    }

    /// 获取底层读取器的可变引用
    pub fn inner_mut(&mut self) -> &mut R {
        &mut self.reader
    }

    /// 消费反序列化器并返回底层读取器
    pub fn into_inner(self) -> R {
        self.reader
    }
}

/// 将多个值流式序列化到写入器
pub fn serialize_streaming<W: Write, T: VersionedSerializable>(writer: &mut W, values: &[T]) -> Result<usize, SerializationError> {
    let mut serializer = StreamingSerializer::new(writer);

    // Write count
    serializer.write_length(values.len() as u32)?;

    // Write each value
    for value in values {
        serializer.serialize(value)?;
    }

    serializer.flush()?;
    Ok(serializer.bytes_written())
}

/// 从读取器流式反序列化多个值
pub fn deserialize_streaming<R: Read, T: VersionedSerializable>(reader: &mut R) -> Result<Vec<T>, SerializationError> {
    let mut deserializer = StreamingDeserializer::new(reader);

    // Read count
    let count = deserializer.read_length()? as usize;
    let mut result = Vec::with_capacity(count);

    // Read each value
    for _ in 0..count {
        let value = deserializer.deserialize()?;
        result.push(value);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellOutput, Script};

    fn create_test_output() -> CellOutput {
        CellOutput { lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]), type_: None, capacity: 1000 }
    }

    #[test]
    fn test_streaming_serializer_basic() {
        let mut buf = Vec::new();
        let mut serializer = StreamingSerializer::new(&mut buf);

        let output = create_test_output();
        serializer.serialize(&output).unwrap();

        assert!(serializer.bytes_written() > 0);
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_streaming_deserializer_basic() {
        let output = create_test_output();
        let envelope = crate::serialization::VersionedEnvelope::new(&output).unwrap();
        let bytes = borsh::to_vec(&envelope).unwrap();

        let mut deserializer = StreamingDeserializer::new(&bytes[..]);
        let restored: CellOutput = deserializer.deserialize().unwrap();

        assert_eq!(output, restored);
    }

    #[test]
    fn test_streaming_roundtrip() {
        let outputs: Vec<CellOutput> = (0..5)
            .map(|i| CellOutput { lock: Script::new([i as u8; 32], 0, vec![i as u8; 20]), type_: None, capacity: i as u64 })
            .collect();

        // Serialize
        let mut buf = Vec::new();
        {
            let mut serializer = StreamingSerializer::new(&mut buf);
            serializer.write_length(outputs.len() as u32).unwrap();
            for output in &outputs {
                serializer.serialize(output).unwrap();
            }
        }

        // Deserialize
        let mut deserializer = StreamingDeserializer::new(&buf[..]);
        let count = deserializer.read_length().unwrap() as usize;
        let mut restored = Vec::with_capacity(count);
        for _ in 0..count {
            restored.push(deserializer.deserialize::<CellOutput>().unwrap());
        }

        assert_eq!(outputs, restored);
    }

    #[test]
    fn test_serialize_streaming_helper() {
        let outputs: Vec<CellOutput> = (0..3).map(|_| create_test_output()).collect();

        let mut buf = Vec::new();
        let bytes_written = serialize_streaming(&mut buf, &outputs).unwrap();

        assert!(bytes_written > 0);
        assert_eq!(buf.len(), bytes_written);
    }

    #[test]
    fn test_deserialize_streaming_helper() {
        let outputs: Vec<CellOutput> = (0..3).map(|_| create_test_output()).collect();

        let mut buf = Vec::new();
        serialize_streaming(&mut buf, &outputs).unwrap();

        let restored = deserialize_streaming::<_, CellOutput>(&mut &buf[..]).unwrap();
        assert_eq!(outputs.len(), restored.len());
    }

    #[test]
    fn test_streaming_write_read_raw() {
        let mut buf = Vec::new();

        // Write
        {
            let mut serializer = StreamingSerializer::new(&mut buf);
            serializer.write_length(42).unwrap();
            serializer.write_raw(&[0x01, 0x02, 0x03]).unwrap();
        }

        // Read
        {
            let mut deserializer = StreamingDeserializer::new(&buf[..]);
            let len = deserializer.read_length().unwrap();
            assert_eq!(len, 42);

            let mut raw = [0u8; 3];
            deserializer.read_raw(&mut raw).unwrap();
            assert_eq!(raw, [0x01, 0x02, 0x03]);
        }
    }

    #[test]
    fn test_streaming_bytes_tracking() {
        let mut buf = Vec::new();
        let mut serializer = StreamingSerializer::new(&mut buf);

        let initial = serializer.bytes_written();
        serializer.write_length(100).unwrap();
        let after_length = serializer.bytes_written();
        serializer.write_raw(&[0x00; 10]).unwrap();
        let after_raw = serializer.bytes_written();

        assert_eq!(after_length - initial, 4);
        assert_eq!(after_raw - after_length, 10);
    }

    #[test]
    fn test_streaming_into_inner() {
        let buf = Vec::new();
        let serializer = StreamingSerializer::new(buf);
        let buf = serializer.into_inner();
        assert!(buf.is_empty());
    }

    #[test]
    fn test_streaming_empty_sequence() {
        let outputs: Vec<CellOutput> = vec![];

        let mut buf = Vec::new();
        serialize_streaming(&mut buf, &outputs).unwrap();

        let restored = deserialize_streaming::<_, CellOutput>(&mut &buf[..]).unwrap();
        assert!(restored.is_empty());
    }

    #[test]
    fn test_streaming_large_data() {
        let outputs: Vec<CellOutput> = (0..1000)
            .map(|i| CellOutput {
                lock: Script::new([i as u8; 32], 0, vec![0xBB; 100]),
                type_: Some(Script::new([0xCC; 32], 1, vec![0xDD; 50])),
                capacity: i as u64,
            })
            .collect();

        let mut buf = Vec::new();
        let bytes_written = serialize_streaming(&mut buf, &outputs).unwrap();

        let restored = deserialize_streaming::<_, CellOutput>(&mut &buf[..]).unwrap();
        assert_eq!(outputs.len(), restored.len());

        // Verify a few items
        for i in [0, 100, 500, 999] {
            assert_eq!(outputs[i], restored[i]);
        }

        println!("Serialized {} items, {} bytes", outputs.len(), bytes_written);
    }
}
