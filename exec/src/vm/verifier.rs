// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Cell transaction script verifier
// Reference: ckb/script/src/verify.rs

use super::error::{ScriptError, ScriptResult, VMError};
use super::machine::ScriptVersion;
use super::scheduler::{FullSuspendedState, ProgramPiece, ProgramPlace, ProgramResolver, RunMode, VmScheduler};
use super::{VmSemantics, MAX_SCRIPT_SIZE, MAX_VM_MEMORY};
use crate::celltx::{CellOutput, CellTx, Script};
use crate::serialization::molecule_compat::{
    ckb_raw_transaction_hash_molecule, ckb_script_hash_molecule, deserialize_resolved_cell_molecule,
    deserialize_resolved_header_molecule, serialize_resolved_cell_molecule, serialize_resolved_header_molecule,
    serialize_transaction_molecule, CkbHeader,
};
use crate::serialization::{split_vm_abi_trailer, VmAbiError, VmAbiFormat, VmAbiNegotiator, VmSerializable};
use rayon::prelude::*;
use std::{collections::HashSet, sync::Arc};

/// Script group type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptGroupType {
    /// Lock script (who can spend)
    Lock,
    /// Type script (state transition rules)
    Type,
}

/// Transaction-level resumable verification state.
#[derive(Clone)]
pub struct TransactionState {
    /// Current script-group index being verified.
    pub current: usize,
    /// Optional suspended scheduler state for the current group.
    pub state: Option<FullSuspendedState>,
    /// Total cycles completed before the current group fully finishes.
    pub current_cycles: u64,
    /// The total transaction-level VM budget reached by the current suspended step.
    pub limit_cycles: u64,
}

impl TransactionState {
    /// Create a resumable transaction verification state.
    pub fn new(state: Option<FullSuspendedState>, current: usize, current_cycles: u64, limit_cycles: u64) -> Self {
        Self { current, state, current_cycles, limit_cycles }
    }

    /// Return the next cycle budget from an incremental step size and a maximum bound.
    pub fn next_limit_cycles(&self, step_cycles: u64, max_cycles: u64) -> (u64, bool) {
        let next_limit = self.limit_cycles.saturating_add(step_cycles).max(self.current_cycles);
        if next_limit < max_cycles {
            (next_limit, false)
        } else {
            (max_cycles, true)
        }
    }
}

impl std::fmt::Debug for TransactionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionState")
            .field("current", &self.current)
            .field("current_cycles", &self.current_cycles)
            .field("limit_cycles", &self.limit_cycles)
            .finish()
    }
}

/// Result of resumable transaction verification.
#[derive(Debug)]
pub enum VerifyResult {
    /// Verification completed and returns total consumed cycles.
    Completed(u64),
    /// Verification suspended and returns resumable transaction state.
    Suspended(TransactionState),
}

enum GroupRunResult {
    Completed(u64),
    Suspended(FullSuspendedState),
}

type SyscallFactory = dyn Fn(u64, &super::scheduler::VmRuntime) -> ScriptResult<Vec<super::scheduler::BoxedSyscall>> + Send + Sync;

struct GroupRuntime {
    script_code: Vec<u8>,
    program_resolver: ProgramResolver,
    syscall_factory: Arc<SyscallFactory>,
}

/// Script group: cells sharing the same script
#[derive(Debug, Clone)]
pub struct ScriptGroup {
    /// The script
    pub script: Script,
    /// Group type
    pub group_type: ScriptGroupType,
    /// Input indices referencing this script
    pub input_indices: Vec<usize>,
    /// Output indices referencing this script
    pub output_indices: Vec<usize>,
}

/// Fully resolved cell contents available to the VM runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedCell {
    /// Full cell output structure.
    pub cell_output: CellOutput,
    /// Optional associated cell data.
    pub data: Option<Vec<u8>>,
}

/// Fully resolved header contents available to the VM runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedHeader {
    /// Header hash.
    pub hash: [u8; 32],
    /// Header version.
    pub version: u32,
    /// Linear parent block hash, following CKB header semantics.
    pub parent_hash: [u8; 32],
    /// Block number.
    pub number: u64,
    /// Packed epoch number-with-fraction value.
    pub epoch: u64,
    /// Transaction merkle root.
    pub transactions_root: [u8; 32],
    /// Proposal IDs hash.
    pub proposals_hash: [u8; 32],
    /// Uncles hash.
    pub uncles_hash: [u8; 32],
    /// DAO field.
    pub dao: [u8; 32],
    /// Execution-related state commitment.
    pub cell_commitment: [u8; 32],
    /// Cell state root.
    pub cell_root: [u8; 32],
    /// Data-availability segment root.
    pub segment_root: [u8; 32],
    /// Timestamp in milliseconds.
    pub timestamp: u64,
    /// Compact target, matching CKB raw header semantics.
    pub compact_target: u32,
    /// CKB-compatible nonce width.
    pub nonce: u128,
}

impl VmSerializable for ResolvedHeader {
    /// Public trait ABI version: Molecule v1.
    fn abi_version() -> u16 {
        VmAbiNegotiator::ABI_VERSION_MOLECULE_V1
    }

    /// Serialize to public VM-visible bytes using Molecule.
    fn to_vm_bytes(&self) -> Vec<u8> {
        serialize_resolved_header_molecule(self).expect("Molecule serialization should not fail for ResolvedHeader")
    }

    /// Deserialize from public VM-visible bytes using Molecule.
    fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError> {
        deserialize_resolved_header_molecule(bytes).map_err(|e| VmAbiError::DeserializationFailed(e.to_string()))
    }
}

impl VmSerializable for ResolvedCell {
    /// Public trait ABI version: Molecule v1.
    fn abi_version() -> u16 {
        VmAbiNegotiator::ABI_VERSION_MOLECULE_V1
    }

    /// Serialize to public VM-visible bytes using Molecule.
    fn to_vm_bytes(&self) -> Vec<u8> {
        serialize_resolved_cell_molecule(self).expect("Molecule serialization should not fail for ResolvedCell")
    }

    /// Deserialize from public VM-visible bytes using Molecule.
    fn from_vm_bytes(bytes: &[u8]) -> Result<Self, VmAbiError> {
        deserialize_resolved_cell_molecule(bytes).map_err(|e| VmAbiError::DeserializationFailed(e.to_string()))
    }
}

