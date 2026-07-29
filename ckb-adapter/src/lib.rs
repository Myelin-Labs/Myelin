// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Fail-closed evidence adapter to an authoritative CKB node.

use merkle_cbt::{merkle_tree::Merge, MerkleProof, CBMT};
use myelin_exec::{
    ckb_cell_data_hash, ckb_header_hash_molecule, ckb_raw_transaction_hash_molecule, parse_ckb_dep_group_data, CellDep, CellInput,
    CellOutput, CellTx, CkbHeader, CkbRawHeader, DepType, OutPoint, ProjectionStage, ResolvedCell, Script, SimpleDataProvider,
    TransactionScriptVerifier, VmSemantics,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
    thread,
    time::Duration,
};

const CONTEXT_SCHEMA: &str = "myelin-ckb-context-resolved-v1";
const CONSENSUS_SCHEMA: &str = "myelin-ckb-consensus-validated-v1";
const SCRIPTS_SCHEMA: &str = "myelin-ckb-scripts-verified-v1";
const NODE_SCHEMA: &str = "myelin-ckb-node-accepted-v1";
const COMMITTED_SCHEMA: &str = "myelin-ckb-committed-v1";
const FINALITY_SCHEMA: &str = "myelin-ckb-finalized-v1";
const PROJECTION_SCHEMA: &str = "myelin-ckb-evidence-projection-v1";

struct CkbMerkleMerge;

impl Merge for CkbMerkleMerge {
    type Item = [u8; 32];

    fn merge(left: &Self::Item, right: &Self::Item) -> Self::Item {
        let mut pair = [0u8; 64];
        pair[..32].copy_from_slice(left);
        pair[32..].copy_from_slice(right);
        ckb_cell_data_hash(&pair)
    }
}

/// Minimal synchronous JSON-RPC boundary used by the evidence engine.
pub trait CkbRpc {
    /// Call one CKB JSON-RPC method and return its `result` value.
    fn call(&self, method: &str, params: Value) -> Result<Value, CkbAdapterError>;
}

/// HTTP(S) CKB JSON-RPC client.
pub struct HttpCkbRpc {
    endpoint: String,
    client: reqwest::blocking::Client,
    request_id: AtomicU64,
}

impl HttpCkbRpc {
    /// Construct a client with bounded connect and request timeouts.
    pub fn new(endpoint: impl Into<String>) -> Result<Self, CkbAdapterError> {
        let endpoint = endpoint.into();
        let parsed = reqwest::Url::parse(&endpoint).map_err(|error| CkbAdapterError::InvalidRpcUrl(error.to_string()))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(CkbAdapterError::InvalidRpcUrl("CKB RPC URL must use http or https".to_owned()));
        }
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(CkbAdapterError::Http)?;
        Ok(Self { endpoint, client, request_id: AtomicU64::new(1) })
    }
}

impl CkbRpc for HttpCkbRpc {
    fn call(&self, method: &str, params: Value) -> Result<Value, CkbAdapterError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let response = self
            .client
            .post(&self.endpoint)
            .json(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .send()
            .map_err(CkbAdapterError::Http)?
            .error_for_status()
            .map_err(CkbAdapterError::Http)?
            .json::<Value>()
            .map_err(CkbAdapterError::Http)?;
        if let Some(error) = response.get("error").filter(|value| !value.is_null()) {
            return Err(CkbAdapterError::RpcRejected { method: method.to_owned(), error: error.clone() });
        }
        response.get("result").cloned().ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{method} response has no result")))
    }
}

/// Role of one resolved Cell in the exact transaction context.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResolvedCellRole {
    /// Consumed input.
    Input,
    /// Declared code dependency.
    CellDep,
    /// Member expanded from a CKB Molecule DepGroup.
    DepGroupMember,
}

/// One fully resolved Cell and its data under a stable CKB tip.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCellEvidence {
    /// Context role.
    pub role: ResolvedCellRole,
    /// Index in the declared input/CellDep vector, or member index for a DepGroup.
    pub index: usize,
    /// Exact Cell OutPoint.
    pub out_point: OutPoint,
    /// Declared DepGroup root when this is an expanded member.
    pub expanded_from: Option<OutPoint>,
    /// Full CellOutput returned by CKB.
    pub output: CellOutput,
    /// Exact Cell data.
    pub data: Vec<u8>,
    /// CKB-default Blake2b-256 data hash.
    pub data_hash: [u8; 32],
    /// Creating block when the referenced transaction is committed.
    pub creation_block_hash: Option<[u8; 32]>,
}

/// Canonical CKB header fields used by strict VM syscalls.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CkbHeaderEvidence {
    /// Header hash.
    pub hash: [u8; 32],
    /// Header version.
    pub version: u32,
    /// Compact target.
    pub compact_target: u32,
    /// Timestamp in milliseconds.
    pub timestamp: u64,
    /// Block number.
    pub number: u64,
    /// Packed epoch number-with-fraction.
    pub epoch: u64,
    /// Parent block hash.
    pub parent_hash: [u8; 32],
    /// Transactions merkle root.
    pub transactions_root: [u8; 32],
    /// Proposal IDs hash.
    pub proposals_hash: [u8; 32],
    /// Extra hash.
    pub extra_hash: [u8; 32],
    /// DAO field.
    pub dao: [u8; 32],
    /// PoW nonce.
    pub nonce: u128,
}

impl CkbHeaderEvidence {
    fn packed(&self) -> CkbHeader {
        CkbHeader {
            raw: CkbRawHeader {
                version: self.version,
                compact_target: self.compact_target,
                timestamp: self.timestamp,
                number: self.number,
                epoch: self.epoch,
                parent_hash: self.parent_hash,
                transactions_root: self.transactions_root,
                proposals_hash: self.proposals_hash,
                extra_hash: self.extra_hash,
                dao: self.dao,
            },
            nonce: self.nonce,
        }
    }

    fn verify_hash(&self) -> Result<(), CkbAdapterError> {
        let actual = ckb_header_hash_molecule(&self.packed()).map_err(|error| CkbAdapterError::Encoding(error.to_string()))?;
        if actual != self.hash {
            return Err(CkbAdapterError::EvidenceMismatch("CKB header hash does not match its canonical fields".to_owned()));
        }
        Ok(())
    }
}

/// Immutable, hash-bound CKB context resolution receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextResolvedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Chain name returned by the node.
    pub chain: String,
    /// CKB node software version.
    pub node_version: String,
    /// Genesis block hash.
    pub genesis_hash: [u8; 32],
    /// Canonical commitment to `get_consensus` output.
    pub consensus_rules_hash: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Canonical commitment to the JSON transaction submitted to RPC.
    pub transaction_json_hash: [u8; 32],
    /// Stable tip observed before and after resolution.
    pub tip: CkbHeaderEvidence,
    /// Every declared and expanded Cell.
    pub cells: Vec<ResolvedCellEvidence>,
    /// Every declared header dependency.
    pub headers: Vec<CkbHeaderEvidence>,
    /// Domain-separated commitment over all preceding fields.
    pub context_commitment: [u8; 32],
}

impl ContextResolvedReceipt {
    /// Recompute all structural, CKB hash, cardinality, DepGroup and receipt commitments.
    pub fn verify(&self, tx: &CellTx) -> Result<(), CkbAdapterError> {
        if self.schema != CONTEXT_SCHEMA {
            return Err(CkbAdapterError::EvidenceMismatch("unsupported context receipt schema".to_owned()));
        }
        let raw_tx_hash = ckb_raw_transaction_hash_molecule(tx).map_err(|error| CkbAdapterError::Encoding(error.to_string()))?;
        if raw_tx_hash != self.raw_tx_hash || canonical_json_hash(&ckb_json_transaction(tx)?) != self.transaction_json_hash {
            return Err(CkbAdapterError::EvidenceMismatch("context receipt is bound to a different transaction".to_owned()));
        }
        self.tip.verify_hash()?;
        if self.genesis_hash == [0; 32]
            || self.consensus_rules_hash == [0; 32]
            || self.chain.is_empty()
            || self.node_version.is_empty()
        {
            return Err(CkbAdapterError::EvidenceMismatch("context chain identity is incomplete".to_owned()));
        }

        let mut seen = BTreeSet::new();
        for cell in &self.cells {
            if ckb_cell_data_hash(&cell.data) != cell.data_hash {
                return Err(CkbAdapterError::EvidenceMismatch(format!("Cell data hash mismatch at {}", cell.out_point)));
            }
            if !seen.insert((cell.role as u8, cell.index, cell.out_point, cell.expanded_from)) {
                return Err(CkbAdapterError::EvidenceMismatch("duplicate resolved Cell evidence".to_owned()));
            }
        }
        for (index, input) in tx.inputs.iter().enumerate() {
            require_cell(self, ResolvedCellRole::Input, index, input.previous_output, None)?;
        }
        for (index, dep) in tx.cell_deps.iter().enumerate() {
            let root = require_cell(self, ResolvedCellRole::CellDep, index, dep.out_point, None)?;
            if dep.dep_type == DepType::DepGroup {
                let members = parse_ckb_dep_group_data(&root.data).map_err(CkbAdapterError::DepGroup)?;
                for (member_index, member) in members.into_iter().enumerate() {
                    require_cell(self, ResolvedCellRole::DepGroupMember, member_index, member, Some(dep.out_point))?;
                }
            }
        }

        let header_map = self.headers.iter().map(|header| (header.hash, header)).collect::<BTreeMap<_, _>>();
        if header_map.len() != self.headers.len() || header_map.len() != tx.header_deps.len() {
            return Err(CkbAdapterError::EvidenceMismatch("header dependency cardinality mismatch".to_owned()));
        }
        for hash in &tx.header_deps {
            header_map
                .get(hash)
                .ok_or_else(|| CkbAdapterError::EvidenceMismatch("missing header dependency".to_owned()))?
                .verify_hash()?;
        }
        if context_commitment(self) != self.context_commitment {
            return Err(CkbAdapterError::EvidenceMismatch("context receipt commitment mismatch".to_owned()));
        }
        Ok(())
    }
}

