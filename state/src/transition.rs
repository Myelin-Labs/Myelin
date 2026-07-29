// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Atomic live-cell state transitions.

use crate::{CellEntry, CellStateTree};
use myelin_exec::{CapacityError, CellTx, OutPoint};
use myelin_hashes::Hash;
use std::{collections::BTreeSet, fmt::Display};

/// Chain/session context required to apply one transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateTransitionContext {
    /// Block or session height assigned to newly created cells.
    pub block_number: u64,
    /// Explicitly permit a no-input genesis/cellbase transaction.
    pub allow_cellbase: bool,
}

impl StateTransitionContext {
    /// Context for an ordinary transaction.
    pub const fn ordinary(block_number: u64) -> Self {
        Self { block_number, allow_cellbase: false }
    }

    /// Context for an explicitly authorized genesis/cellbase transaction.
    pub const fn cellbase(block_number: u64) -> Self {
        Self { block_number, allow_cellbase: true }
    }
}

/// Input together with the live cell resolved from the pre-state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedStateInput {
    /// Consumed live-cell outpoint.
    pub outpoint: OutPoint,
    /// Full committed entry available to script verification.
    pub cell: CellEntry,
}

/// Successful atomic transition receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateTransitionReceipt {
    /// Raw transaction identifier; witnesses are intentionally excluded.
    pub txid: [u8; 32],
    /// Root of the live-cell set before the transition.
    pub state_root_before: Hash,
    /// Root of the live-cell set after the transition.
    pub state_root_after: Hash,
    /// Consumed outpoints in transaction input order.
    pub consumed: Vec<OutPoint>,
    /// Created outpoints in transaction output order.
    pub created: Vec<OutPoint>,
    /// Capacity fee: total inputs minus total outputs.
    pub fee: u64,
    /// Cycles returned by the authoritative script verifier.
    pub cycles: u64,
}

/// Immutable admission proof produced by contextual state and script checks.
///
/// Fields are private so fee and cycle values cannot be supplied independently
/// of the transaction that was verified.
#[derive(Clone, Debug)]
pub struct VerifiedStateTransaction {
    tx: CellTx,
    txid: [u8; 32],
    state_root: Hash,
    state_root_after: Hash,
    fee: u64,
    cycles: u64,
}

impl VerifiedStateTransaction {
    /// Verified transaction.
    pub fn transaction(&self) -> &CellTx {
        &self.tx
    }

    /// Raw transaction id bound to this admission proof.
    pub fn txid(&self) -> [u8; 32] {
        self.txid
    }

    /// State root against which inputs and capacity were resolved.
    pub fn state_root(&self) -> Hash {
        self.state_root
    }

    /// State root produced by the verified transition overlay.
    pub fn state_root_after(&self) -> Hash {
        self.state_root_after
    }

    /// Capacity fee derived from resolved inputs and outputs.
    pub fn fee(&self) -> u64 {
        self.fee
    }

    /// Cycles returned by the verifier.
    pub fn cycles(&self) -> u64 {
        self.cycles
    }
}

/// Rejection reasons that leave the state tree unchanged.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StateTransitionError {
    /// No-input transactions require an explicit cellbase/genesis context.
    #[error("no-input transaction is not authorized in this context")]
    UnauthorizedCellbase,
    /// The same live cell appears more than once in the input list.
    #[error("duplicate input {0}")]
    DuplicateInput(OutPoint),
    /// An input is not live in the transition pre-state.
    #[error("input cell is not live: {0}")]
    MissingInput(OutPoint),
    /// A created outpoint already exists in the live-cell set.
    #[error("created outpoint already exists: {0}")]
    OutputAlreadyExists(OutPoint),
    /// One output does not contain enough capacity for its shape and data.
    #[error("output {index} has invalid capacity: {source}")]
    InvalidOutputCapacity {
        /// Output index.
        index: usize,
        /// Structured occupied-capacity error.
        source: CapacityError,
    },
    /// Summing capacities overflowed `u64`.
    #[error("capacity sum overflow")]
    CapacityOverflow,
    /// Outputs create more capacity than the resolved inputs contain.
    #[error("insufficient input capacity: inputs {inputs}, outputs {outputs}")]
    InsufficientInputCapacity {
        /// Total resolved input capacity.
        inputs: u64,
        /// Total output capacity.
        outputs: u64,
    },
    /// Output metadata could not be encoded with CKB-compatible hashes.
    #[error("invalid output metadata at index {index}: {reason}")]
    InvalidOutputMetadata {
        /// Output index.
        index: usize,
        /// Hash/serialization failure.
        reason: String,
    },
    /// Script verification failed before state mutation.
    #[error("script verification failed: {0}")]
    ScriptVerification(String),
}