fn script_hash_for_semantics(script: &Script, semantics: VmSemantics) -> ScriptResult<[u8; 32]> {
    match semantics {
        VmSemantics::MyelinExtended => Ok(script.hash()),
        VmSemantics::CkbStrict => ckb_script_hash_molecule(script)
            .map_err(|err| ScriptError::VM(VMError::InvalidData(format!("failed to hash CKB Molecule Script: {err}")))),
    }
}

fn transaction_hash_and_data_for_semantics(tx: &CellTx, semantics: VmSemantics) -> ScriptResult<([u8; 32], Vec<u8>)> {
    match semantics {
        VmSemantics::MyelinExtended => {
            let tx_hash = crate::celltx::compute_txid(tx);
            let tx_data = serialize_transaction_molecule(tx).map_err(|err| {
                ScriptError::VM(VMError::InvalidData(format!("failed to serialize Molecule tx for LOAD_TRANSACTION syscall: {err}")))
            })?;
            Ok((tx_hash, tx_data))
        }
        VmSemantics::CkbStrict => {
            let tx_hash = ckb_raw_transaction_hash_molecule(tx)
                .map_err(|err| ScriptError::VM(VMError::InvalidData(format!("failed to hash CKB RawTransaction: {err}"))))?;
            let tx_data = serialize_transaction_molecule(tx)
                .map_err(|err| ScriptError::VM(VMError::InvalidData(format!("failed to serialize CKB Transaction: {err}"))))?;
            Ok((tx_hash, tx_data))
        }
    }
}

/// Cell data provider trait (for loading cell data)
pub trait CellDataProvider: Send + Sync + 'static {
    /// Load cell data by script code hash
    fn load_cell_data(&self, code_hash: &[u8; 32]) -> Option<Vec<u8>>;

    /// Load a fully resolved cell by outpoint.
    fn load_cell_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> Option<ResolvedCell>;

    /// Load a fully resolved header by hash.
    fn load_header(&self, hash: &[u8; 32]) -> Option<ResolvedHeader>;

    /// Load the header associated with a resolved input or dep outpoint.
    fn load_header_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> Option<ResolvedHeader>;

    /// Load a CKB packed header by hash.
    fn load_ckb_header(&self, _hash: &[u8; 32]) -> Option<CkbHeader> {
        None
    }

    /// Load the CKB block header hash associated with a resolved input or dep outpoint.
    fn load_ckb_header_hash_by_outpoint(&self, _tx_hash: &[u8; 32], _index: u32) -> Option<[u8; 32]> {
        None
    }

    /// Load a fully resolved cell associated with a header hash.
    ///
    /// This supports `Source::HeaderDep` in `LOAD_CELL` / `LOAD_CELL_DATA`
    /// syscalls by mapping a block header hash to a representative cell
    /// (e.g. the cellbase output of that block).
    fn load_cell_by_header(&self, header_hash: &[u8; 32]) -> Option<ResolvedCell>;
}

/// Transaction script verifier
pub struct TransactionScriptVerifier<D: CellDataProvider> {
    /// The transaction to verify
    tx: Arc<CellTx>,
    /// Cell data provider
    data_provider: Arc<D>,
    /// Script version
    version: ScriptVersion,
    /// Max cycles per script
    max_cycles: u64,
    /// Max VM memory per script execution.
    max_memory: usize,
    /// Max script binary size.
    max_script_size: usize,
    /// Skip lock-script groups and verify only type-script groups.
    skip_lock_groups: bool,
    /// Skip selected lock-script groups by script hash.
    skip_lock_script_hashes: HashSet<[u8; 32]>,
    /// Runtime syscall semantics profile.
    semantics: VmSemantics,
    /// VM ABI wire format for full object load syscalls.
    abi_format: VmAbiFormat,
}

impl<D: CellDataProvider> TransactionScriptVerifier<D> {
    /// Create a new verifier.
    ///
    /// The public VM object ABI is Molecule. Artifact trailers declaring other
    /// ABI versions are rejected.
    pub fn new(tx: Arc<CellTx>, data_provider: Arc<D>) -> Self {
        Self {
            tx,
            data_provider,
            version: ScriptVersion::latest(),
            max_cycles: 10_000_000, // 10M cycles default
            max_memory: MAX_VM_MEMORY,
            max_script_size: MAX_SCRIPT_SIZE,
            skip_lock_groups: false,
            skip_lock_script_hashes: HashSet::new(),
            semantics: VmSemantics::MyelinExtended,
            abi_format: VmAbiFormat::Molecule,
        }
    }

    /// Set script version
    pub fn with_version(mut self, version: ScriptVersion) -> Self {
        self.version = version;
        self
    }

    /// Set max cycles
    pub fn with_max_cycles(mut self, max_cycles: u64) -> Self {
        self.max_cycles = max_cycles;
        self
    }

    /// Set max VM memory
    pub fn with_max_memory(mut self, max_memory: usize) -> Self {
        self.max_memory = max_memory;
        self
    }

    /// Set max script size
    pub fn with_max_script_size(mut self, max_script_size: usize) -> Self {
        self.max_script_size = max_script_size;
        self
    }

    /// Verify only type-script groups and skip lock-script groups.
    pub fn with_skip_lock_groups(mut self, skip_lock_groups: bool) -> Self {
        self.skip_lock_groups = skip_lock_groups;
        self
    }

    /// Skip lock-script groups whose script hash is included in `script_hashes`.
    pub fn with_skip_lock_script_hashes(mut self, script_hashes: HashSet<[u8; 32]>) -> Self {
        self.skip_lock_script_hashes = script_hashes;
        self
    }

    /// Select VM syscall semantics profile.
    pub fn with_semantics(mut self, semantics: VmSemantics) -> Self {
        self.semantics = semantics;
        self
    }

    /// Select the VM ABI wire format used by full object load syscalls.
    pub fn with_abi_format(mut self, abi_format: VmAbiFormat) -> Self {
        self.abi_format = abi_format;
        self
    }

    /// Select the VM ABI wire format from a negotiated artifact/runtime ABI version.
    pub fn with_abi_version(mut self, abi_version: u16) -> ScriptResult<Self> {
        self.abi_format = VmAbiFormat::from_abi_version(abi_version)
            .map_err(|err| VMError::InvalidData(format!("invalid VM ABI version: {}", err)))?;
        Ok(self)
    }