/// CKB tx-pool contextual validation receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConsensusValidatedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Exact context commitment validated by the node.
    pub context_commitment: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Stable tip used by `test_tx_pool_accept`.
    pub validation_tip_hash: [u8; 32],
    /// Fee computed by CKB.
    pub fee: u64,
    /// Cycles computed by CKB.
    pub cycles: u64,
    /// Canonical hash of the node result.
    pub node_result_hash: [u8; 32],
    /// Receipt commitment.
    pub receipt_commitment: [u8; 32],
}

/// Strict local CKB-VM plus authoritative node script-verification receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptsVerifiedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Consensus-validation receipt commitment.
    pub consensus_receipt_commitment: [u8; 32],
    /// Exact context commitment used locally and by the node.
    pub context_commitment: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Shared local transaction-level cycle limit.
    pub max_cycles: u64,
    /// Cycles consumed by Myelin's strict CKB-VM verifier.
    pub local_vm_cycles: u64,
    /// Cycles returned by CKB `test_tx_pool_accept`.
    pub node_vm_cycles: u64,
    /// Receipt commitment.
    pub receipt_commitment: [u8; 32],
}

/// Observed node acceptance receipt for the exact transaction.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeAcceptedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Scripts-verification receipt commitment.
    pub scripts_receipt_commitment: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Hash returned by `send_transaction`.
    pub submitted_tx_hash: [u8; 32],
    /// Status observed through `get_transaction`.
    pub observed_status: String,
    /// Optional committed block hash.
    pub block_hash: Option<[u8; 32]>,
    /// Optional committed block number.
    pub block_number: Option<u64>,
    /// Canonical hash of the observation response.
    pub observation_hash: [u8; 32],
    /// Receipt commitment.
    pub receipt_commitment: [u8; 32],
}

/// Canonical block inclusion receipt for the exact transaction.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Node-acceptance receipt commitment.
    pub node_receipt_commitment: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Canonical block header observed for the inclusion.
    pub block: CkbHeaderEvidence,
    /// Exact committed `get_transaction` response.
    pub committed_observation: Value,
    /// Inclusion proof returned by `get_transaction_proof`.
    pub transaction_proof: Value,
    /// Transaction hashes recovered by CKB `verify_transaction_proof`.
    pub proof_verified_tx_hashes: Vec<[u8; 32]>,
    /// Receipt commitment.
    pub receipt_commitment: [u8; 32],
}

/// Confirmation-depth and canonical-chain stability receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizedReceipt {
    /// Receipt schema.
    pub schema: String,
    /// Committed receipt commitment.
    pub committed_receipt_commitment: [u8; 32],
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Required confirmation depth.
    pub min_confirmations: u64,
    /// Observed confirmation depth (`tip.number - block.number`).
    pub confirmations: u64,
    /// Tip observed when the depth requirement passed.
    pub tip: CkbHeaderEvidence,
    /// Header re-resolved by block number to detect a reorganization.
    pub canonical_block: CkbHeaderEvidence,
    /// Exact committed transaction observation at finality time.
    pub committed_observation: Value,
    /// Receipt commitment.
    pub receipt_commitment: [u8; 32],
}

/// Evidence-staged projection whose stage is derived from concrete receipts.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CkbEvidenceProjection {
    /// Projection schema.
    pub schema: String,
    /// Highest supported stage.
    pub stage: ProjectionStage,
    /// Exact raw transaction hash.
    pub raw_tx_hash: [u8; 32],
    /// Context receipt.
    pub context: ContextResolvedReceipt,
    /// CKB contextual validation receipt.
    pub consensus: ConsensusValidatedReceipt,
    /// Script verification receipt.
    pub scripts: ScriptsVerifiedReceipt,
    /// Node observation, present only at `NodeAccepted`.
    pub node: Option<NodeAcceptedReceipt>,
    /// Canonical inclusion evidence, present at `Committed` and above.
    pub committed: Option<CommittedReceipt>,
    /// Confirmation and reorg-stability evidence, present only at `Finalized`.
    pub finalized: Option<FinalizedReceipt>,
}

/// CKB projection evidence engine over one concrete RPC transport.
pub struct CkbEvidenceEngine<R> {
    rpc: R,
}

impl<R: CkbRpc> CkbEvidenceEngine<R> {
    /// Construct an engine.
    pub fn new(rpc: R) -> Self {
        Self { rpc }
    }

    /// Resolve every input, CellDep/DepGroup member and header under one stable tip.
    pub fn resolve_context(&self, tx: &CellTx, max_attempts: usize) -> Result<ContextResolvedReceipt, CkbAdapterError> {
        if max_attempts == 0 {
            return Err(CkbAdapterError::InvalidRequest("max_attempts must be nonzero".to_owned()));
        }
        let transaction_json = ckb_json_transaction(tx)?;
        let transaction_json_hash = canonical_json_hash(&transaction_json);
        let raw_tx_hash = ckb_raw_transaction_hash_molecule(tx).map_err(|error| CkbAdapterError::Encoding(error.to_string()))?;

        for _ in 0..max_attempts {
            let tip = parse_header(&self.rpc.call("get_tip_header", json!([]))?)?;
            let local_node = self.rpc.call("local_node_info", json!([]))?;
            let chain_info = self.rpc.call("get_blockchain_info", json!([]))?;
            let consensus = self.rpc.call("get_consensus", json!([]))?;
            let genesis = parse_header(&self.rpc.call("get_header_by_number", json!(["0x0", null]))?)?;
            let node_version = value_string(&local_node, &["/version", "/node_version"])?;
            let chain = value_string(&chain_info, &["/chain"])?;

            let mut cells = Vec::new();
            for (index, input) in tx.inputs.iter().enumerate() {
                cells.push(self.resolve_cell(ResolvedCellRole::Input, index, input.previous_output, None)?);
            }
            for (index, dep) in tx.cell_deps.iter().enumerate() {
                let root = self.resolve_cell(ResolvedCellRole::CellDep, index, dep.out_point, None)?;
                if dep.dep_type == DepType::DepGroup {
                    for (member_index, member) in
                        parse_ckb_dep_group_data(&root.data).map_err(CkbAdapterError::DepGroup)?.into_iter().enumerate()
                    {
                        cells.push(self.resolve_cell(ResolvedCellRole::DepGroupMember, member_index, member, Some(dep.out_point))?);
                    }
                }
                cells.push(root);
            }
            let headers = tx
                .header_deps
                .iter()
                .map(|hash| self.rpc.call("get_header", json!([hash_hex(hash), null])).and_then(|value| parse_header(&value)))
                .collect::<Result<Vec<_>, _>>()?;
            let tip_after = parse_header(&self.rpc.call("get_tip_header", json!([]))?)?;
            if tip_after.hash != tip.hash {
                continue;
            }
            let mut receipt = ContextResolvedReceipt {
                schema: CONTEXT_SCHEMA.to_owned(),
                chain,
                node_version,
                genesis_hash: genesis.hash,
                consensus_rules_hash: canonical_json_hash(&consensus),
                raw_tx_hash,
                transaction_json_hash,
                tip,
                cells,
                headers,
                context_commitment: [0; 32],
            };
            receipt.context_commitment = context_commitment(&receipt);
            receipt.verify(tx)?;
            return Ok(receipt);
        }
        Err(CkbAdapterError::UnstableTip)
    }