/// Owns the canonical in-memory live-cell state and applies transactions atomically.
#[derive(Clone)]
pub struct StateTransitionEngine {
    state: CellStateTree,
}

impl StateTransitionEngine {
    /// Create an engine over an existing state snapshot.
    pub fn new(state: CellStateTree) -> Self {
        Self { state }
    }

    /// Borrow the committed state.
    pub fn state(&self) -> &CellStateTree {
        &self.state
    }

    /// Mutably borrow the committed state for explicit bootstrap/import only.
    pub fn state_mut_for_bootstrap(&mut self) -> &mut CellStateTree {
        &mut self.state
    }

    /// Consume the engine and return the committed state.
    pub fn into_state(self) -> CellStateTree {
        self.state
    }

    /// Return the current committed root.
    pub fn state_root(&mut self) -> Hash {
        self.state.root()
    }

    /// Verify a transaction against the current snapshot without committing it.
    ///
    /// This is the sole constructor for [`VerifiedStateTransaction`], used by
    /// mempool admission. Inclusion must still re-run or atomically commit the
    /// transition against the then-current state root.
    pub fn verify_transaction<F, E>(
        &mut self,
        tx: &CellTx,
        context: StateTransitionContext,
        verifier: F,
    ) -> Result<VerifiedStateTransaction, StateTransitionError>
    where
        F: FnOnce(&CellTx, &[ResolvedStateInput]) -> Result<u64, E>,
        E: Display,
    {
        let mut overlay = self.clone();
        let receipt = overlay.apply_transaction(tx, context, verifier)?;
        Ok(VerifiedStateTransaction {
            tx: tx.clone(),
            txid: receipt.txid,
            state_root: receipt.state_root_before,
            state_root_after: receipt.state_root_after,
            fee: receipt.fee,
            cycles: receipt.cycles,
        })
    }

    /// Verify and atomically apply one transaction.
    ///
    /// The verifier runs against inputs resolved from the exact pre-state. The
    /// state is swapped only after all contextual, capacity, metadata and script
    /// checks succeed.
    pub fn apply_transaction<F, E>(
        &mut self,
        tx: &CellTx,
        context: StateTransitionContext,
        verifier: F,
    ) -> Result<StateTransitionReceipt, StateTransitionError>
    where
        F: FnOnce(&CellTx, &[ResolvedStateInput]) -> Result<u64, E>,
        E: Display,
    {
        if tx.inputs.is_empty() && !context.allow_cellbase {
            return Err(StateTransitionError::UnauthorizedCellbase);
        }

        let state_root_before = self.state.root();
        let mut seen_inputs = BTreeSet::new();
        let mut resolved_inputs = Vec::with_capacity(tx.inputs.len());
        let mut input_capacity = 0u64;
        for input in &tx.inputs {
            let outpoint = input.previous_output;
            if !seen_inputs.insert(outpoint) {
                return Err(StateTransitionError::DuplicateInput(outpoint));
            }
            let cell = self.state.get(&outpoint).cloned().ok_or(StateTransitionError::MissingInput(outpoint))?;
            input_capacity = input_capacity.checked_add(cell.capacity).ok_or(StateTransitionError::CapacityOverflow)?;
            resolved_inputs.push(ResolvedStateInput { outpoint, cell });
        }

        let txid = tx.id();
        let mut output_capacity = 0u64;
        let mut created_entries = Vec::with_capacity(tx.outputs.len());
        for (index, (output, data)) in tx.outputs.iter().zip(&tx.outputs_data).enumerate() {
            output.verify_capacity(data.len()).map_err(|source| StateTransitionError::InvalidOutputCapacity { index, source })?;
            output_capacity = output_capacity.checked_add(output.capacity).ok_or(StateTransitionError::CapacityOverflow)?;
            let outpoint = OutPoint::new(txid, index as u32);
            if self.state.get(&outpoint).is_some() {
                return Err(StateTransitionError::OutputAlreadyExists(outpoint));
            }
            let entry = CellEntry::from_output(output, data, context.block_number, tx.inputs.is_empty())
                .map_err(|error| StateTransitionError::InvalidOutputMetadata { index, reason: error.to_string() })?;
            created_entries.push((outpoint, entry));
        }

        if !tx.inputs.is_empty() && output_capacity > input_capacity {
            return Err(StateTransitionError::InsufficientInputCapacity { inputs: input_capacity, outputs: output_capacity });
        }

        let cycles = verifier(tx, &resolved_inputs).map_err(|error| StateTransitionError::ScriptVerification(error.to_string()))?;

        let mut next_state = self.state.clone();
        for input in &resolved_inputs {
            let removed = next_state.remove(&input.outpoint);
            debug_assert!(removed.is_some(), "prechecked live input disappeared from isolated overlay");
        }
        let created = created_entries.iter().map(|(outpoint, _)| *outpoint).collect::<Vec<_>>();
        for (outpoint, entry) in created_entries {
            next_state.insert(outpoint, entry);
        }
        let state_root_after = next_state.root();
        self.state = next_state;

        Ok(StateTransitionReceipt {
            txid,
            state_root_before,
            state_root_after,
            consumed: resolved_inputs.iter().map(|input| input.outpoint).collect(),
            created,
            fee: input_capacity.saturating_sub(output_capacity),
            cycles,
        })
    }
}