    /// Extract script groups from transaction
    pub fn extract_script_groups(&self) -> ScriptResult<Vec<ScriptGroup>> {
        use std::collections::BTreeMap;

        let mut lock_groups: BTreeMap<[u8; 32], ScriptGroup> = BTreeMap::new();
        let mut type_groups: BTreeMap<[u8; 32], ScriptGroup> = BTreeMap::new();

        // Lock scripts execute against resolved input cells.
        for (i, input) in self.tx.inputs.iter().enumerate() {
            let resolved = self
                .data_provider
                .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                .ok_or_else(|| {
                    ScriptError::VM(super::error::VMError::ItemMissing(format!(
                        "missing resolved input cell {:02x?}:{}",
                        input.previous_output.tx_hash, input.previous_output.index
                    )))
                })?;
            if !self.skip_lock_groups {
                let lock_hash = script_hash_for_semantics(&resolved.cell_output.lock, self.semantics)?;
                if self.skip_lock_script_hashes.contains(&lock_hash) {
                    continue;
                }

                lock_groups
                    .entry(lock_hash)
                    .or_insert_with(|| ScriptGroup {
                        script: resolved.cell_output.lock.clone(),
                        group_type: ScriptGroupType::Lock,
                        input_indices: vec![],
                        output_indices: vec![],
                    })
                    .input_indices
                    .push(i);
            }

            if let Some(ref type_script) = resolved.cell_output.type_ {
                let type_hash = script_hash_for_semantics(type_script, self.semantics)?;

                type_groups
                    .entry(type_hash)
                    .or_insert_with(|| ScriptGroup {
                        script: type_script.clone(),
                        group_type: ScriptGroupType::Type,
                        input_indices: vec![],
                        output_indices: vec![],
                    })
                    .input_indices
                    .push(i);
            }
        }

        // Type scripts execute over both consumed and created cells.
        for (i, output) in self.tx.outputs.iter().enumerate() {
            if let Some(ref type_script) = output.type_ {
                let type_hash = script_hash_for_semantics(type_script, self.semantics)?;

                type_groups
                    .entry(type_hash)
                    .or_insert_with(|| ScriptGroup {
                        script: type_script.clone(),
                        group_type: ScriptGroupType::Type,
                        input_indices: vec![],
                        output_indices: vec![],
                    })
                    .output_indices
                    .push(i);
            }
        }

        Ok(lock_groups.into_values().chain(type_groups.into_values()).collect())
    }

    /// Verify all scripts in the transaction
    pub fn verify(&self) -> ScriptResult<()> {
        self.verify_with_cycles().map(|_| ())
    }

    /// Verify all scripts in the transaction and return the total consumed cycles.
    pub fn verify_with_cycles(&self) -> ScriptResult<u64> {
        let script_groups = self.extract_script_groups()?;
        let group_results: Vec<ScriptResult<u64>> =
            script_groups.par_iter().map(|group| self.verify_script_group(group, self.max_cycles)).collect();

        // Keep error selection deterministic by folding results in the stable
        // script-group order produced by extract_script_groups.
        group_results
            .into_iter()
            .try_fold(0u64, |total_cycles, group_cycles| group_cycles.map(|cycles| total_cycles.saturating_add(cycles)))
    }

    /// Verify all scripts with a resumable transaction-level state machine.
    pub fn resumable_verify(&self, limit_cycles: u64) -> ScriptResult<VerifyResult> {
        let script_groups = self.extract_script_groups()?;
        let mut total_cycles = 0u64;
        let mut current_consumed_cycles = 0u64;

        for (idx, group) in script_groups.iter().enumerate() {
            let Some(remain_cycles) = limit_cycles.checked_sub(current_consumed_cycles) else {
                return Ok(VerifyResult::Suspended(TransactionState::new(None, idx, total_cycles, limit_cycles)));
            };

            match self.verify_script_group_chunk(group, remain_cycles, None)? {
                GroupRunResult::Completed(group_cycles) => {
                    current_consumed_cycles = current_consumed_cycles.saturating_add(group_cycles);
                    total_cycles = total_cycles.saturating_add(group_cycles);
                    if idx + 1 < script_groups.len() && current_consumed_cycles >= limit_cycles {
                        return Ok(VerifyResult::Suspended(TransactionState::new(None, idx + 1, total_cycles, limit_cycles)));
                    }
                }
                GroupRunResult::Suspended(state) => {
                    return Ok(VerifyResult::Suspended(TransactionState::new(Some(state), idx, total_cycles, limit_cycles)));
                }
            }
        }

        Ok(VerifyResult::Completed(total_cycles))
    }

    /// Resume transaction verification from a previous suspended state.
    pub fn resume_from_state(&self, state: &TransactionState, limit_cycles: u64) -> ScriptResult<VerifyResult> {
        let script_groups = self.extract_script_groups()?;
        let current_group = script_groups.get(state.current).ok_or_else(|| {
            ScriptError::VM(super::error::VMError::ExecutionError(format!("snapshot group missing {}", state.current)))
        })?;

        let mut total_cycles = state.current_cycles;
        let Some(current_group_limit) = limit_cycles.checked_sub(total_cycles) else {
            return Ok(VerifyResult::Suspended(TransactionState::new(state.state.clone(), state.current, total_cycles, limit_cycles)));
        };

        match self.verify_script_group_chunk(current_group, current_group_limit, state.state.as_ref())? {
            GroupRunResult::Completed(group_cycles) => {
                total_cycles = total_cycles.saturating_add(group_cycles);
            }
            GroupRunResult::Suspended(next_state) => {
                return Ok(VerifyResult::Suspended(TransactionState::new(
                    Some(next_state),
                    state.current,
                    total_cycles,
                    limit_cycles,
                )));
            }
        }

        for (idx, group) in script_groups.iter().enumerate().skip(state.current + 1) {
            let Some(remain_cycles) = limit_cycles.checked_sub(total_cycles) else {
                return Ok(VerifyResult::Suspended(TransactionState::new(None, idx, total_cycles, limit_cycles)));
            };

            match self.verify_script_group_chunk(group, remain_cycles, None)? {
                GroupRunResult::Completed(group_cycles) => {
                    total_cycles = total_cycles.saturating_add(group_cycles);
                    if idx + 1 < script_groups.len() && total_cycles >= limit_cycles {
                        return Ok(VerifyResult::Suspended(TransactionState::new(None, idx + 1, total_cycles, limit_cycles)));
                    }
                }
                GroupRunResult::Suspended(next_state) => {
                    return Ok(VerifyResult::Suspended(TransactionState::new(Some(next_state), idx, total_cycles, limit_cycles)));
                }
            }
        }

        Ok(VerifyResult::Completed(total_cycles))
    }