    /// Run full CKB tx-pool validation and strict local CKB-VM verification over the same receipt.
    pub fn validate_and_verify(
        &self,
        tx: &CellTx,
        context: ContextResolvedReceipt,
        max_cycles: u64,
    ) -> Result<CkbEvidenceProjection, CkbAdapterError> {
        context.verify(tx)?;
        if max_cycles == 0 {
            return Err(CkbAdapterError::InvalidRequest("max_cycles must be nonzero".to_owned()));
        }
        self.require_tip(context.tip.hash)?;
        let node_result = self.rpc.call("test_tx_pool_accept", json!([ckb_json_transaction(tx)?, "passthrough"]))?;
        self.require_tip(context.tip.hash)?;
        let cycles = parse_quantity_value(
            node_result.get("cycles").ok_or_else(|| CkbAdapterError::InvalidResponse("missing tx-pool cycles".to_owned()))?,
            "cycles",
        )?;
        let fee = parse_quantity_value(
            node_result.get("fee").ok_or_else(|| CkbAdapterError::InvalidResponse("missing tx-pool fee".to_owned()))?,
            "fee",
        )?;
        if cycles > max_cycles {
            return Err(CkbAdapterError::EvidenceMismatch(format!("CKB node cycles {cycles} exceed configured limit {max_cycles}")));
        }
        let mut consensus = ConsensusValidatedReceipt {
            schema: CONSENSUS_SCHEMA.to_owned(),
            context_commitment: context.context_commitment,
            raw_tx_hash: context.raw_tx_hash,
            validation_tip_hash: context.tip.hash,
            fee,
            cycles,
            node_result_hash: canonical_json_hash(&node_result),
            receipt_commitment: [0; 32],
        };
        consensus.receipt_commitment = consensus_commitment(&consensus);

        let provider = context_provider(&context)?;
        let local_vm_cycles = TransactionScriptVerifier::new(Arc::new(tx.clone()), Arc::new(provider))
            .with_semantics(VmSemantics::CkbStrict)
            .with_max_cycles(max_cycles)
            .verify_with_cycles()
            .map_err(|error| CkbAdapterError::LocalVm(error.to_string()))?;
        let mut scripts = ScriptsVerifiedReceipt {
            schema: SCRIPTS_SCHEMA.to_owned(),
            consensus_receipt_commitment: consensus.receipt_commitment,
            context_commitment: context.context_commitment,
            raw_tx_hash: context.raw_tx_hash,
            max_cycles,
            local_vm_cycles,
            node_vm_cycles: cycles,
            receipt_commitment: [0; 32],
        };
        scripts.receipt_commitment = scripts_commitment(&scripts);
        Ok(CkbEvidenceProjection {
            schema: PROJECTION_SCHEMA.to_owned(),
            stage: ProjectionStage::ScriptsVerified,
            raw_tx_hash: context.raw_tx_hash,
            context,
            consensus,
            scripts,
            node: None,
            committed: None,
            finalized: None,
        })
    }

    /// Submit an already verified exact transaction and require an observable node status.
    pub fn submit_and_observe(
        &self,
        tx: &CellTx,
        mut projection: CkbEvidenceProjection,
        poll_attempts: usize,
        poll_interval: Duration,
    ) -> Result<CkbEvidenceProjection, CkbAdapterError> {
        verify_projection(tx, &projection)?;
        if projection.stage != ProjectionStage::ScriptsVerified
            || projection.node.is_some()
            || projection.committed.is_some()
            || projection.finalized.is_some()
            || poll_attempts == 0
        {
            return Err(CkbAdapterError::InvalidRequest("projection is not ready for node submission".to_owned()));
        }
        self.require_tip(projection.context.tip.hash)?;
        let result = self.rpc.call("send_transaction", json!([ckb_json_transaction(tx)?, "passthrough"]))?;
        let submitted_tx_hash = parse_hash_value(&result, "send_transaction result")?;
        if submitted_tx_hash != projection.raw_tx_hash {
            return Err(CkbAdapterError::EvidenceMismatch("send_transaction returned a different transaction hash".to_owned()));
        }
        let mut observed = None;
        for attempt in 0..poll_attempts {
            let value = self.rpc.call("get_transaction", json!([hash_hex(&submitted_tx_hash), "0x2", false]))?;
            if !value.is_null() {
                let status = value
                    .pointer("/tx_status/status")
                    .and_then(Value::as_str)
                    .ok_or_else(|| CkbAdapterError::InvalidResponse("get_transaction is missing tx_status.status".to_owned()))?;
                if matches!(status, "pending" | "proposed" | "committed") {
                    observed = Some((status.to_owned(), value));
                    break;
                }
                if status == "rejected" {
                    return Err(CkbAdapterError::EvidenceMismatch("node reported the submitted transaction as rejected".to_owned()));
                }
            }
            if attempt + 1 < poll_attempts && !poll_interval.is_zero() {
                thread::sleep(poll_interval);
            }
        }
        let (observed_status, observation) = observed.ok_or(CkbAdapterError::NodeObservationTimeout)?;
        let observed_hash = observation
            .pointer("/transaction/hash")
            .or_else(|| observation.pointer("/transaction/inner/hash"))
            .map(|value| parse_hash_value(value, "observed transaction hash"))
            .transpose()?;
        if observed_hash.is_some_and(|hash| hash != submitted_tx_hash) {
            return Err(CkbAdapterError::EvidenceMismatch("get_transaction returned a different transaction".to_owned()));
        }
        let block_hash = observation
            .pointer("/tx_status/block_hash")
            .filter(|value| !value.is_null())
            .map(|value| parse_hash_value(value, "block_hash"))
            .transpose()?;
        let block_number = observation
            .pointer("/tx_status/block_number")
            .filter(|value| !value.is_null())
            .map(|value| parse_quantity_value(value, "block_number"))
            .transpose()?;
        let mut node = NodeAcceptedReceipt {
            schema: NODE_SCHEMA.to_owned(),
            scripts_receipt_commitment: projection.scripts.receipt_commitment,
            raw_tx_hash: projection.raw_tx_hash,
            submitted_tx_hash,
            observed_status,
            block_hash,
            block_number,
            observation_hash: canonical_json_hash(&observation),
            receipt_commitment: [0; 32],
        };
        node.receipt_commitment = node_commitment(&node);
        projection.stage = ProjectionStage::NodeAccepted;
        projection.node = Some(node);
        verify_projection(tx, &projection)?;
        Ok(projection)
    }

    /// Wait until the accepted transaction is committed in the canonical chain
    /// and require CKB to generate and verify its inclusion proof.
    pub fn observe_committed(
        &self,
        tx: &CellTx,
        mut projection: CkbEvidenceProjection,
        poll_attempts: usize,
        poll_interval: Duration,
    ) -> Result<CkbEvidenceProjection, CkbAdapterError> {
        verify_projection(tx, &projection)?;
        if projection.stage != ProjectionStage::NodeAccepted
            || projection.node.is_none()
            || projection.committed.is_some()
            || projection.finalized.is_some()
            || poll_attempts == 0
        {
            return Err(CkbAdapterError::InvalidRequest("projection is not ready for committed observation".to_owned()));
        }

        let mut committed_observation = None;
        for attempt in 0..poll_attempts {
            let value = self.rpc.call("get_transaction", json!([hash_hex(&projection.raw_tx_hash), "0x2", false]))?;
            if !value.is_null() {
                match value.pointer("/tx_status/status").and_then(Value::as_str) {
                    Some("committed") => {
                        committed_observation = Some(value);
                        break;
                    }
                    Some("rejected") => {
                        return Err(CkbAdapterError::EvidenceMismatch(
                            "node reported the previously accepted transaction as rejected".to_owned(),
                        ));
                    }
                    Some("pending" | "proposed") => {}
                    Some(status) => {
                        return Err(CkbAdapterError::InvalidResponse(format!(
                            "unsupported transaction status while waiting for commitment: {status}"
                        )));
                    }
                    None => return Err(CkbAdapterError::InvalidResponse("get_transaction is missing tx_status.status".to_owned())),
                }
            }
            if attempt + 1 < poll_attempts && !poll_interval.is_zero() {
                thread::sleep(poll_interval);
            }
        }
        let committed_observation = committed_observation.ok_or(CkbAdapterError::CommitObservationTimeout)?;
        verify_transaction_observation(&committed_observation, projection.raw_tx_hash, "committed")?;
        let block_hash = parse_hash_value(
            committed_observation
                .pointer("/tx_status/block_hash")
                .ok_or_else(|| CkbAdapterError::InvalidResponse("committed transaction has no block_hash".to_owned()))?,
            "committed block_hash",
        )?;
        let block_number = parse_quantity_value(
            committed_observation
                .pointer("/tx_status/block_number")
                .ok_or_else(|| CkbAdapterError::InvalidResponse("committed transaction has no block_number".to_owned()))?,
            "committed block_number",
        )?;
        let block = parse_header(&self.rpc.call("get_header", json!([hash_hex(&block_hash), null]))?)?;
        block.verify_hash()?;
        if block.hash != block_hash || block.number != block_number {
            return Err(CkbAdapterError::EvidenceMismatch("committed transaction and block header disagree".to_owned()));
        }
        let canonical = self.rpc.call("get_header_by_number", json!([quantity_hex(block_number), null]))?;
        if canonical.is_null() || parse_header(&canonical)?.hash != block_hash {
            return Err(CkbAdapterError::ReorgDetected);
        }

        let transaction_proof =
            self.rpc.call("get_transaction_proof", json!([[hash_hex(&projection.raw_tx_hash)], hash_hex(&block_hash)]))?;
        if parse_hash_field(&transaction_proof, "block_hash")? != block_hash {
            return Err(CkbAdapterError::EvidenceMismatch("transaction proof is bound to a different block".to_owned()));
        }
        verify_ckb_transaction_proof(&transaction_proof, projection.raw_tx_hash, block.transactions_root)?;
        let verified = self.rpc.call("verify_transaction_proof", json!([transaction_proof.clone()]))?;
        let proof_verified_tx_hashes = verified
            .as_array()
            .ok_or_else(|| CkbAdapterError::InvalidResponse("verify_transaction_proof result must be an array".to_owned()))?
            .iter()
            .map(|value| parse_hash_value(value, "proof-verified transaction hash"))
            .collect::<Result<Vec<_>, _>>()?;
        if proof_verified_tx_hashes != vec![projection.raw_tx_hash] {
            return Err(CkbAdapterError::EvidenceMismatch(
                "CKB inclusion proof did not recover exactly the submitted transaction".to_owned(),
            ));
        }

        let mut committed = CommittedReceipt {
            schema: COMMITTED_SCHEMA.to_owned(),
            node_receipt_commitment: projection.node.as_ref().expect("checked above").receipt_commitment,
            raw_tx_hash: projection.raw_tx_hash,
            block,
            committed_observation,
            transaction_proof,
            proof_verified_tx_hashes,
            receipt_commitment: [0; 32],
        };
        committed.receipt_commitment = committed_commitment(&committed);
        projection.stage = ProjectionStage::Committed;
        projection.committed = Some(committed);
        verify_projection(tx, &projection)?;
        Ok(projection)
    }

