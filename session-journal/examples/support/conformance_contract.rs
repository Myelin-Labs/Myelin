//! JSON test-vector contract, not a consensus or journal transport.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    pub kind: String,
    pub profile: [u8; 32],
    pub snapshot_limit: u64,
    pub initial: Vec<u8>,
    pub commands: Vec<Vec<u8>>,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub success: bool,
    pub payload: Vec<u8>,
}
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub width: usize,
    pub restart: bool,
    pub snapshot: Vec<u8>,
    pub root: [u8; 32],
    pub outcomes: Vec<Outcome>,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vector {
    pub scenario: Scenario,
    pub expected: Vec<Observation>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format: String,
    pub native_program_digest: [u8; 32],
    pub vectors: Vec<Vector>,
}