    /// Finish a suspended verification or return a cycles-exceeded error if it still cannot complete.
    pub fn complete(&self, state: &TransactionState, max_cycles: u64) -> ScriptResult<u64> {
        if max_cycles < state.current_cycles {
            return Err(ScriptError::VM(super::error::VMError::CyclesExceeded { limit: max_cycles, actual: state.current_cycles }));
        }

        let script_groups = self.extract_script_groups()?;
        let current_group = script_groups.get(state.current).ok_or_else(|| {
            ScriptError::VM(super::error::VMError::ExecutionError(format!("snapshot group missing {}", state.current)))
        })?;

        let mut total_cycles = state.current_cycles;

        match self.verify_script_group_chunk(current_group, max_cycles - total_cycles, state.state.as_ref())? {
            GroupRunResult::Completed(group_cycles) => {
                total_cycles = total_cycles.saturating_add(group_cycles);
            }
            GroupRunResult::Suspended(_) => {
                return Err(ScriptError::VM(super::error::VMError::CyclesExceeded {
                    limit: max_cycles,
                    actual: max_cycles.saturating_add(1),
                }));
            }
        }

        for group in script_groups.iter().skip(state.current + 1) {
            let remain_cycles = max_cycles
                .checked_sub(total_cycles)
                .ok_or(ScriptError::VM(super::error::VMError::CyclesExceeded { limit: max_cycles, actual: total_cycles }))?;
            match self.verify_script_group_chunk(group, remain_cycles, None)? {
                GroupRunResult::Completed(group_cycles) => {
                    total_cycles = total_cycles.saturating_add(group_cycles);
                }
                GroupRunResult::Suspended(_) => {
                    return Err(ScriptError::VM(super::error::VMError::CyclesExceeded {
                        limit: max_cycles,
                        actual: max_cycles.saturating_add(1),
                    }));
                }
            }
        }

        Ok(total_cycles)
    }

    /// Verify a single script group
    fn verify_script_group(&self, group: &ScriptGroup, max_cycles: u64) -> ScriptResult<u64> {
        let mut scheduler = self.create_scheduler(group)?;
        let result = scheduler.run_with_mode(RunMode::LimitCycles(max_cycles))?;
        if result.exit_code != 0 {
            return Err(ScriptError::VM(super::error::VMError::NonZeroExitCode(result.exit_code)));
        }

        log::debug!("Script group {:?} verified successfully, cycles: {}", group.group_type, result.consumed_cycles);

        Ok(result.consumed_cycles)
    }

    fn verify_script_group_chunk(
        &self,
        group: &ScriptGroup,
        max_cycles: u64,
        state: Option<&FullSuspendedState>,
    ) -> ScriptResult<GroupRunResult> {
        let mut scheduler = match state {
            Some(state) => self.resume_scheduler(group, state)?,
            None => self.create_scheduler(group)?,
        };

        match scheduler.run_with_mode(RunMode::LimitCycles(max_cycles)) {
            Ok(result) => {
                if result.exit_code != 0 {
                    return Err(ScriptError::VM(super::error::VMError::NonZeroExitCode(result.exit_code)));
                }
                Ok(GroupRunResult::Completed(result.consumed_cycles))
            }
            Err(ScriptError::VM(super::error::VMError::CyclesExceeded { .. } | super::error::VMError::Paused)) => {
                Ok(GroupRunResult::Suspended(scheduler.suspend()?))
            }
            Err(other) => Err(other),
        }
    }

    fn create_scheduler(&self, group: &ScriptGroup) -> ScriptResult<VmScheduler> {
        let prepared = self.prepare_group_runtime(group)?;
        Ok(VmScheduler::new(
            self.version,
            self.max_cycles,
            self.max_memory,
            self.max_script_size,
            prepared.script_code,
            vec![group.script.args.clone()],
            prepared.syscall_factory,
            prepared.program_resolver,
        ))
    }

    fn resume_scheduler(&self, group: &ScriptGroup, state: &FullSuspendedState) -> ScriptResult<VmScheduler> {
        let prepared = self.prepare_group_runtime(group)?;

        VmScheduler::resume(
            self.version,
            self.max_cycles,
            self.max_memory,
            self.max_script_size,
            prepared.script_code,
            vec![group.script.args.clone()],
            prepared.syscall_factory,
            prepared.program_resolver,
            state.clone(),
        )
    }