impl Default for StateTransitionEngine {
    fn default() -> Self {
        Self::new(CellStateTree::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::{CellInput, CellOutput, Script};

    fn output(capacity: u64) -> CellOutput {
        CellOutput { lock: Script::new([1u8; 32], 0, vec![]), type_: None, capacity }
    }

    fn seed_engine(capacity: u64) -> (StateTransitionEngine, OutPoint) {
        let outpoint = OutPoint::new([9u8; 32], 0);
        let mut state = CellStateTree::new();
        state.insert(outpoint, CellEntry::from_output(&output(capacity), &[], 0, false).unwrap());
        (StateTransitionEngine::new(state), outpoint)
    }

    fn spending_tx(input: OutPoint, capacity: u64) -> CellTx {
        CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![output(capacity)], vec![vec![]], vec![]).unwrap()
    }

    #[test]
    fn applies_spend_and_create_atomically() {
        let (mut engine, input) = seed_engine(1_000);
        let tx = spending_tx(input, 900);
        let receipt = engine
            .apply_transaction(&tx, StateTransitionContext::ordinary(7), |_, resolved| {
                assert_eq!(resolved.len(), 1);
                Ok::<_, String>(42)
            })
            .unwrap();

        assert_eq!(receipt.fee, 100);
        assert_eq!(receipt.cycles, 42);
        assert_ne!(receipt.state_root_before, receipt.state_root_after);
        assert!(engine.state().get(&input).is_none());
        assert!(engine.state().get(&OutPoint::new(tx.id(), 0)).is_some());
    }

    #[test]
    fn verifier_failure_leaves_state_unchanged() {
        let (mut engine, input) = seed_engine(1_000);
        let root_before = engine.state_root();
        let tx = spending_tx(input, 900);
        let error = engine.apply_transaction(&tx, StateTransitionContext::ordinary(1), |_, _| Err::<u64, _>("rejected")).unwrap_err();

        assert_eq!(error, StateTransitionError::ScriptVerification("rejected".to_owned()));
        assert_eq!(engine.state_root(), root_before);
        assert!(engine.state().get(&input).is_some());
    }

    #[test]
    fn duplicate_input_is_rejected_without_mutation() {
        let (mut engine, input) = seed_engine(1_000);
        let root_before = engine.state_root();
        let tx =
            CellTx::new(vec![CellInput::new(input, 0), CellInput::new(input, 0)], vec![], vec![output(900)], vec![vec![]], vec![])
                .unwrap();

        assert_eq!(
            engine.apply_transaction(&tx, StateTransitionContext::ordinary(1), |_, _| Ok::<_, String>(0)),
            Err(StateTransitionError::DuplicateInput(input))
        );
        assert_eq!(engine.state_root(), root_before);
    }

    #[test]
    fn no_input_transaction_requires_explicit_context() {
        let mut engine = StateTransitionEngine::default();
        let tx = CellTx::new(vec![], vec![], vec![output(100)], vec![vec![]], vec![]).unwrap();

        assert_eq!(
            engine.apply_transaction(&tx, StateTransitionContext::ordinary(0), |_, _| Ok::<_, String>(0)),
            Err(StateTransitionError::UnauthorizedCellbase)
        );
        assert!(engine.apply_transaction(&tx, StateTransitionContext::cellbase(0), |_, _| Ok::<_, String>(0)).is_ok());
    }
}
