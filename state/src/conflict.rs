// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Fail-closed resolution of typed-cell conflict domains from concrete state.

use crate::CellStateTree;
use myelin_exec::{
    celltx::{encode_conflict_key_value_composite, validate_typed_cell_decl},
    compute_conflict_hash, compute_typed_data_hash, CellTx, ConflictKeySpec, DepType, OutPoint, Script, ScriptId, TypedCellDecl,
};
use std::collections::BTreeMap;

/// Concrete transaction source selected by compiler access metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictCellSource {
    /// Consumed input.
    Input,
    /// Read-only Cell dependency.
    CellDep,
    /// Created output.
    Output,
}

/// Reads one named field from canonical typed Cell data.
///
/// Implementations must use the exact schema/code version authenticated for the
/// type script. Text parsing, lossy JSON coercion, and binding-name hashes are not
/// valid production implementations.
pub trait ConflictFieldReader {
    /// Return the canonical encoded value of one field.
    fn read_field(&self, type_script: &Script, data: &[u8], field: &str) -> Result<Vec<u8>, String>;
}

impl<F> ConflictFieldReader for F
where
    F: Fn(&Script, &[u8], &str) -> Result<Vec<u8>, String>,
{
    fn read_field(&self, type_script: &Script, data: &[u8], field: &str) -> Result<Vec<u8>, String> {
        self(type_script, data, field)
    }
}

/// Fully resolved scheduling commitment for one concrete Cell access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedConflictDomain {
    /// Stable logical conflict domain.
    pub conflict_hash: [u8; 32],
    /// Commitment to the exact current Cell data.
    pub typed_data_hash: [u8; 32],
}

/// Validated typed-cell declarations keyed by full type-script identity.
#[derive(Clone, Debug, Default)]
pub struct ConflictDomainRegistry {
    declarations: BTreeMap<ScriptId, TypedCellDecl>,
}