    fn prepare_group_runtime(&self, group: &ScriptGroup) -> ScriptResult<GroupRuntime> {
        if group.script.hash_type != 0 {
            return Err(ScriptError::InvalidHashType(group.script.hash_type));
        }

        let raw_script_code =
            self.data_provider.load_cell_data(&group.script.code_hash).ok_or(ScriptError::ScriptNotFound(group.script.code_hash))?;
        let (script_code, artifact_abi_format) = split_vm_abi_trailer(&raw_script_code)
            .map_err(|err| ScriptError::VM(VMError::InvalidData(format!("invalid VM ABI artifact trailer: {}", err))))?;
        let script_code = script_code.to_vec();
        let effective_abi_format = artifact_abi_format.unwrap_or(self.abi_format);

        use super::syscalls::load_signature_hash::standard_signing_input_from_resolved_cell;
        use super::syscalls::*;

        let signing_inputs = self
            .tx
            .inputs
            .iter()
            .map(|input| {
                self.data_provider
                    .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                    .map(|resolved| standard_signing_input_from_resolved_cell(&resolved))
                    .ok_or_else(|| {
                        ScriptError::VM(super::error::VMError::ItemMissing(format!(
                            "missing resolved input cell {:02x?}:{}",
                            input.previous_output.tx_hash, input.previous_output.index
                        )))
                    })
            })
            .collect::<ScriptResult<Vec<_>>>()?;
        let (tx_hash, tx_data) = transaction_hash_and_data_for_semantics(&self.tx, self.semantics)?;

        let tx = Arc::clone(&self.tx);
        let provider = Arc::clone(&self.data_provider);
        let script = Arc::new(group.script.clone());
        let group_input_indices = group.input_indices.clone();
        let group_output_indices = group.output_indices.clone();

        let program_resolver: ProgramResolver = Arc::new({
            let tx = Arc::clone(&tx);
            let provider = Arc::clone(&provider);
            let group_input_indices = group_input_indices.clone();
            let group_output_indices = group_output_indices.clone();
            move |piece: &ProgramPiece| -> Result<Vec<u8>, u8> {
                match piece.place {
                    ProgramPlace::CellData => match piece.source {
                        Source::Input => {
                            let input = tx.inputs.get(piece.index).ok_or(INDEX_OUT_OF_BOUND)?;
                            provider
                                .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                                .map(|cell| cell.data.unwrap_or_default())
                                .ok_or(ITEM_MISSING)
                        }
                        Source::Output => {
                            tx.outputs.get(piece.index).ok_or(INDEX_OUT_OF_BOUND)?;
                            tx.outputs_data.get(piece.index).cloned().ok_or(ITEM_MISSING)
                        }
                        Source::CellDep => {
                            let dep = tx.cell_deps.get(piece.index).ok_or(INDEX_OUT_OF_BOUND)?;
                            provider
                                .load_cell_by_outpoint(&dep.out_point.tx_hash, dep.out_point.index)
                                .map(|cell| cell.data.unwrap_or_default())
                                .ok_or(ITEM_MISSING)
                        }
                        Source::GroupInput => {
                            let input_index = group_input_indices.get(piece.index).copied().ok_or(INDEX_OUT_OF_BOUND)?;
                            let input = tx.inputs.get(input_index).ok_or(INDEX_OUT_OF_BOUND)?;
                            provider
                                .load_cell_by_outpoint(&input.previous_output.tx_hash, input.previous_output.index)
                                .map(|cell| cell.data.unwrap_or_default())
                                .ok_or(ITEM_MISSING)
                        }
                        Source::GroupOutput => {
                            let output_index = group_output_indices.get(piece.index).copied().ok_or(INDEX_OUT_OF_BOUND)?;
                            tx.outputs.get(output_index).ok_or(INDEX_OUT_OF_BOUND)?;
                            tx.outputs_data.get(output_index).cloned().ok_or(ITEM_MISSING)
                        }
                        Source::GroupCellDep | Source::GroupHeaderDep => Err(INDEX_OUT_OF_BOUND),
                        Source::HeaderDep => Err(INDEX_OUT_OF_BOUND),
                    },
                    ProgramPlace::Witness => {
                        let witness_index = match piece.source {
                            Source::Input => piece.index,
                            Source::Output => tx.inputs.len().checked_add(piece.index).ok_or(INDEX_OUT_OF_BOUND)?,
                            Source::CellDep => tx
                                .inputs
                                .len()
                                .checked_add(tx.outputs.len())
                                .and_then(|base| base.checked_add(piece.index))
                                .ok_or(INDEX_OUT_OF_BOUND)?,
                            Source::GroupInput => group_input_indices.get(piece.index).copied().ok_or(INDEX_OUT_OF_BOUND)?,
                            Source::GroupOutput => {
                                let output_index = group_output_indices.get(piece.index).copied().ok_or(INDEX_OUT_OF_BOUND)?;
                                tx.inputs.len().checked_add(output_index).ok_or(INDEX_OUT_OF_BOUND)?
                            }
                            Source::GroupCellDep | Source::GroupHeaderDep => return Err(INDEX_OUT_OF_BOUND),
                            Source::HeaderDep => return Err(INDEX_OUT_OF_BOUND),
                        };
                        tx.witnesses.get(witness_index).cloned().ok_or(INDEX_OUT_OF_BOUND)
                    }
                }
            }
        });

        let syscall_factory = Arc::new({
            let tx = Arc::clone(&tx);
            let provider = Arc::clone(&provider);
            let script = Arc::clone(&script);
            let signing_inputs = signing_inputs.clone();
            let group_input_indices = group_input_indices.clone();
            let group_output_indices = group_output_indices.clone();
            let tx_data = tx_data.clone();
            #[cfg(feature = "vm-ipc")]
            let program_resolver = Arc::clone(&program_resolver);
            let semantics = self.semantics;
            let abi_format = effective_abi_format;
            move |_vm_id, runtime: &super::scheduler::VmRuntime| -> ScriptResult<Vec<super::scheduler::BoxedSyscall>> {
                let mut syscalls: Vec<super::scheduler::BoxedSyscall> = vec![
                    Box::new(LoadTx::new(tx_hash, tx_data.clone())),
                    Box::new(
                        LoadCell::new(
                            Arc::clone(&tx),
                            Arc::clone(&provider),
                            group_input_indices.clone(),
                            group_output_indices.clone(),
                        )
                        .with_semantics(semantics)
                        .with_abi_format(abi_format),
                    ),
                    Box::new(
                        LoadCellData::new(
                            Arc::clone(&tx),
                            Arc::clone(&provider),
                            group_input_indices.clone(),
                            group_output_indices.clone(),
                        )
                        .with_semantics(semantics),
                    ),
                    Box::new(
                        LoadInput::new(Arc::clone(&tx), group_input_indices.clone())
                            .with_abi_format(abi_format)
                            .with_semantics(semantics),
                    ),
                    Box::new(
                        LoadWitness::new(Arc::clone(&tx), group_input_indices.clone(), group_output_indices.clone())
                            .with_semantics(semantics),
                    ),
                    Box::new(LoadScript::new(Arc::clone(&script)).with_abi_format(abi_format).with_semantics(semantics)),
                ];
                if semantics.allow_myelin_extension_syscalls() {
                    syscalls.push(Box::new(LoadSignatureHash::new(
                        Arc::clone(&tx),
                        signing_inputs.clone(),
                        group_input_indices.clone(),
                    )));
                }
                syscalls.push(Box::new(
                    LoadHeader::new(Arc::clone(&tx), Arc::clone(&provider), group_input_indices.clone(), group_output_indices.clone())
                        .with_abi_format(abi_format)
                        .with_semantics(semantics),
                ));
                syscalls.push(Box::new(VMVersion::new()));
                syscalls.push(Box::new(CurrentCycles::with_base_cycles(Arc::clone(&runtime.base_cycles))));
                syscalls.push(Box::new(Debugger::new(script.code_hash)));
                syscalls.push(Box::new(
                    Exec::new(Arc::clone(&tx), Arc::clone(&provider), group_input_indices.clone(), group_output_indices.clone())
                        .with_snapshot_tracking(Arc::clone(&runtime.snapshot2_context), runtime.data_source.clone())
                        .with_semantics(semantics),
                ));
                #[cfg(feature = "vm-ipc")]
                {
                    syscalls.push(Box::new(ProcessId::new(_vm_id)));
                    syscalls
                        .push(Box::new(Spawn::with_runtime(_vm_id, runtime, Arc::clone(&program_resolver)).with_semantics(semantics)));
                    syscalls.push(Box::new(Wait::with_runtime(_vm_id, runtime)));
                    syscalls.push(Box::new(Pipe::with_runtime(_vm_id, runtime)));
                    syscalls.push(Box::new(Read::with_runtime(_vm_id, runtime)));
                    syscalls.push(Box::new(Write::with_runtime(_vm_id, runtime)));
                    syscalls.push(Box::new(InheritedFd::with_runtime(_vm_id, runtime)));
                    syscalls.push(Box::new(Close::with_runtime(_vm_id, runtime)));
                }
                if semantics.allow_myelin_extension_syscalls() {
                    syscalls.push(Box::new(Blake3Hash::new()));
                    syscalls.push(Box::new(Secp256k1Verify::new()));
                }
                Ok(syscalls)
            }
        });

        Ok(GroupRuntime { script_code, program_resolver, syscall_factory })
    }
}