    /// Wait until a committed transaction reaches the configured confirmation
    /// depth, while re-resolving both the transaction and its canonical block.
    pub fn wait_for_finality(
        &self,
        tx: &CellTx,
        mut projection: CkbEvidenceProjection,
        min_confirmations: u64,
        poll_attempts: usize,
        poll_interval: Duration,
    ) -> Result<CkbEvidenceProjection, CkbAdapterError> {
        verify_projection(tx, &projection)?;
        if projection.stage != ProjectionStage::Committed
            || projection.committed.is_none()
            || projection.finalized.is_some()
            || min_confirmations == 0
            || poll_attempts == 0
        {
            return Err(CkbAdapterError::InvalidRequest("projection is not ready for finality observation".to_owned()));
        }
        let committed = projection.committed.as_ref().expect("checked above");

        for attempt in 0..poll_attempts {
            let committed_observation = self.rpc.call("get_transaction", json!([hash_hex(&projection.raw_tx_hash), "0x2", false]))?;
            if committed_observation.is_null()
                || verify_transaction_observation(&committed_observation, projection.raw_tx_hash, "committed").is_err()
            {
                return Err(CkbAdapterError::ReorgDetected);
            }
            let observed_block_hash = parse_hash_value(
                committed_observation
                    .pointer("/tx_status/block_hash")
                    .ok_or_else(|| CkbAdapterError::InvalidResponse("committed transaction has no block_hash".to_owned()))?,
                "committed block_hash",
            )?;
            if observed_block_hash != committed.block.hash {
                return Err(CkbAdapterError::ReorgDetected);
            }

            let canonical_value = self.rpc.call("get_header_by_number", json!([quantity_hex(committed.block.number), null]))?;
            if canonical_value.is_null() {
                return Err(CkbAdapterError::ReorgDetected);
            }
            let canonical_block = parse_header(&canonical_value)?;
            canonical_block.verify_hash()?;
            if canonical_block != committed.block {
                return Err(CkbAdapterError::ReorgDetected);
            }
            let tip = parse_header(&self.rpc.call("get_tip_header", json!([]))?)?;
            tip.verify_hash()?;
            let confirmations = tip.number.saturating_sub(committed.block.number);
            if tip.number >= committed.block.number && confirmations >= min_confirmations {
                let mut finalized = FinalizedReceipt {
                    schema: FINALITY_SCHEMA.to_owned(),
                    committed_receipt_commitment: committed.receipt_commitment,
                    raw_tx_hash: projection.raw_tx_hash,
                    min_confirmations,
                    confirmations,
                    tip,
                    canonical_block,
                    committed_observation,
                    receipt_commitment: [0; 32],
                };
                finalized.receipt_commitment = finalized_commitment(&finalized);
                projection.stage = ProjectionStage::Finalized;
                projection.finalized = Some(finalized);
                verify_projection(tx, &projection)?;
                return Ok(projection);
            }
            if attempt + 1 < poll_attempts && !poll_interval.is_zero() {
                thread::sleep(poll_interval);
            }
        }
        Err(CkbAdapterError::FinalityObservationTimeout { min_confirmations })
    }

    fn resolve_cell(
        &self,
        role: ResolvedCellRole,
        index: usize,
        out_point: OutPoint,
        expanded_from: Option<OutPoint>,
    ) -> Result<ResolvedCellEvidence, CkbAdapterError> {
        let value = self.rpc.call("get_live_cell", json!([ckb_json_out_point(&out_point), true, false]))?;
        if value.get("status").and_then(Value::as_str) != Some("live") {
            return Err(CkbAdapterError::MissingLiveCell(out_point));
        }
        let cell = value.get("cell").ok_or(CkbAdapterError::MissingLiveCell(out_point))?;
        let output = parse_cell_output(
            cell.get("output").ok_or_else(|| CkbAdapterError::InvalidResponse("live Cell has no output".to_owned()))?,
        )?;
        let data_value = cell.get("data").ok_or_else(|| CkbAdapterError::InvalidResponse("live Cell has no data".to_owned()))?;
        let data = parse_bytes_value(
            data_value.get("content").ok_or_else(|| CkbAdapterError::InvalidResponse("live Cell data has no content".to_owned()))?,
            "Cell data content",
        )?;
        let data_hash = parse_hash_value(
            data_value.get("hash").ok_or_else(|| CkbAdapterError::InvalidResponse("live Cell data has no hash".to_owned()))?,
            "Cell data hash",
        )?;
        if ckb_cell_data_hash(&data) != data_hash {
            return Err(CkbAdapterError::EvidenceMismatch(format!("CKB returned inconsistent data hash for {out_point}")));
        }
        let tx = self.rpc.call("get_transaction", json!([hash_hex(&out_point.tx_hash), "0x2", true]))?;
        let creation_block_hash = tx
            .pointer("/tx_status/block_hash")
            .filter(|value| !value.is_null())
            .map(|value| parse_hash_value(value, "creation block hash"))
            .transpose()?;
        Ok(ResolvedCellEvidence { role, index, out_point, expanded_from, output, data, data_hash, creation_block_hash })
    }

    fn require_tip(&self, expected: [u8; 32]) -> Result<(), CkbAdapterError> {
        let actual = parse_header(&self.rpc.call("get_tip_header", json!([]))?)?;
        if actual.hash != expected {
            return Err(CkbAdapterError::StaleContext { expected, actual: actual.hash });
        }
        Ok(())
    }
}

/// Verify the complete receipt chain without trusting serialized stage labels.
pub fn verify_projection(tx: &CellTx, projection: &CkbEvidenceProjection) -> Result<(), CkbAdapterError> {
    if projection.schema != PROJECTION_SCHEMA || projection.raw_tx_hash != projection.context.raw_tx_hash {
        return Err(CkbAdapterError::EvidenceMismatch("projection envelope is invalid".to_owned()));
    }
    projection.context.verify(tx)?;
    let consensus = &projection.consensus;
    if consensus.schema != CONSENSUS_SCHEMA
        || consensus.context_commitment != projection.context.context_commitment
        || consensus.raw_tx_hash != projection.raw_tx_hash
        || consensus.validation_tip_hash != projection.context.tip.hash
        || consensus.receipt_commitment != consensus_commitment(consensus)
    {
        return Err(CkbAdapterError::EvidenceMismatch("consensus receipt chain is invalid".to_owned()));
    }
    let scripts = &projection.scripts;
    if scripts.schema != SCRIPTS_SCHEMA
        || scripts.consensus_receipt_commitment != consensus.receipt_commitment
        || scripts.context_commitment != projection.context.context_commitment
        || scripts.raw_tx_hash != projection.raw_tx_hash
        || scripts.local_vm_cycles > scripts.max_cycles
        || scripts.node_vm_cycles > scripts.max_cycles
        || scripts.receipt_commitment != scripts_commitment(scripts)
    {
        return Err(CkbAdapterError::EvidenceMismatch("scripts receipt chain is invalid".to_owned()));
    }
    match (&projection.node, &projection.committed, &projection.finalized, projection.stage) {
        (None, None, None, ProjectionStage::ScriptsVerified) => Ok(()),
        (Some(node), None, None, ProjectionStage::NodeAccepted) => verify_node_receipt(node, scripts, projection.raw_tx_hash),
        (Some(node), Some(committed), None, ProjectionStage::Committed) => {
            verify_node_receipt(node, scripts, projection.raw_tx_hash)?;
            verify_committed_receipt(committed, node, projection.raw_tx_hash)
        }
        (Some(node), Some(committed), Some(finalized), ProjectionStage::Finalized) => {
            verify_node_receipt(node, scripts, projection.raw_tx_hash)?;
            verify_committed_receipt(committed, node, projection.raw_tx_hash)?;
            verify_finalized_receipt(finalized, committed, projection.raw_tx_hash)
        }
        _ => Err(CkbAdapterError::EvidenceMismatch("projection stage is not supported by its receipts".to_owned())),
    }
}

