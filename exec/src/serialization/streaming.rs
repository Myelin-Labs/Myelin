// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
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
        let bytes = envelope.to_bytes();

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
        let mut len_bytes = [0u8; 4];
        self.reader.read_exact(&mut len_bytes).map_err(|e| SerializationError::IoError(e.to_string()))?;
        let total_len = u32::from_le_bytes(len_bytes) as usize;
        if total_len < 4 {
            return Err(SerializationError::DeserializationFailed("invalid Molecule envelope length".to_string()));
        }

        let mut bytes = Vec::with_capacity(total_len);
        bytes.extend_from_slice(&len_bytes);
        let mut rest = vec![0u8; total_len - 4];
        self.reader.read_exact(&mut rest).map_err(|e| SerializationError::IoError(e.to_string()))?;
        bytes.extend_from_slice(&rest);
        self.bytes_read += total_len;

        let envelope = crate::serialization::VersionedEnvelope::<T>::from_bytes(&bytes)?;
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

    #[test]
    fn streaming_sequences_roundtrip_and_reject_truncation() {
        for count in [0, 1, 5, 1000] {
            let outputs: Vec<CellOutput> = (0..count)
                .map(|i| CellOutput {
                    lock: Script::new([i as u8; 32], 0, vec![0xBB; 100]),
                    type_: (i % 2 == 0).then(|| Script::new([0xCC; 32], 1, vec![0xDD; 50])),
                    capacity: i as u64,
                })
                .collect();
            let mut bytes = Vec::new();
            assert_eq!(serialize_streaming(&mut bytes, &outputs).unwrap(), bytes.len());
            let mut reader = bytes.as_slice();
            assert_eq!(deserialize_streaming::<_, CellOutput>(&mut reader).unwrap(), outputs);
            assert!(reader.is_empty());
            for cut in [0, 1, 3, bytes.len() - 1] {
                assert!(deserialize_streaming::<_, CellOutput>(&mut &bytes[..cut]).is_err());
            }
        }
        // One item with an impossible envelope length.
        assert!(deserialize_streaming::<_, CellOutput>(&mut &[1, 0, 0, 0, 3, 0, 0, 0][..]).is_err());
    }

    #[test]
    fn streaming_raw_frames_preserve_bytes_and_counters() {
        let mut serializer = StreamingSerializer::new(Vec::new());
        serializer.write_length(42).unwrap();
        serializer.write_raw(&[0x01, 0x02, 0x03]).unwrap();
        assert_eq!(serializer.bytes_written(), 7);
        let bytes = serializer.into_inner();
        assert_eq!(bytes, [42, 0, 0, 0, 1, 2, 3]);

        let mut deserializer = StreamingDeserializer::new(bytes.as_slice());
        assert_eq!(deserializer.read_length().unwrap(), 42);
        let mut raw = [0; 3];
        deserializer.read_raw(&mut raw).unwrap();
        assert_eq!(raw, [1, 2, 3]);
        assert_eq!(deserializer.bytes_read(), bytes.len());
        assert!(deserializer.into_inner().is_empty());
    }
}