/// Simple in-memory cell data provider (for testing)
pub struct SimpleDataProvider {
    scripts: std::collections::HashMap<[u8; 32], Vec<u8>>,
    cells: std::collections::HashMap<([u8; 32], u32), ResolvedCell>,
    headers: std::collections::HashMap<[u8; 32], ResolvedHeader>,
    ckb_headers: std::collections::HashMap<[u8; 32], CkbHeader>,
    cell_headers: std::collections::HashMap<([u8; 32], u32), [u8; 32]>,
    header_cells: std::collections::HashMap<[u8; 32], ResolvedCell>,
}

impl Default for SimpleDataProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl SimpleDataProvider {
    pub fn new() -> Self {
        Self {
            scripts: std::collections::HashMap::new(),
            cells: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            ckb_headers: std::collections::HashMap::new(),
            cell_headers: std::collections::HashMap::new(),
            header_cells: std::collections::HashMap::new(),
        }
    }

    pub fn add_script(&mut self, code_hash: [u8; 32], code: Vec<u8>) {
        self.scripts.insert(code_hash, code);
    }

    pub fn add_cell(&mut self, tx_hash: [u8; 32], index: u32, cell: ResolvedCell) {
        self.cells.insert((tx_hash, index), cell);
    }

    pub fn add_cell_with_header(&mut self, tx_hash: [u8; 32], index: u32, cell: ResolvedCell, header_hash: [u8; 32]) {
        self.cells.insert((tx_hash, index), cell);
        self.cell_headers.insert((tx_hash, index), header_hash);
    }

    pub fn add_header(&mut self, hash: [u8; 32], header: ResolvedHeader) {
        self.headers.insert(hash, header);
    }

    pub fn add_ckb_header(&mut self, hash: [u8; 32], header: CkbHeader) {
        self.ckb_headers.insert(hash, header);
    }

    /// Register a cell associated with a header hash (e.g. cellbase output).
    pub fn add_cell_by_header(&mut self, header_hash: [u8; 32], cell: ResolvedCell) {
        self.header_cells.insert(header_hash, cell);
    }
}

impl CellDataProvider for SimpleDataProvider {
    fn load_cell_data(&self, code_hash: &[u8; 32]) -> Option<Vec<u8>> {
        self.scripts.get(code_hash).cloned()
    }

    fn load_cell_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> Option<ResolvedCell> {
        self.cells.get(&(*tx_hash, index)).cloned()
    }

    fn load_header(&self, hash: &[u8; 32]) -> Option<ResolvedHeader> {
        self.headers.get(hash).cloned()
    }

    fn load_header_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> Option<ResolvedHeader> {
        let header_hash = self.cell_headers.get(&(*tx_hash, index))?;
        self.headers.get(header_hash).cloned()
    }

    fn load_ckb_header(&self, hash: &[u8; 32]) -> Option<CkbHeader> {
        self.ckb_headers.get(hash).cloned()
    }

    fn load_ckb_header_hash_by_outpoint(&self, tx_hash: &[u8; 32], index: u32) -> Option<[u8; 32]> {
        self.cell_headers.get(&(*tx_hash, index)).copied()
    }