fn verify_node_receipt(
    node: &NodeAcceptedReceipt,
    scripts: &ScriptsVerifiedReceipt,
    raw_tx_hash: [u8; 32],
) -> Result<(), CkbAdapterError> {
    if node.schema != NODE_SCHEMA
        || node.scripts_receipt_commitment != scripts.receipt_commitment
        || node.raw_tx_hash != raw_tx_hash
        || node.submitted_tx_hash != raw_tx_hash
        || !matches!(node.observed_status.as_str(), "pending" | "proposed" | "committed")
        || node.receipt_commitment != node_commitment(node)
    {
        return Err(CkbAdapterError::EvidenceMismatch("node-acceptance receipt chain is invalid".to_owned()));
    }
    Ok(())
}

fn verify_committed_receipt(
    committed: &CommittedReceipt,
    node: &NodeAcceptedReceipt,
    raw_tx_hash: [u8; 32],
) -> Result<(), CkbAdapterError> {
    committed.block.verify_hash()?;
    verify_transaction_observation(&committed.committed_observation, raw_tx_hash, "committed")?;
    verify_ckb_transaction_proof(&committed.transaction_proof, raw_tx_hash, committed.block.transactions_root)?;
    let observed_block_hash = parse_hash_value(
        committed
            .committed_observation
            .pointer("/tx_status/block_hash")
            .ok_or_else(|| CkbAdapterError::EvidenceMismatch("committed receipt has no observed block hash".to_owned()))?,
        "committed block_hash",
    )?;
    let observed_block_number = parse_quantity_value(
        committed
            .committed_observation
            .pointer("/tx_status/block_number")
            .ok_or_else(|| CkbAdapterError::EvidenceMismatch("committed receipt has no observed block number".to_owned()))?,
        "committed block_number",
    )?;
    if committed.schema != COMMITTED_SCHEMA
        || committed.node_receipt_commitment != node.receipt_commitment
        || committed.raw_tx_hash != raw_tx_hash
        || committed.block.hash != observed_block_hash
        || committed.block.number != observed_block_number
        || parse_hash_field(&committed.transaction_proof, "block_hash")? != committed.block.hash
        || committed.proof_verified_tx_hashes != vec![raw_tx_hash]
        || committed.receipt_commitment != committed_commitment(committed)
    {
        return Err(CkbAdapterError::EvidenceMismatch("committed receipt chain is invalid".to_owned()));
    }
    Ok(())
}

fn verify_finalized_receipt(
    finalized: &FinalizedReceipt,
    committed: &CommittedReceipt,
    raw_tx_hash: [u8; 32],
) -> Result<(), CkbAdapterError> {
    finalized.tip.verify_hash()?;
    finalized.canonical_block.verify_hash()?;
    verify_transaction_observation(&finalized.committed_observation, raw_tx_hash, "committed")?;
    let observed_block_hash = parse_hash_value(
        finalized
            .committed_observation
            .pointer("/tx_status/block_hash")
            .ok_or_else(|| CkbAdapterError::EvidenceMismatch("finality receipt has no observed block hash".to_owned()))?,
        "finalized block_hash",
    )?;
    let confirmations = finalized.tip.number.saturating_sub(committed.block.number);
    if finalized.schema != FINALITY_SCHEMA
        || finalized.committed_receipt_commitment != committed.receipt_commitment
        || finalized.raw_tx_hash != raw_tx_hash
        || finalized.min_confirmations == 0
        || finalized.tip.number < committed.block.number
        || finalized.confirmations != confirmations
        || confirmations < finalized.min_confirmations
        || finalized.canonical_block != committed.block
        || observed_block_hash != committed.block.hash
        || finalized.receipt_commitment != finalized_commitment(finalized)
    {
        return Err(CkbAdapterError::EvidenceMismatch("finality receipt chain is invalid".to_owned()));
    }
    Ok(())
}

/// Convert a Myelin CellTx to exact CKB JSON-RPC transaction shape.
pub fn ckb_json_transaction(tx: &CellTx) -> Result<Value, CkbAdapterError> {
    Ok(json!({
        "version": quantity_hex(u64::from(tx.version)),
        "cell_deps": tx.cell_deps.iter().map(ckb_json_cell_dep).collect::<Result<Vec<_>, _>>()?,
        "header_deps": tx.header_deps.iter().map(hash_hex).collect::<Vec<_>>(),
        "inputs": tx.inputs.iter().map(ckb_json_cell_input).collect::<Vec<_>>(),
        "outputs": tx.outputs.iter().map(ckb_json_cell_output).collect::<Result<Vec<_>, _>>()?,
        "outputs_data": tx.outputs_data.iter().map(|data| bytes_hex(data)).collect::<Vec<_>>(),
        "witnesses": tx.witnesses.iter().map(|data| bytes_hex(data)).collect::<Vec<_>>(),
    }))
}

/// Parse an exact CKB JSON-RPC transaction object into Myelin's isomorphic CellTx.
pub fn parse_ckb_json_transaction(value: &Value) -> Result<CellTx, CkbAdapterError> {
    let version = u32::try_from(parse_quantity_field(value, "version")?)
        .map_err(|_| CkbAdapterError::InvalidResponse("transaction version exceeds u32".to_owned()))?;
    if version != 0 {
        return Err(CkbAdapterError::InvalidRequest(format!("CKB transaction version must be zero, got {version}")));
    }
    let inputs = value
        .get("inputs")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction inputs must be an array".to_owned()))?
        .iter()
        .map(|input| {
            Ok(CellInput::new(
                parse_out_point(
                    input
                        .get("previous_output")
                        .ok_or_else(|| CkbAdapterError::InvalidResponse("input missing previous_output".to_owned()))?,
                )?,
                parse_quantity_field(input, "since")?,
            ))
        })
        .collect::<Result<Vec<_>, CkbAdapterError>>()?;
    let cell_deps = value
        .get("cell_deps")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction cell_deps must be an array".to_owned()))?
        .iter()
        .map(|dep| {
            let dep_type = match dep.get("dep_type").and_then(Value::as_str) {
                Some("code") => DepType::Code,
                Some("dep_group") => DepType::DepGroup,
                _ => return Err(CkbAdapterError::InvalidResponse("unsupported CellDep dep_type".to_owned())),
            };
            Ok(CellDep {
                out_point: parse_out_point(
                    dep.get("out_point").ok_or_else(|| CkbAdapterError::InvalidResponse("CellDep missing out_point".to_owned()))?,
                )?,
                dep_type,
            })
        })
        .collect::<Result<Vec<_>, CkbAdapterError>>()?;
    let header_deps = value
        .get("header_deps")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction header_deps must be an array".to_owned()))?
        .iter()
        .map(|hash| parse_hash_value(hash, "header_dep"))
        .collect::<Result<Vec<_>, _>>()?;
    let outputs = value
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction outputs must be an array".to_owned()))?
        .iter()
        .map(parse_cell_output)
        .collect::<Result<Vec<_>, _>>()?;
    let outputs_data = parse_hex_array(value, "outputs_data")?;
    let witnesses = parse_hex_array(value, "witnesses")?;
    CellTx::new_with_header_deps(inputs, cell_deps, header_deps, outputs, outputs_data, witnesses)
        .map_err(|error| CkbAdapterError::InvalidRequest(error.to_owned()))
}

fn parse_out_point(value: &Value) -> Result<OutPoint, CkbAdapterError> {
    Ok(OutPoint::new(
        parse_hash_field(value, "tx_hash")?,
        u32::try_from(parse_quantity_field(value, "index")?)
            .map_err(|_| CkbAdapterError::InvalidResponse("OutPoint index exceeds u32".to_owned()))?,
    ))
}

