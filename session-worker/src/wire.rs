use myelin_session::{bounded_codec, ExecutionOutcome, FrameInput, Hash32, ProtocolEffectsContext, SessionHandoff};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{Read, Write};

pub const MAX_PACKET_BYTES: usize = bounded_codec::MAX_RECORD_BYTES;
const HEADER_BYTES: usize = 44;
const MAGIC: &[u8; 4] = b"MWK2";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub profile: Hash32,
    pub pre_root: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub snapshot: Vec<u8>,
    pub operation: Operation,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    Restore,
    Frame {
        height: u64,
        input: FrameInput,
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        handoffs: Vec<SessionHandoff>,
        #[serde(deserialize_with = "bounded_codec::transactions")]
        transactions: Vec<Vec<u8>>,
    },
    Block {
        context: Box<ProtocolEffectsContext>,
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        actions: Vec<myelin_session::ResolvedHandoffAction>,
        input: FrameInput,
        #[serde(deserialize_with = "bounded_codec::bounded_vec")]
        handoffs: Vec<SessionHandoff>,
        #[serde(deserialize_with = "bounded_codec::transactions")]
        transactions: Vec<Vec<u8>>,
    },
}

#[derive(Serialize)]
pub(crate) struct RequestRef<'a> {
    pub profile: Hash32,
    pub pre_root: Hash32,
    pub snapshot: &'a [u8],
    pub operation: OperationRef<'a>,
}