    fn load_cell_by_header(&self, header_hash: &[u8; 32]) -> Option<ResolvedCell> {
        self.header_cells.get(header_hash).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::{CellInput, OutPoint};
    use crate::scripts::{always_success_code_hash, ALWAYS_SUCCESS_SCRIPT};
    use crate::serialization::molecule_compat::{
        ckb_raw_transaction_hash_molecule, ckb_script_hash_molecule, serialize_transaction_molecule,
    };
    use crate::vm::VmSemantics;

    fn always_success_verifier() -> TransactionScriptVerifier<SimpleDataProvider> {
        let mut provider = SimpleDataProvider::new();
        let code_hash = always_success_code_hash();
        provider.add_script(code_hash, ALWAYS_SUCCESS_SCRIPT.to_vec());
        let input_out_point = OutPoint::new([0x44; 32], 0);
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new(code_hash, 0, vec![]), type_: None },
                data: Some(vec![]),
            },
        );

        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![CellInput::new(input_out_point, 0)],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![CellOutput { capacity: 1000, lock: Script::new(code_hash, 0, vec![]), type_: None }],
            outputs_data: vec![vec![]],
            witnesses: vec![],
        });

        TransactionScriptVerifier::new(tx, Arc::new(provider)).with_version(ScriptVersion::V2).with_max_cycles(10_000)
    }

    #[test]
    fn test_verifier_creation() {
        let tx = Arc::new(CellTx {
            version: 0xC001,
            inputs: vec![],
            cell_deps: vec![],
            header_deps: vec![],
            outputs: vec![],
            outputs_data: vec![],
            witnesses: vec![],
        });

        let provider = Arc::new(SimpleDataProvider::new());
        let verifier = TransactionScriptVerifier::new(tx, provider);

        assert_eq!(verifier.version, ScriptVersion::latest());
        assert_eq!(verifier.max_cycles, 10_000_000);
    }

    #[test]
    fn test_verifier_selects_abi_from_artifact_version() {
        let tx = Arc::new(CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap());
        let provider = Arc::new(SimpleDataProvider::new());

        let verifier = TransactionScriptVerifier::new(tx, provider)
            .with_abi_version(VmAbiNegotiator::ABI_VERSION_MOLECULE_V1)
            .expect("molecule ABI should be supported");

        assert_eq!(verifier.abi_format, VmAbiFormat::Molecule);
    }

    #[test]
    fn test_transaction_hash_and_data_switch_to_ckb_molecule_under_ckb_strict_semantics() {
        let tx = CellTx::new(
            vec![CellInput::new(OutPoint::new([0x11; 32], 0), 7)],
            vec![],
            vec![CellOutput { capacity: 1000, lock: Script::new([0x22; 32], 0, vec![0xAA]), type_: None }],
            vec![vec![0xBB, 0xCC]],
            vec![vec![0xDD]],
        )
        .unwrap();

        let (myelin_hash, myelin_data) = transaction_hash_and_data_for_semantics(&tx, VmSemantics::MyelinExtended).unwrap();
        assert_eq!(myelin_hash, crate::celltx::compute_txid(&tx));
        assert_eq!(myelin_data, serialize_transaction_molecule(&tx).unwrap());

        let (ckb_hash, ckb_data) = transaction_hash_and_data_for_semantics(&tx, VmSemantics::CkbStrict).unwrap();
        assert_eq!(ckb_hash, ckb_raw_transaction_hash_molecule(&tx).unwrap());
        assert_eq!(ckb_data, serialize_transaction_molecule(&tx).unwrap());
        assert_ne!(ckb_hash, myelin_hash);
        assert_eq!(ckb_data, myelin_data);
    }

    #[test]
    fn test_resolved_vm_serializable_uses_molecule_public_abi() {
        let header = ResolvedHeader {
            hash: [0x11; 32],
            version: 7,
            parent_hash: [0xAA; 32],
            transactions_root: [0x22; 32],
            proposals_hash: [0x33; 32],
            cell_commitment: [0x44; 32],
            cell_root: [0x55; 32],
            segment_root: [0x66; 32],
            timestamp: 0x0102_0304_0506_0708,
            compact_target: 0x1d00_ffff,
            nonce: 0x8877_6655_4433_2211,
            number: 0x1122_3344_5566_7788,
            dao: [0x77; 32],
            epoch: 0x99AA_BBCC_DDEE_FF00,
            uncles_hash: [0x88; 32],
        };
        assert_eq!(ResolvedHeader::abi_version(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);
        let header_bytes = header.to_vm_bytes();
        assert_eq!(header_bytes, serialize_resolved_header_molecule(&header).expect("header molecule bytes"));
        assert_eq!(ResolvedHeader::from_vm_bytes(&header_bytes).expect("header roundtrip"), header);

        let cell = ResolvedCell {
            cell_output: CellOutput {
                capacity: 42,
                lock: Script::new([0x99; 32], 0, vec![1, 2, 3]),
                type_: Some(Script::new([0xAA; 32], 1, vec![4, 5])),
            },
            data: Some(vec![9, 8, 7]),
        };
        assert_eq!(ResolvedCell::abi_version(), VmAbiNegotiator::ABI_VERSION_MOLECULE_V1);
        let cell_bytes = cell.to_vm_bytes();
        assert_eq!(cell_bytes, serialize_resolved_cell_molecule(&cell).expect("cell molecule bytes"));
        assert_eq!(ResolvedCell::from_vm_bytes(&cell_bytes).expect("cell roundtrip"), cell);
    }

    #[test]
    fn test_verifier_rejects_unknown_artifact_abi_version() {
        let tx = Arc::new(CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap());
        let provider = Arc::new(SimpleDataProvider::new());

        let err = match TransactionScriptVerifier::new(tx, provider).with_abi_version(0x9001) {
            Ok(_) => panic!("unknown ABI version should be rejected"),
            Err(err) => err,
        };

        assert!(matches!(err, ScriptError::VM(VMError::InvalidData(message)) if message.contains("0x9001")));
    }

    #[test]
    fn test_extract_script_groups_uses_resolved_input_locks() {
        let input_lock = Script::new([1u8; 32], 0, vec![0xAA]);
        let output_lock = Script::new([2u8; 32], 0, vec![0xBB]);
        let input_out_point = OutPoint::new([9u8; 32], 0);
        let tx = Arc::new(
            CellTx::new(
                vec![CellInput::new(input_out_point, 0)],
                vec![],
                vec![CellOutput { capacity: 1000, lock: output_lock.clone(), type_: None }],
                vec![vec![]],
                vec![],
            )
            .unwrap(),
        );

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell { cell_output: CellOutput { capacity: 1000, lock: input_lock.clone(), type_: None }, data: Some(vec![]) },
        );

        let groups = TransactionScriptVerifier::new(tx, Arc::new(provider)).extract_script_groups().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_type, ScriptGroupType::Lock);
        assert_eq!(groups[0].script, input_lock);
        assert_eq!(groups[0].input_indices, vec![0]);
        assert!(groups[0].output_indices.is_empty());
    }

    #[test]
    fn test_verify_rejects_unsupported_hash_type() {
        let tx = Arc::new(CellTx::new(vec![CellInput::new(OutPoint::new([7u8; 32], 0), 0)], vec![], vec![], vec![], vec![]).unwrap());
        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            [7u8; 32],
            0,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: Script::new([3u8; 32], 1, vec![]), type_: None },
                data: Some(vec![]),
            },
        );

        let err = TransactionScriptVerifier::new(tx, Arc::new(provider)).verify().unwrap_err();
        assert!(matches!(err, ScriptError::InvalidHashType(1)));
    }

    #[test]
    fn test_extract_script_groups_merges_type_inputs_and_outputs() {
        let input_lock = Script::new([1u8; 32], 0, vec![0xAA]);
        let shared_type = Script::new([4u8; 32], 0, vec![0xCC]);
        let input_out_point = OutPoint::new([9u8; 32], 0);
        let tx = Arc::new(
            CellTx::new(
                vec![CellInput::new(input_out_point, 0)],
                vec![],
                vec![CellOutput { capacity: 1000, lock: Script::new([2u8; 32], 0, vec![0xBB]), type_: Some(shared_type.clone()) }],
                vec![vec![]],
                vec![],
            )
            .unwrap(),
        );

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            input_out_point.tx_hash,
            input_out_point.index,
            ResolvedCell {
                cell_output: CellOutput { capacity: 1000, lock: input_lock, type_: Some(shared_type.clone()) },
                data: Some(vec![]),
            },
        );

        let groups = TransactionScriptVerifier::new(tx, Arc::new(provider)).extract_script_groups().unwrap();
        let type_group = groups.into_iter().find(|group| group.group_type == ScriptGroupType::Type).expect("type group");
        assert_eq!(type_group.script, shared_type);
        assert_eq!(type_group.input_indices, vec![0]);
        assert_eq!(type_group.output_indices, vec![0]);
    }

    #[test]
    fn test_extract_script_groups_skips_selected_lock_groups() {
        let skipped_lock = Script::new([0x11; 32], 0, vec![0xAA]);
        let retained_lock = Script::new([0x22; 32], 0, vec![0xBB]);
        let skipped_out_point = OutPoint::new([0x31; 32], 0);
        let retained_out_point = OutPoint::new([0x32; 32], 0);
        let tx = Arc::new(
            CellTx::new(
                vec![CellInput::new(skipped_out_point, 0), CellInput::new(retained_out_point, 0)],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .unwrap(),
        );

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            [0x31; 32],
            0,
            ResolvedCell { cell_output: CellOutput { capacity: 1000, lock: skipped_lock.clone(), type_: None }, data: Some(vec![]) },
        );
        provider.add_cell(
            [0x32; 32],
            0,
            ResolvedCell { cell_output: CellOutput { capacity: 1000, lock: retained_lock.clone(), type_: None }, data: Some(vec![]) },
        );

        let mut skip = HashSet::new();
        skip.insert(skipped_lock.hash());
        let groups =
            TransactionScriptVerifier::new(tx, Arc::new(provider)).with_skip_lock_script_hashes(skip).extract_script_groups().unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_type, ScriptGroupType::Lock);
        assert_eq!(groups[0].script, retained_lock);
        assert_eq!(groups[0].input_indices, vec![1]);
    }

    #[test]
    fn test_extract_script_groups_uses_ckb_script_hashes_under_ckb_strict_semantics() {
        let skipped_lock = Script::new([0x11; 32], 0, vec![0xAA]);
        let retained_lock = Script::new([0x22; 32], 0, vec![0xBB]);
        let skipped_out_point = OutPoint::new([0x31; 32], 0);
        let retained_out_point = OutPoint::new([0x32; 32], 0);
        let tx = Arc::new(
            CellTx::new(
                vec![CellInput::new(skipped_out_point, 0), CellInput::new(retained_out_point, 0)],
                vec![],
                vec![],
                vec![],
                vec![],
            )
            .unwrap(),
        );

        let mut provider = SimpleDataProvider::new();
        provider.add_cell(
            [0x31; 32],
            0,
            ResolvedCell { cell_output: CellOutput { capacity: 1000, lock: skipped_lock.clone(), type_: None }, data: Some(vec![]) },
        );
        provider.add_cell(
            [0x32; 32],
            0,
            ResolvedCell { cell_output: CellOutput { capacity: 1000, lock: retained_lock.clone(), type_: None }, data: Some(vec![]) },
        );

        let ckb_skipped_hash = ckb_script_hash_molecule(&skipped_lock).unwrap();
        assert_ne!(ckb_skipped_hash, skipped_lock.hash());
        let mut skip = HashSet::new();
        skip.insert(ckb_skipped_hash);
        let groups = TransactionScriptVerifier::new(tx, Arc::new(provider))
            .with_semantics(VmSemantics::CkbStrict)
            .with_skip_lock_script_hashes(skip)
            .extract_script_groups()
            .unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group_type, ScriptGroupType::Lock);
        assert_eq!(groups[0].script, retained_lock);
        assert_eq!(groups[0].input_indices, vec![1]);
    }

    #[test]
    fn test_verifier_defaults_to_myelin_extended_semantics() {
        let tx = Arc::new(CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap());
        let provider = Arc::new(SimpleDataProvider::new());

        let verifier = TransactionScriptVerifier::new(tx, provider);

        assert_eq!(verifier.semantics, VmSemantics::MyelinExtended);
    }

    #[test]
    fn test_verifier_allows_overriding_semantics() {
        let tx = Arc::new(CellTx::new(vec![], vec![], vec![], vec![], vec![]).unwrap());
        let provider = Arc::new(SimpleDataProvider::new());

        let verifier = TransactionScriptVerifier::new(tx, provider).with_semantics(VmSemantics::CkbStrict);

        assert_eq!(verifier.semantics, VmSemantics::CkbStrict);
    }

    #[test]
    fn test_resumable_verify_suspends_and_resume_completes() {
        let verifier = always_success_verifier();

        let initial = verifier.resumable_verify(1).expect("initial resumable verify should succeed");
        let state = match initial {
            VerifyResult::Suspended(state) => state,
            VerifyResult::Completed(cycles) => panic!("expected suspension, got completion with {cycles} cycles"),
        };

        let resumed = verifier.resume_from_state(&state, 10_000).expect("resume_from_state should succeed");

        let resumed_cycles = match resumed {
            VerifyResult::Completed(cycles) => cycles,
            VerifyResult::Suspended(_) => panic!("expected resumed verification to complete"),
        };

        let direct_cycles = verifier.verify_with_cycles().expect("direct verification should succeed");
        assert_eq!(resumed_cycles, direct_cycles);
    }

    #[test]
    fn test_complete_finishes_suspended_verification() {
        let verifier = always_success_verifier();

        let initial = verifier.resumable_verify(1).expect("initial resumable verify should succeed");
        let state = match initial {
            VerifyResult::Suspended(state) => state,
            VerifyResult::Completed(cycles) => panic!("expected suspension, got completion with {cycles} cycles"),
        };

        let completed_cycles = verifier.complete(&state, 10_000).expect("complete should finish verification");
        let direct_cycles = verifier.verify_with_cycles().expect("direct verification should succeed");

        assert_eq!(completed_cycles, direct_cycles);
    }

    #[test]
    fn test_transaction_state_next_limit_cycles_caps_at_max() {
        let state = TransactionState::new(None, 0, 80, 10);

        let (next_limit, last) = state.next_limit_cycles(15, 100);
        assert_eq!(next_limit, 80);
        assert!(!last);

        let state = TransactionState::new(None, 0, 80, 90);
        let (next_limit, last) = state.next_limit_cycles(15, 100);
        assert_eq!(next_limit, 100);
        assert!(last);
    }
}