fn parse_hex_array(value: &Value, field: &str) -> Result<Vec<Vec<u8>>, CkbAdapterError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse(format!("transaction {field} must be an array")))?
        .iter()
        .map(|item| parse_bytes_value(item, field))
        .collect()
}

fn verify_transaction_observation(value: &Value, expected_hash: [u8; 32], expected_status: &str) -> Result<(), CkbAdapterError> {
    let status = value
        .pointer("/tx_status/status")
        .and_then(Value::as_str)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("get_transaction is missing tx_status.status".to_owned()))?;
    if status != expected_status {
        return Err(CkbAdapterError::EvidenceMismatch(format!("expected transaction status {expected_status}, observed {status}")));
    }
    let observed_hash = value
        .pointer("/transaction/hash")
        .or_else(|| value.pointer("/transaction/inner/hash"))
        .ok_or_else(|| CkbAdapterError::InvalidResponse("get_transaction is missing transaction hash".to_owned()))?;
    if parse_hash_value(observed_hash, "observed transaction hash")? != expected_hash {
        return Err(CkbAdapterError::EvidenceMismatch("get_transaction returned a different transaction".to_owned()));
    }
    Ok(())
}

fn verify_ckb_transaction_proof(
    value: &Value,
    raw_tx_hash: [u8; 32],
    expected_transactions_root: [u8; 32],
) -> Result<(), CkbAdapterError> {
    let proof =
        value.get("proof").ok_or_else(|| CkbAdapterError::InvalidResponse("transaction proof has no proof object".to_owned()))?;
    let indices = proof
        .get("indices")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction proof indices must be an array".to_owned()))?
        .iter()
        .map(|value| {
            u32::try_from(parse_quantity_value(value, "transaction proof index")?)
                .map_err(|_| CkbAdapterError::InvalidResponse("transaction proof index exceeds u32".to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let lemmas = proof
        .get("lemmas")
        .and_then(Value::as_array)
        .ok_or_else(|| CkbAdapterError::InvalidResponse("transaction proof lemmas must be an array".to_owned()))?
        .iter()
        .map(|value| parse_hash_value(value, "transaction proof lemma"))
        .collect::<Result<Vec<_>, _>>()?;
    let witnesses_root = parse_hash_field(value, "witnesses_root")?;
    let raw_transactions_root = MerkleProof::<[u8; 32], CkbMerkleMerge>::new(indices, lemmas)
        .root(&[raw_tx_hash])
        .ok_or_else(|| CkbAdapterError::EvidenceMismatch("transaction inclusion proof is invalid".to_owned()))?;
    let transactions_root = CBMT::<[u8; 32], CkbMerkleMerge>::build_merkle_root(&[raw_transactions_root, witnesses_root]);
    if transactions_root != expected_transactions_root {
        return Err(CkbAdapterError::EvidenceMismatch(
            "transaction inclusion proof does not match the committed header root".to_owned(),
        ));
    }
    Ok(())
}

fn context_provider(context: &ContextResolvedReceipt) -> Result<SimpleDataProvider, CkbAdapterError> {
    let mut provider = SimpleDataProvider::new();
    let header_hashes = context.headers.iter().map(|header| header.hash).collect::<BTreeSet<_>>();
    for header in &context.headers {
        provider.add_ckb_header(header.hash, header.packed());
    }
    for cell in &context.cells {
        let resolved = ResolvedCell { cell_output: cell.output.clone(), data: Some(cell.data.clone()) };
        if let Some(header_hash) = cell.creation_block_hash.filter(|hash| header_hashes.contains(hash)) {
            provider.add_cell_with_header(cell.out_point.tx_hash, cell.out_point.index, resolved, header_hash);
        } else {
            provider.add_cell(cell.out_point.tx_hash, cell.out_point.index, resolved);
        }
    }
    Ok(provider)
}

fn require_cell(
    context: &ContextResolvedReceipt,
    role: ResolvedCellRole,
    index: usize,
    out_point: OutPoint,
    expanded_from: Option<OutPoint>,
) -> Result<&ResolvedCellEvidence, CkbAdapterError> {
    let mut matches = context
        .cells
        .iter()
        .filter(|cell| cell.role == role && cell.index == index && cell.out_point == out_point && cell.expanded_from == expanded_from);
    let cell = matches.next().ok_or_else(|| CkbAdapterError::EvidenceMismatch(format!("missing resolved Cell {out_point}")))?;
    if matches.next().is_some() {
        return Err(CkbAdapterError::EvidenceMismatch(format!("duplicate resolved Cell {out_point}")));
    }
    Ok(cell)
}

fn parse_header(value: &Value) -> Result<CkbHeaderEvidence, CkbAdapterError> {
    if value.is_null() {
        return Err(CkbAdapterError::InvalidResponse("CKB header is null".to_owned()));
    }
    let header = value.get("inner").or_else(|| value.get("header")).unwrap_or(value);
    let evidence = CkbHeaderEvidence {
        hash: parse_hash_field(header, "hash")?,
        version: u32::try_from(parse_quantity_field(header, "version")?)
            .map_err(|_| CkbAdapterError::InvalidResponse("header version exceeds u32".to_owned()))?,
        compact_target: u32::try_from(parse_quantity_field(header, "compact_target")?)
            .map_err(|_| CkbAdapterError::InvalidResponse("header compact_target exceeds u32".to_owned()))?,
        timestamp: parse_quantity_field(header, "timestamp")?,
        number: parse_quantity_field(header, "number")?,
        epoch: parse_quantity_field(header, "epoch")?,
        parent_hash: parse_hash_field(header, "parent_hash")?,
        transactions_root: parse_hash_field(header, "transactions_root")?,
        proposals_hash: parse_hash_field(header, "proposals_hash")?,
        extra_hash: parse_hash_field(header, "extra_hash")?,
        dao: parse_hash_field(header, "dao")?,
        nonce: parse_quantity_u128(
            header.get("nonce").ok_or_else(|| CkbAdapterError::InvalidResponse("header missing nonce".to_owned()))?,
            "nonce",
        )?,
    };
    evidence.verify_hash()?;
    Ok(evidence)
}

fn parse_cell_output(value: &Value) -> Result<CellOutput, CkbAdapterError> {
    Ok(CellOutput {
        capacity: parse_quantity_field(value, "capacity")?,
        lock: parse_script(value.get("lock").ok_or_else(|| CkbAdapterError::InvalidResponse("Cell output missing lock".to_owned()))?)?,
        type_: value.get("type").filter(|value| !value.is_null()).map(parse_script).transpose()?,
    })
}

fn parse_script(value: &Value) -> Result<Script, CkbAdapterError> {
    let hash_type = match value.get("hash_type").and_then(Value::as_str) {
        Some("data") => 0,
        Some("type") => 1,
        Some("data1") => 2,
        Some("data2") => 4,
        _ => return Err(CkbAdapterError::InvalidResponse("unsupported CKB script hash_type".to_owned())),
    };
    Ok(Script {
        code_hash: parse_hash_field(value, "code_hash")?,
        hash_type,
        args: parse_bytes_value(
            value.get("args").ok_or_else(|| CkbAdapterError::InvalidResponse("Script missing args".to_owned()))?,
            "script args",
        )?,
    })
}

fn ckb_json_cell_dep(dep: &CellDep) -> Result<Value, CkbAdapterError> {
    Ok(json!({
        "out_point": ckb_json_out_point(&dep.out_point),
        "dep_type": match dep.dep_type { DepType::Code => "code", DepType::DepGroup => "dep_group" },
    }))
}

fn ckb_json_cell_input(input: &CellInput) -> Value {
    json!({ "previous_output": ckb_json_out_point(&input.previous_output), "since": quantity_hex(input.since) })
}

fn ckb_json_cell_output(output: &CellOutput) -> Result<Value, CkbAdapterError> {
    Ok(json!({
        "capacity": quantity_hex(output.capacity),
        "lock": ckb_json_script(&output.lock)?,
        "type": output.type_.as_ref().map(ckb_json_script).transpose()?,
    }))
}

fn ckb_json_script(script: &Script) -> Result<Value, CkbAdapterError> {
    let hash_type = match script.hash_type {
        0 => "data",
        1 => "type",
        2 => "data1",
        4 => "data2",
        value => return Err(CkbAdapterError::InvalidRequest(format!("unsupported CKB script hash type {value}"))),
    };
    Ok(json!({ "code_hash": hash_hex(&script.code_hash), "hash_type": hash_type, "args": bytes_hex(&script.args) }))
}

fn ckb_json_out_point(out_point: &OutPoint) -> Value {
    json!({ "tx_hash": hash_hex(&out_point.tx_hash), "index": quantity_hex(u64::from(out_point.index)) })
}

fn parse_hash_field(value: &Value, field: &str) -> Result<[u8; 32], CkbAdapterError> {
    parse_hash_value(value.get(field).ok_or_else(|| CkbAdapterError::InvalidResponse(format!("missing {field}")))?, field)
}

fn parse_hash_value(value: &Value, field: &str) -> Result<[u8; 32], CkbAdapterError> {
    let bytes = parse_bytes_value(value, field)?;
    bytes.try_into().map_err(|_| CkbAdapterError::InvalidResponse(format!("{field} must be 32 bytes")))
}

fn parse_bytes_value(value: &Value, field: &str) -> Result<Vec<u8>, CkbAdapterError> {
    let text = value.as_str().ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} must be hex data")))?;
    let hex = text.strip_prefix("0x").ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} lacks 0x prefix")))?;
    hex::decode(hex).map_err(|_| CkbAdapterError::InvalidResponse(format!("{field} is invalid hex")))
}

fn parse_quantity_field(value: &Value, field: &str) -> Result<u64, CkbAdapterError> {
    parse_quantity_value(value.get(field).ok_or_else(|| CkbAdapterError::InvalidResponse(format!("missing {field}")))?, field)
}

fn parse_quantity_value(value: &Value, field: &str) -> Result<u64, CkbAdapterError> {
    let text = value.as_str().ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} must be a CKB hex quantity")))?;
    let hex = text
        .strip_prefix("0x")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} is not a CKB hex quantity")))?;
    u64::from_str_radix(hex, 16).map_err(|_| CkbAdapterError::InvalidResponse(format!("{field} exceeds u64")))
}

