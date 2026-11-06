//! Reference encoder for published protocol proposals, not a bundle importer
//! or an application authorization/finality verifier.
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Corpus {
    pub format: String,
    pub vectors: Vec<Vector>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Vector {
    pub name: String,
    pub domain: String,
    pub fields: Vec<Field>,
    pub preimage_hex: String,
    pub blake3: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub encoding: String,
    pub value: String,
}

pub fn corpus() -> Result<Corpus, String> {
    let corpus: Corpus =
        serde_json::from_str(include_str!("../../../fixtures/protocol/cross-session-v1.json")).map_err(|error| error.to_string())?;
    if corpus.format != "myelin-cross-session-reference-v1" {
        return Err("unknown vector format".to_owned());
    }
    Ok(corpus)
}

pub fn preimage(vector: &Vector) -> Result<Vec<u8>, String> {
    let mut bytes = vector.domain.as_bytes().to_vec();
    for field in &vector.fields {
        match field.encoding.as_str() {
            "u8" => bytes.push(field.value.parse::<u8>().map_err(|error| error.to_string())?),
            "u16" => bytes.extend(field.value.parse::<u16>().map_err(|error| error.to_string())?.to_le_bytes()),
            "u64" => bytes.extend(field.value.parse::<u64>().map_err(|error| error.to_string())?.to_le_bytes()),
            "hash32" | "bytes" => {
                let value = unhex(&field.value)?;
                if field.encoding == "hash32" {
                    if value.len() != 32 {
                        return Err(format!("{} is not a 32-byte hash", field.name));
                    }
                } else {
                    bytes.extend((value.len() as u64).to_le_bytes());
                }
                bytes.extend(value);
            }
            other => return Err(format!("unknown field encoding {other}")),
        }
    }
    Ok(bytes)
}

pub fn verify(vector: &Vector) -> Result<(), String> {
    let bytes = preimage(vector)?;
    if hex(&bytes) != vector.preimage_hex {
        return Err(format!("{}: canonical preimage mismatch", vector.name));
    }
    if blake3::hash(&bytes).to_hex().as_str() != vector.blake3 {
        return Err(format!("{}: BLAKE3 commitment mismatch", vector.name));
    }
    Ok(())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Result<Vec<u8>, String> {
    if !value.is_ascii() || !value.len().is_multiple_of(2) {
        return Err("expected even-length ASCII hex".to_owned());
    }
    (0..value.len()).step_by(2).map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|error| error.to_string())).collect()
}