impl ConflictDomainRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one declaration after enforcing all cross-axis and key-shape rules.
    pub fn register(&mut self, type_script: Script, declaration: TypedCellDecl) -> Result<(), ConflictResolutionError> {
        validate_typed_cell_decl(&declaration).map_err(|error| ConflictResolutionError::InvalidDeclaration(error.to_string()))?;
        let script_id = ScriptId::from_script(&type_script);
        if let Some(existing) = self.declarations.get(&script_id) {
            if existing != &declaration {
                return Err(ConflictResolutionError::ConflictingDeclaration);
            }
            return Ok(());
        }
        self.declarations.insert(script_id, declaration);
        Ok(())
    }

    /// Resolve one compiler-selected transaction access against exact state/output data.
    pub fn resolve_transaction_access<R: ConflictFieldReader>(
        &self,
        state: &CellStateTree,
        tx: &CellTx,
        source: ConflictCellSource,
        index: u32,
        fields: &R,
    ) -> Result<ResolvedConflictDomain, ConflictResolutionError> {
        let index_usize = usize::try_from(index).unwrap_or(usize::MAX);
        let (outpoint, type_script, data) = match source {
            ConflictCellSource::Input => {
                let input = tx.inputs.get(index_usize).ok_or(ConflictResolutionError::SourceIndexOutOfBounds {
                    cell_source: source,
                    index,
                    available: tx.inputs.len(),
                })?;
                let entry =
                    state.get(&input.previous_output).ok_or(ConflictResolutionError::MissingLiveCell(input.previous_output))?;
                let type_script = entry.type_script.as_ref().ok_or(ConflictResolutionError::MissingTypeScript)?;
                let data = entry.data.as_deref().ok_or(ConflictResolutionError::MissingResolvedData)?;
                (input.previous_output, type_script, data)
            }
            ConflictCellSource::CellDep => {
                let dep = tx.cell_deps.get(index_usize).ok_or(ConflictResolutionError::SourceIndexOutOfBounds {
                    cell_source: source,
                    index,
                    available: tx.cell_deps.len(),
                })?;
                if dep.dep_type == DepType::DepGroup {
                    return Err(ConflictResolutionError::DepGroupRequiresExpansion);
                }
                let entry = state.get(&dep.out_point).ok_or(ConflictResolutionError::MissingLiveCell(dep.out_point))?;
                let type_script = entry.type_script.as_ref().ok_or(ConflictResolutionError::MissingTypeScript)?;
                let data = entry.data.as_deref().ok_or(ConflictResolutionError::MissingResolvedData)?;
                (dep.out_point, type_script, data)
            }
            ConflictCellSource::Output => {
                let output = tx.outputs.get(index_usize).ok_or(ConflictResolutionError::SourceIndexOutOfBounds {
                    cell_source: source,
                    index,
                    available: tx.outputs.len(),
                })?;
                let data = tx.outputs_data.get(index_usize).ok_or(ConflictResolutionError::MissingOutputData(index))?;
                let type_script = output.type_.as_ref().ok_or(ConflictResolutionError::MissingTypeScript)?;
                (OutPoint::new(tx.id(), index), type_script, data.as_slice())
            }
        };

        let declaration =
            self.declarations.get(&ScriptId::from_script(type_script)).ok_or(ConflictResolutionError::UnregisteredTypeScript)?;
        let key_value = match &declaration.runtime.conflict_key {
            ConflictKeySpec::CellId => outpoint.to_key().to_vec(),
            ConflictKeySpec::Field(field) => fields
                .read_field(type_script, data, field)
                .map_err(|reason| ConflictResolutionError::FieldRead { field: field.clone(), reason })?,
            ConflictKeySpec::Composite(field_names) => {
                let values = field_names
                    .iter()
                    .map(|field| {
                        fields
                            .read_field(type_script, data, field)
                            .map_err(|reason| ConflictResolutionError::FieldRead { field: field.clone(), reason })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                encode_conflict_key_value_composite(&values.iter().map(Vec::as_slice).collect::<Vec<_>>())
            }
            ConflictKeySpec::None => return Err(ConflictResolutionError::ConflictKeyDisabled),
        };

        Ok(ResolvedConflictDomain {
            conflict_hash: compute_conflict_hash(type_script, &key_value),
            typed_data_hash: compute_typed_data_hash(type_script, data),
        })
    }
}

/// Typed conflict-domain resolution failure.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConflictResolutionError {
    /// Declaration failed semantic or key-shape validation.
    #[error("invalid typed-cell declaration: {0}")]
    InvalidDeclaration(String),
    /// The same full script identity was registered with different semantics.
    #[error("conflicting declaration for the same type-script identity")]
    ConflictingDeclaration,
    /// Compiler source index is outside the transaction vector.
    #[error("{cell_source:?} index {index} is out of bounds; available {available}")]
    SourceIndexOutOfBounds {
        /// Source vector.
        cell_source: ConflictCellSource,
        /// Requested index.
        index: u32,
        /// Available entries.
        available: usize,
    },
    /// Selected live Cell does not exist in the exact state snapshot.
    #[error("selected Cell is not live: {0}")]
    MissingLiveCell(OutPoint),
    /// DepGroup entries must be expanded and authenticated before typed access resolution.
    #[error("typed conflict access cannot target an unexpanded DepGroup")]
    DepGroupRequiresExpansion,
    /// Selected output has no matching data item.
    #[error("missing output data at index {0}")]
    MissingOutputData(u32),
    /// Typed scheduling requires a type script.
    #[error("selected Cell has no type script")]
    MissingTypeScript,
    /// Live Cell metadata was imported without full data bytes.
    #[error("selected live Cell has no resolved data bytes")]
    MissingResolvedData,
    /// No validated typed declaration exists for the full type-script identity.
    #[error("selected type script is not registered for typed scheduling")]
    UnregisteredTypeScript,
    /// The declaration deliberately disables logical conflict scheduling.
    #[error("selected typed declaration has ConflictKeySpec::None")]
    ConflictKeyDisabled,
    /// Schema-aware field decoding failed.
    #[error("cannot read conflict-key field {field}: {reason}")]
    FieldRead {
        /// Field name.
        field: String,
        /// Codec error.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellEntry;
    use myelin_exec::{
        celltx::CellSettlement, CellAccounting, CellIdentity, CellInput, CellMutability, CellOutput, CellOwnership,
        RuntimeCellSemantics, TypedCellSemanticMetadata,
    };

    fn script() -> Script {
        Script::new([4; 32], 2, vec![5])
    }

    fn declaration(spec: ConflictKeySpec) -> TypedCellDecl {
        TypedCellDecl {
            runtime: RuntimeCellSemantics { ownership: CellOwnership::Shared, conflict_key: spec },
            semantic: TypedCellSemanticMetadata {
                mutability: CellMutability::Versioned,
                accounting: vec![CellAccounting::NonFungible],
                identity: CellIdentity::Singleton,
                settlement: CellSettlement::Committed,
            },
        }
    }

    fn output(data_len: usize) -> CellOutput {
        CellOutput { lock: Script::new([1; 32], 2, vec![]), type_: Some(script()), capacity: 1_000 + data_len as u64 }
    }

    #[test]
    fn field_key_is_stable_across_input_output_data_updates() {
        let old_data = [7, 1];
        let new_data = [7, 2];
        let input = OutPoint::new([8; 32], 0);
        let mut state = CellStateTree::new();
        state.insert(input, CellEntry::from_output(&output(old_data.len()), &old_data, 0, false).unwrap());
        let tx = CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![output(new_data.len())], vec![new_data.to_vec()], vec![])
            .unwrap();
        let mut registry = ConflictDomainRegistry::new();
        registry.register(script(), declaration(ConflictKeySpec::Field("session_id".to_owned()))).unwrap();
        let reader = |_: &Script, data: &[u8], field: &str| {
            if field == "session_id" {
                Ok(vec![data[0]])
            } else {
                Err("unknown field".to_owned())
            }
        };

        let before = registry.resolve_transaction_access(&state, &tx, ConflictCellSource::Input, 0, &reader).unwrap();
        let after = registry.resolve_transaction_access(&state, &tx, ConflictCellSource::Output, 0, &reader).unwrap();
        assert_eq!(before.conflict_hash, after.conflict_hash);
        assert_ne!(before.typed_data_hash, after.typed_data_hash);
    }

    #[test]
    fn missing_schema_field_fails_closed() {
        let input = OutPoint::new([8; 32], 0);
        let mut state = CellStateTree::new();
        state.insert(input, CellEntry::from_output(&output(1), &[7], 0, false).unwrap());
        let tx = CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![], vec![], vec![]).unwrap();
        let mut registry = ConflictDomainRegistry::new();
        registry.register(script(), declaration(ConflictKeySpec::Field("missing".to_owned()))).unwrap();
        let reader = |_: &Script, _: &[u8], _: &str| Err("not in schema".to_owned());

        assert!(matches!(
            registry.resolve_transaction_access(&state, &tx, ConflictCellSource::Input, 0, &reader),
            Err(ConflictResolutionError::FieldRead { .. })
        ));
    }
}