fn parse_quantity_u128(value: &Value, field: &str) -> Result<u128, CkbAdapterError> {
    let text = value.as_str().ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} must be a CKB hex quantity")))?;
    let hex = text
        .strip_prefix("0x")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CkbAdapterError::InvalidResponse(format!("{field} is not a CKB hex quantity")))?;
    u128::from_str_radix(hex, 16).map_err(|_| CkbAdapterError::InvalidResponse(format!("{field} exceeds u128")))
}

fn value_string(value: &Value, pointers: &[&str]) -> Result<String, CkbAdapterError> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| CkbAdapterError::InvalidResponse(format!("missing string at one of {pointers:?}")))
}

fn quantity_hex(value: u64) -> String {
    format!("0x{value:x}")
}

fn bytes_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

fn hash_hex(hash: &[u8; 32]) -> String {
    bytes_hex(hash)
}

fn canonical_json_hash(value: &Value) -> [u8; 32] {
    let canonical = canonicalize_json(value);
    domain_hash(b"myelin:ckb-canonical-json:v1", &serde_json::to_vec(&canonical).expect("JSON value serialization cannot fail"))
}

fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted = map.iter().map(|(key, value)| (key.clone(), canonicalize_json(value))).collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

fn context_commitment(receipt: &ContextResolvedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-context-resolved-receipt:v1",
        &(
            &receipt.schema,
            &receipt.chain,
            &receipt.node_version,
            receipt.genesis_hash,
            receipt.consensus_rules_hash,
            receipt.raw_tx_hash,
            receipt.transaction_json_hash,
            &receipt.tip,
            &receipt.cells,
            &receipt.headers,
        ),
    )
}

fn consensus_commitment(receipt: &ConsensusValidatedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-consensus-validated-receipt:v1",
        &(
            &receipt.schema,
            receipt.context_commitment,
            receipt.raw_tx_hash,
            receipt.validation_tip_hash,
            receipt.fee,
            receipt.cycles,
            receipt.node_result_hash,
        ),
    )
}

fn scripts_commitment(receipt: &ScriptsVerifiedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-scripts-verified-receipt:v1",
        &(
            &receipt.schema,
            receipt.consensus_receipt_commitment,
            receipt.context_commitment,
            receipt.raw_tx_hash,
            receipt.max_cycles,
            receipt.local_vm_cycles,
            receipt.node_vm_cycles,
        ),
    )
}

fn node_commitment(receipt: &NodeAcceptedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-node-accepted-receipt:v1",
        &(
            &receipt.schema,
            receipt.scripts_receipt_commitment,
            receipt.raw_tx_hash,
            receipt.submitted_tx_hash,
            &receipt.observed_status,
            receipt.block_hash,
            receipt.block_number,
            receipt.observation_hash,
        ),
    )
}

fn committed_commitment(receipt: &CommittedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-committed-receipt:v1",
        &(
            &receipt.schema,
            receipt.node_receipt_commitment,
            receipt.raw_tx_hash,
            &receipt.block,
            &receipt.committed_observation,
            &receipt.transaction_proof,
            &receipt.proof_verified_tx_hashes,
        ),
    )
}

fn finalized_commitment(receipt: &FinalizedReceipt) -> [u8; 32] {
    hash_serializable(
        b"myelin:ckb-finalized-receipt:v1",
        &(
            &receipt.schema,
            receipt.committed_receipt_commitment,
            receipt.raw_tx_hash,
            receipt.min_confirmations,
            receipt.confirmations,
            &receipt.tip,
            &receipt.canonical_block,
            &receipt.committed_observation,
        ),
    )
}

