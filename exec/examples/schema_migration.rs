// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Schema Migration Example

use myelin_exec::{SerializationError, VersionedEnvelope, VersionedSerializable};

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserDataV1 {
    name: String,
    age: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UserDataV2 {
    name: String,
    birth_year: u32,
    email: Option<String>,
}

impl UserDataV2 {
    fn new(name: &str, birth_year: u32, email: Option<&str>) -> Self {
        Self { name: name.to_string(), birth_year, email: email.map(|s| s.to_string()) }
    }
}

impl VersionedSerializable for UserDataV2 {
    const CURRENT_VERSION: u8 = 2;

    fn to_versioned_payload(&self) -> Result<Vec<u8>, SerializationError> {
        let mut out = Vec::new();
        push_string(&mut out, &self.name);
        out.extend_from_slice(&self.birth_year.to_le_bytes());
        match &self.email {
            Some(email) => {
                out.push(1);
                push_string(&mut out, email);
            }
            None => out.push(0),
        }
        Ok(out)
    }

    fn upgrade_from(version: u8, bytes: &[u8]) -> Result<Self, SerializationError> {
        match version {
            1 => {
                let v1 = UserDataV1::from_payload(bytes)?;
                let current_year: u32 = 2026;
                Ok(Self { name: v1.name, birth_year: current_year.saturating_sub(v1.age), email: None })
            }
            2 => Self::from_payload(bytes),
            _ => Err(SerializationError::UpgradePathNotAvailable { from: version, to: Self::CURRENT_VERSION }),
        }
    }
}

impl UserDataV1 {
    fn to_payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        push_string(&mut out, &self.name);
        out.extend_from_slice(&self.age.to_le_bytes());
        out
    }

    fn from_payload(bytes: &[u8]) -> Result<Self, SerializationError> {
        let mut offset = 0;
        let name = read_string(bytes, &mut offset)?;
        let age = read_u32(bytes, &mut offset)?;
        ensure_consumed(bytes, offset)?;
        Ok(Self { name, age })
    }
}

impl UserDataV2 {
    fn from_payload(bytes: &[u8]) -> Result<Self, SerializationError> {
        let mut offset = 0;
        let name = read_string(bytes, &mut offset)?;
        let birth_year = read_u32(bytes, &mut offset)?;
        let has_email = read_u8(bytes, &mut offset)?;
        let email = match has_email {
            0 => None,
            1 => Some(read_string(bytes, &mut offset)?),
            other => return Err(SerializationError::DeserializationFailed(format!("invalid optional email flag {other}"))),
        };
        ensure_consumed(bytes, offset)?;
        Ok(Self { name, birth_year, email })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Schema Migration Example ===\n");

    let user_v2 = UserDataV2::new("Alice", 1990, Some("alice@example.com"));
    let envelope = VersionedEnvelope::new(&user_v2)?;
    let stored_bytes = envelope.to_bytes();

    println!("Stored v2 user with schema version {}", envelope.schema_version());

    let restored_envelope = VersionedEnvelope::<UserDataV2>::from_bytes(&stored_bytes)?;
    let restored_user = restored_envelope.parse()?;
    assert_eq!(user_v2, restored_user);
    println!("Read v2 data successfully");

    let user_v1 = UserDataV1 { name: "Bob".to_string(), age: 30 };
    let old_envelope =
        VersionedEnvelope::<UserDataV2>::from_parts(VersionedEnvelope::<UserDataV2>::FORMAT_VERSION_MOLECULE, 1, user_v1.to_payload());
    let old_stored_bytes = old_envelope.to_bytes();

    let migrated_envelope = VersionedEnvelope::<UserDataV2>::from_bytes(&old_stored_bytes)?;
    let migrated_user = migrated_envelope.parse()?;

    assert_eq!(migrated_user.name, "Bob");
    assert_eq!(migrated_user.birth_year, 1996);
    assert_eq!(migrated_user.email, None);
    println!("Migrated v1 data to v2 successfully");

    Ok(())
}

fn push_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn read_string(bytes: &[u8], offset: &mut usize) -> Result<String, SerializationError> {
    let len = read_u32(bytes, offset)? as usize;
    let end = offset.checked_add(len).ok_or_else(|| SerializationError::DeserializationFailed("offset overflow".to_string()))?;
    if end > bytes.len() {
        return Err(SerializationError::DeserializationFailed("string extends past end of payload".to_string()));
    }
    let value =
        std::str::from_utf8(&bytes[*offset..end]).map_err(|e| SerializationError::DeserializationFailed(e.to_string()))?.to_string();
    *offset = end;
    Ok(value)
}

fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, SerializationError> {
    let end = offset.checked_add(4).ok_or_else(|| SerializationError::DeserializationFailed("offset overflow".to_string()))?;
    if end > bytes.len() {
        return Err(SerializationError::DeserializationFailed("u32 extends past end of payload".to_string()));
    }
    let value = u32::from_le_bytes(bytes[*offset..end].try_into().expect("slice length checked"));
    *offset = end;
    Ok(value)
}

fn read_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, SerializationError> {
    if *offset >= bytes.len() {
        return Err(SerializationError::DeserializationFailed("u8 extends past end of payload".to_string()));
    }
    let value = bytes[*offset];
    *offset += 1;
    Ok(value)
}

fn ensure_consumed(bytes: &[u8], offset: usize) -> Result<(), SerializationError> {
    if offset == bytes.len() {
        Ok(())
    } else {
        Err(SerializationError::DeserializationFailed("trailing bytes in payload".to_string()))
    }
}