#[derive(Serialize)]
pub(crate) enum OperationRef<'a> {
    Restore,
    Frame {
        height: u64,
        input: &'a FrameInput,
        handoffs: &'a [SessionHandoff],
        transactions: &'a [Vec<u8>],
    },
    Block {
        context: &'a ProtocolEffectsContext,
        actions: &'a [myelin_session::ResolvedHandoffAction],
        input: &'a FrameInput,
        handoffs: &'a [SessionHandoff],
        transactions: &'a [Vec<u8>],
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub request_hash: Hash32,
    pub result: Result<StateResult, ApplicationError>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateResult {
    pub pre_root: Hash32,
    pub state_root: Hash32,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub snapshot: Vec<u8>,
    pub outcome: Option<ExecutionOutcome>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationError {
    #[serde(deserialize_with = "bounded_codec::bounded_string")]
    pub message: String,
}

pub fn encode<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>, String> {
    let cap = limit.min(MAX_PACKET_BYTES).checked_sub(HEADER_BYTES).ok_or("worker packet limit is too small")?;
    bounded_codec::size_with_limit(value, "worker_packet_bytes", cap as u64).map_err(|e| e.to_string())?;
    let body = postcard::to_allocvec(value).map_err(|e| e.to_string())?;
    let mut packet = Vec::with_capacity(HEADER_BYTES + body.len());
    packet.extend_from_slice(MAGIC);
    packet.extend_from_slice(&(body.len() as u64).to_le_bytes());
    packet.extend_from_slice(blake3::hash(&body).as_bytes());
    packet.extend_from_slice(&body);
    Ok(packet)
}

pub fn decode<T: DeserializeOwned>(packet: &[u8], limit: usize) -> Result<T, String> {
    if packet.len() > limit.min(MAX_PACKET_BYTES) || packet.len() < HEADER_BYTES || !packet.starts_with(MAGIC) {
        return Err("invalid worker packet envelope or size".into());
    }
    let length = u64::from_le_bytes(packet[4..12].try_into().expect("checked header"));
    let body = &packet[HEADER_BYTES..];
    if length != body.len() as u64 || blake3::hash(body).as_bytes() != &packet[12..44] {
        return Err("worker packet length or checksum mismatch".into());
    }
    let (value, rest) = postcard::take_from_bytes(body).map_err(|e| e.to_string())?;
    if !rest.is_empty() {
        return Err("trailing worker packet bytes".into());
    }
    Ok(value)
}

/// Child entry point. The caller supplies the locally known profile identity
/// and restores its own application executor. No native plugin is loaded into
/// the parent verifier. Stdin/stdout are reserved for one bounded binary packet.
pub fn serve_stdio<E, F>(expected_profile: Hash32, restore: F) -> Result<(), String>
where
    E: myelin_session::TransitionExecutor,
    F: FnOnce(&[u8]) -> Result<E, String>,
{
    let mut packet = Vec::new();
    std::io::stdin().lock().take(MAX_PACKET_BYTES as u64 + 1).read_to_end(&mut packet).map_err(|e| e.to_string())?;
    let request: Request = decode(&packet, MAX_PACKET_BYTES)?;
    let request_hash = *blake3::hash(&packet).as_bytes();
    let execute = || {
        if request.profile != expected_profile {
            return Err("worker application profile mismatch".into());
        }
        let mut executor = restore(&request.snapshot)?;
        if executor.state_root() != request.pre_root {
            return Err("worker restored pre-state root mismatch".into());
        }
        let outcome = match request.operation {
            Operation::Restore => None,
            Operation::Frame { height, input, handoffs, transactions } => {
                Some(executor.execute_block(height, &input, &handoffs, &transactions)?)
            }
            Operation::Block { context, actions, input, handoffs, transactions } => {
                context.validate().map_err(|e| e.to_string())?;
                if context.pre_state_root != request.pre_root || context.input_root != input.root() {
                    return Err("worker lifecycle context does not match execution".into());
                }
                for action in &actions {
                    action.validate_for(context.source_session_id, context.height).map_err(|e| e.to_string())?;
                }
                Some(executor.execute_block_with_lifecycle(&context, &actions, &input, &handoffs, &transactions)?)
            }
        };
        Ok(StateResult { pre_root: request.pre_root, state_root: executor.state_root(), snapshot: executor.snapshot()?, outcome })
    };
    let result = execute().map_err(|message: String| ApplicationError { message: message.chars().take(1024).collect() });
    let reply = encode(&Reply { request_hash, result }, MAX_PACKET_BYTES)?;
    std::io::stdout().lock().write_all(&reply).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_requests_match_the_owned_worker_abi() {
        let input = FrameInput {
            commands: vec![],
            start_position: 0,
            end_position: 1,
            logical_time_start: 0,
            logical_time_end: 1,
            payload: vec![7],
        };
        let context = ProtocolEffectsContext {
            source_genesis_commitment: [3; 32],
            source_session_id: [4; 32],
            policy: myelin_session::ProtocolEffectsPolicy::AuthorityMetadata { policy_hash: [99; 32] },
            height: 9,
            timestamp_ms: 10,
            pre_state_root: [2; 32],
            input_root: input.root(),
            issued_handoffs: vec![],
            successor_config: vec![],
        };
        for operation in [
            OperationRef::Block { context: &context, actions: &[], input: &input, handoffs: &[], transactions: &[] },
            OperationRef::Restore,
            OperationRef::Frame { height: 9, input: &input, handoffs: &[], transactions: &[] },
        ] {
            let packet = encode(&RequestRef { profile: [1; 32], pre_root: [2; 32], snapshot: &[3; 32], operation }, 1024).unwrap();
            let owned: Request = decode(&packet, 1024).unwrap();
            assert_eq!(encode(&owned, 1024).unwrap(), packet);
        }
    }

    #[test]
    fn packet_import_rejects_wrong_checksum_declared_size_trailing_bytes_and_caps() {
        let packet =
            encode(&RequestRef { profile: [1; 32], pre_root: [2; 32], snapshot: &[3; 32], operation: OperationRef::Restore }, 1024)
                .unwrap();
        assert!(decode::<Request>(&packet, packet.len() - 1).is_err());
        let mut changed = packet.clone();
        changed[3] = b'1';
        assert!(decode::<Request>(&changed, 1024).is_err());
        let mut changed = packet.clone();
        changed[12] ^= 1;
        assert!(decode::<Request>(&changed, 1024).is_err());
        let mut changed = packet.clone();
        changed[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode::<Request>(&changed, 1024).is_err());
        let mut changed = packet.clone();
        changed.push(0);
        let len = changed.len() - HEADER_BYTES;
        changed[4..12].copy_from_slice(&(len as u64).to_le_bytes());
        let hash = blake3::hash(&changed[HEADER_BYTES..]);
        changed[12..44].copy_from_slice(hash.as_bytes());
        assert!(decode::<Request>(&changed, 1024).err().unwrap().contains("trailing"));
    }
}