fn hash_serializable(domain: &[u8], value: &impl Serialize) -> [u8; 32] {
    domain_hash(domain, &serde_json::to_vec(value).expect("receipt serialization cannot fail"))
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

/// CKB adapter failure.
#[derive(Debug, thiserror::Error)]
pub enum CkbAdapterError {
    /// RPC URL is invalid.
    #[error("invalid CKB RPC URL: {0}")]
    InvalidRpcUrl(String),
    /// HTTP transport failed.
    #[error("CKB RPC HTTP error: {0}")]
    Http(#[source] reqwest::Error),
    /// CKB returned a JSON-RPC error.
    #[error("CKB RPC method {method} rejected the request: {error}")]
    RpcRejected {
        /// RPC method.
        method: String,
        /// Structured CKB error.
        error: Value,
    },
    /// CKB returned malformed or incomplete data.
    #[error("invalid CKB RPC response: {0}")]
    InvalidResponse(String),
    /// Request parameters violate the evidence contract.
    #[error("invalid CKB evidence request: {0}")]
    InvalidRequest(String),
    /// Canonical CKB encoding failed.
    #[error("CKB encoding failed: {0}")]
    Encoding(String),
    /// CKB DepGroup data is malformed.
    #[error("invalid CKB DepGroup: {0}")]
    DepGroup(String),
    /// Referenced Cell is not live in the stable chain snapshot.
    #[error("CKB Cell is not live: {0}")]
    MissingLiveCell(OutPoint),
    /// Tip changed repeatedly during context resolution.
    #[error("CKB tip did not remain stable while resolving the transaction context")]
    UnstableTip,
    /// Previously resolved context is stale.
    #[error("CKB context is stale: expected tip {expected:02x?}, observed {actual:02x?}")]
    StaleContext {
        /// Expected tip hash.
        expected: [u8; 32],
        /// Current tip hash.
        actual: [u8; 32],
    },
    /// Receipt mutation, replay or cross-transaction binding was detected.
    #[error("CKB evidence mismatch: {0}")]
    EvidenceMismatch(String),
    /// Strict local CKB-VM verification failed.
    #[error("strict local CKB-VM verification failed: {0}")]
    LocalVm(String),
    /// A submitted transaction never became observable in the node.
    #[error("submitted transaction was not observable before the polling limit")]
    NodeObservationTimeout,
    /// An accepted transaction did not become committed before the polling limit.
    #[error("accepted transaction was not committed before the polling limit")]
    CommitObservationTimeout,
    /// A committed transaction did not reach the requested confirmation depth.
    #[error("committed transaction did not reach {min_confirmations} confirmations before the polling limit")]
    FinalityObservationTimeout {
        /// Required confirmation depth.
        min_confirmations: u64,
    },
    /// Canonical block identity or committed transaction status changed.
    #[error("CKB reorganization detected while advancing projection evidence")]
    ReorgDetected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::scripts::ALWAYS_SUCCESS_SCRIPT;
    use std::sync::Mutex;

    struct FixtureRpc {
        tip: Value,
        block: Value,
        canonical_block: Value,
        submitted_status: &'static str,
        input: OutPoint,
        dep: OutPoint,
        input_output: CellOutput,
        dep_output: CellOutput,
        raw_tx_hash: [u8; 32],
        calls: Mutex<Vec<String>>,
    }

    impl CkbRpc for FixtureRpc {
        fn call(&self, method: &str, params: Value) -> Result<Value, CkbAdapterError> {
            self.calls.lock().unwrap().push(method.to_owned());
            match method {
                "get_tip_header" => Ok(self.tip.clone()),
                "get_header_by_number" => Ok(self.canonical_block.clone()),
                "get_header" => Ok(self.block.clone()),
                "local_node_info" => Ok(json!({ "version": "0.207.0" })),
                "get_blockchain_info" => Ok(json!({ "chain": "ckb_dev" })),
                "get_consensus" => Ok(json!({ "id": "fixture-consensus", "max_block_cycles": "0x989680" })),
                "get_live_cell" => {
                    let out_point = params.get(0).unwrap();
                    let tx_hash = parse_hash_field(out_point, "tx_hash")?;
                    let index = u32::try_from(parse_quantity_field(out_point, "index")?).unwrap();
                    let (output, data) = if OutPoint::new(tx_hash, index) == self.input {
                        (&self.input_output, Vec::new())
                    } else if OutPoint::new(tx_hash, index) == self.dep {
                        (&self.dep_output, ALWAYS_SUCCESS_SCRIPT.to_vec())
                    } else {
                        return Ok(json!({ "status": "unknown", "cell": null }));
                    };
                    Ok(json!({
                        "status": "live",
                        "cell": {
                            "output": ckb_json_cell_output(output)?,
                            "data": { "content": bytes_hex(&data), "hash": hash_hex(&ckb_cell_data_hash(&data)) }
                        }
                    }))
                }
                "get_transaction" => {
                    let requested = parse_hash_value(params.get(0).unwrap(), "requested transaction hash")?;
                    if requested == self.raw_tx_hash {
                        let committed = self.submitted_status == "committed";
                        Ok(json!({
                            "transaction": { "hash": hash_hex(&self.raw_tx_hash) },
                            "tx_status": {
                                "status": self.submitted_status,
                                "block_hash": committed.then(|| self.block["hash"].clone()),
                                "block_number": committed.then(|| self.block["number"].clone())
                            }
                        }))
                    } else {
                        Ok(json!({
                            "transaction": { "hash": hash_hex(&requested) },
                            "tx_status": { "status": "committed", "block_hash": self.block["hash"], "block_number": self.block["number"] }
                        }))
                    }
                }
                "test_tx_pool_accept" => Ok(json!({ "cycles": "0x219", "fee": "0x64" })),
                "send_transaction" => Ok(Value::String(hash_hex(&self.raw_tx_hash))),
                "get_transaction_proof" => Ok(json!({
                    "block_hash": self.block["hash"],
                    "proof": { "indices": ["0x0"], "lemmas": [] },
                    "witnesses_root": hash_hex(&[9; 32])
                })),
                "verify_transaction_proof" => Ok(json!([hash_hex(&self.raw_tx_hash)])),
                other => Err(CkbAdapterError::InvalidResponse(format!("unexpected fixture RPC {other}"))),
            }
        }
    }

    fn header_at(number: u64, transactions_root: [u8; 32]) -> Value {
        let packed = CkbHeader {
            raw: CkbRawHeader {
                version: 0,
                compact_target: 0x2001_0000,
                timestamp: 1,
                number,
                epoch: 0,
                parent_hash: [0; 32],
                transactions_root,
                proposals_hash: [2; 32],
                extra_hash: [3; 32],
                dao: [4; 32],
            },
            nonce: 0,
        };
        let hash = ckb_header_hash_molecule(&packed).unwrap();
        json!({
            "hash": hash_hex(&hash), "version": "0x0", "compact_target": "0x20010000", "timestamp": "0x1",
            "number": quantity_hex(number), "epoch": "0x0", "parent_hash": hash_hex(&[0; 32]),
            "transactions_root": hash_hex(&transactions_root), "proposals_hash": hash_hex(&[2; 32]),
            "extra_hash": hash_hex(&[3; 32]), "dao": hash_hex(&[4; 32]), "nonce": "0x0"
        })
    }

    fn fixture() -> (FixtureRpc, CellTx) {
        let code_hash = ckb_cell_data_hash(ALWAYS_SUCCESS_SCRIPT);
        let input = OutPoint::new([7; 32], 0);
        let dep = OutPoint::new([8; 32], 0);
        let lock = Script::new(code_hash, 2, vec![]);
        let input_output = CellOutput { capacity: 1_000, lock: lock.clone(), type_: None };
        let dep_output = CellOutput { capacity: 1_000, lock: Script::new([0; 32], 0, vec![]), type_: None };
        let tx = CellTx::new(
            vec![CellInput::new(input, 0)],
            vec![CellDep { out_point: dep, dep_type: DepType::Code }],
            vec![CellOutput { capacity: 900, lock, type_: None }],
            vec![Vec::new()],
            vec![Vec::new()],
        )
        .unwrap();
        let raw_tx_hash = ckb_raw_transaction_hash_molecule(&tx).unwrap();
        let witnesses_root = [9; 32];
        let transactions_root = CkbMerkleMerge::merge(&raw_tx_hash, &witnesses_root);
        let block = header_at(0, transactions_root);
        (
            FixtureRpc {
                tip: block.clone(),
                block: block.clone(),
                canonical_block: block,
                submitted_status: "pending",
                input,
                dep,
                input_output,
                dep_output,
                raw_tx_hash,
                calls: Mutex::new(Vec::new()),
            },
            tx,
        )
    }

    #[test]
    fn resolves_and_rejects_mutated_context_receipts() {
        let (rpc, tx) = fixture();
        let engine = CkbEvidenceEngine::new(rpc);
        let receipt = engine.resolve_context(&tx, 1).unwrap();
        receipt.verify(&tx).unwrap();
        let mut mutated = receipt;
        mutated.cells[0].data_hash[0] ^= 1;
        assert!(mutated.verify(&tx).is_err());
    }

    #[test]
    fn exact_ckb_json_uses_raw_transaction_hash_identity() {
        let (_, tx) = fixture();
        let value = ckb_json_transaction(&tx).unwrap();
        assert_eq!(value["version"], "0x0");
        assert_eq!(value["cell_deps"][0]["dep_type"], "code");
        assert_eq!(
            parse_hash_value(&Value::String(hash_hex(&ckb_raw_transaction_hash_molecule(&tx).unwrap())), "hash").unwrap(),
            ckb_raw_transaction_hash_molecule(&tx).unwrap()
        );
        assert_eq!(parse_ckb_json_transaction(&value).unwrap(), tx);
    }

    #[test]
    fn advances_only_with_concrete_validation_and_node_receipts() {
        let (rpc, tx) = fixture();
        let engine = CkbEvidenceEngine::new(rpc);
        let context = engine.resolve_context(&tx, 1).unwrap();
        let verified = engine.validate_and_verify(&tx, context, 10_000).unwrap();
        assert_eq!(verified.stage, ProjectionStage::ScriptsVerified);
        assert!(verified.scripts.local_vm_cycles > 0);
        verify_projection(&tx, &verified).unwrap();

        let accepted = engine.submit_and_observe(&tx, verified, 1, Duration::ZERO).unwrap();
        assert_eq!(accepted.stage, ProjectionStage::NodeAccepted);
        assert_eq!(accepted.node.as_ref().unwrap().observed_status, "pending");
        verify_projection(&tx, &accepted).unwrap();

        let mut mutated = accepted;
        mutated.scripts.node_vm_cycles += 1;
        assert!(verify_projection(&tx, &mutated).is_err());
    }

    #[test]
    fn committed_and_finalized_receipts_bind_proof_depth_and_canonical_block() {
        let (mut rpc, tx) = fixture();
        rpc.submitted_status = "committed";
        rpc.tip = header_at(6, [6; 32]);
        let engine = CkbEvidenceEngine::new(rpc);
        let context = engine.resolve_context(&tx, 1).unwrap();
        let verified = engine.validate_and_verify(&tx, context, 10_000).unwrap();
        let accepted = engine.submit_and_observe(&tx, verified, 1, Duration::ZERO).unwrap();
        let committed = engine.observe_committed(&tx, accepted, 1, Duration::ZERO).unwrap();
        assert_eq!(committed.stage, ProjectionStage::Committed);
        assert_eq!(committed.committed.as_ref().unwrap().proof_verified_tx_hashes, vec![committed.raw_tx_hash]);

        let finalized = engine.wait_for_finality(&tx, committed.clone(), 6, 1, Duration::ZERO).unwrap();
        assert_eq!(finalized.stage, ProjectionStage::Finalized);
        assert_eq!(finalized.finalized.as_ref().unwrap().confirmations, 6);
        verify_projection(&tx, &finalized).unwrap();

        let mut mutated_proof = committed.clone();
        mutated_proof.committed.as_mut().unwrap().proof_verified_tx_hashes[0][0] ^= 1;
        assert!(verify_projection(&tx, &mutated_proof).is_err());

        let mut mutated_depth = finalized;
        mutated_depth.finalized.as_mut().unwrap().confirmations += 1;
        assert!(verify_projection(&tx, &mutated_depth).is_err());

        let (mut reorg_rpc, _) = fixture();
        reorg_rpc.submitted_status = "committed";
        reorg_rpc.tip = header_at(6, [6; 32]);
        reorg_rpc.canonical_block = header_at(1, [1; 32]);
        let error = CkbEvidenceEngine::new(reorg_rpc).wait_for_finality(&tx, committed, 6, 1, Duration::ZERO).unwrap_err();
        assert!(matches!(error, CkbAdapterError::ReorgDetected));
    }
}
