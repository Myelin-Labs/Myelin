// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// CellDAG: RW-Set dependency graph construction

use crate::celltx::types::{CellTx, OutPoint, CELLSCRIPT_SCHEDULER_OP_READ_REF};
use std::collections::{BTreeMap, BTreeSet};

/// Node ID in the transaction DAG
pub type NodeId = usize;

/// DAG edge type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DagEdge {
    /// Dependency edge: A produces Cell that B consumes
    Dependency,
    /// Read dependency: A produces Cell that B reads (deps)
    ReadDep,
}

/// Access mode for conflict detection
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessMode {
    /// Read-only access (READ_REF)
    Read,
    /// Write access (CONSUME, CREATE, DESTROY, TRANSFER)
    Write,
}

impl AccessMode {
    /// Derive access mode from scheduler operation
    pub fn from_operation(op: u8) -> Self {
        match op {
            CELLSCRIPT_SCHEDULER_OP_READ_REF => AccessMode::Read,
            _ => AccessMode::Write,
        }
    }
}

/// Conflict entry with access mode
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictEntry {
    /// Node ID of the transaction
    pub node_id: NodeId,
    /// Access mode (read or write)
    pub mode: AccessMode,
}

/// Cell transaction DAG
///
/// Builds a dependency graph from RW-Sets:
/// - Nodes: transactions
/// - Edges: data dependencies (outputs -> inputs)
/// - Conflicts: transactions competing for the same Cell or conflict_hash
#[derive(Debug, Clone)]
pub struct CellDAG {
    /// Number of nodes (transactions)
    pub node_count: usize,

    /// Adjacency list: node → [(successor, edge_type)]
    pub edges: BTreeMap<NodeId, Vec<(NodeId, DagEdge)>>,

    /// Reverse adjacency: node → [predecessors]
    pub reverse_edges: BTreeMap<NodeId, Vec<NodeId>>,

    /// Conflict groups: OutPoint -> [NodeIds competing for it]
    pub conflicts: BTreeMap<OutPoint, Vec<NodeId>>,

    /// Conflict-hash-level entries: conflict_hash -> [ConflictEntry]
    ///
    /// Used for typed-cell conflict detection where multiple transactions
    /// touch the same stable conflict domain.
    pub conflict_hash_conflicts: BTreeMap<[u8; 32], Vec<ConflictEntry>>,

    /// Topological layers (for parallel execution)
    pub layers: Vec<Vec<NodeId>>,
}

impl CellDAG {
    /// Build DAG from a set of Cell transactions
    ///
    /// # Algorithm
    /// 1. Build RW-Sets for each transaction
    /// 2. Detect dependencies: A.outputs ∩ B.inputs → A → B
    /// 3. Detect read cell_deps: A.outputs ∩ B.cell_deps → A → B
    /// 4. Detect conflicts: A.inputs ∩ B.inputs ≠ ∅
    /// 5. Compute topological layers
    pub fn build(txs: &[CellTx]) -> Result<Self, DagError> {
        let node_count = txs.len();
        let mut edges: BTreeMap<NodeId, Vec<(NodeId, DagEdge)>> = BTreeMap::new();
        let mut reverse_edges: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        let mut conflicts: BTreeMap<OutPoint, Vec<NodeId>> = BTreeMap::new();

        // Step 1: Build producers map (OutPoint → NodeId)
        let mut producers: BTreeMap<OutPoint, NodeId> = BTreeMap::new();
        for (node_id, tx) in txs.iter().enumerate() {
            let tx_hash = crate::celltx::sighash::compute_wtxid(tx);
            for (idx, _) in tx.outputs.iter().enumerate() {
                let out_point = OutPoint::new(tx_hash, idx as u32);
                producers.insert(out_point, node_id);
            }
        }

        // Step 2: Detect dependencies and conflicts
        for (consumer_id, tx) in txs.iter().enumerate() {
            // Check inputs (consume edges)
            for input in &tx.inputs {
                if let Some(&producer_id) = producers.get(&input.previous_output) {
                    // Dependency: producer → consumer
                    edges.entry(producer_id).or_default().push((consumer_id, DagEdge::Dependency));

                    reverse_edges.entry(consumer_id).or_default().push(producer_id);
                } else {
                    // External Cell (not in this DAG)
                    // Will be resolved from state layer
                }

                // Track conflicts (multiple consumers for same Cell)
                conflicts.entry(input.previous_output.clone()).or_default().push(consumer_id);
            }

            // Check deps (read-only edges)
            for dep in &tx.cell_deps {
                if let Some(&producer_id) = producers.get(&dep.out_point) {
                    edges.entry(producer_id).or_default().push((consumer_id, DagEdge::ReadDep));

                    reverse_edges.entry(consumer_id).or_default().push(producer_id);
                }
            }
        }

        // Step 3: Filter conflicts (keep only actual conflicts)
        conflicts.retain(|_, consumers| consumers.len() > 1);

        // Step 4: Compute topological layers
        let layers = Self::compute_layers(node_count, &edges, &reverse_edges)?;

        Ok(CellDAG { node_count, edges, reverse_edges, conflicts, conflict_hash_conflicts: BTreeMap::new(), layers })
    }

    /// Build DAG from typed cell transactions with conflict_hash awareness.
    ///
    /// Extends the base `build` with typed-cell conflict rules:
    ///
    /// ```text
    /// READ  + READ  same conflict_hash → same layer (no edge)
    /// READ  + WRITE same conflict_hash → dependency edge
    /// WRITE + WRITE same conflict_hash → dependency edge (different layers)
    /// ```
    pub fn build_from_typed(txs: &[CellTx]) -> Result<Self, DagError> {
        let mut dag = Self::build(txs)?;

        // Extract conflict_hash accesses from scheduler witnesses
        let mut conflict_hash_conflicts: BTreeMap<[u8; 32], Vec<ConflictEntry>> = BTreeMap::new();

        for (node_id, tx) in txs.iter().enumerate() {
            for witness_result in tx.decoded_cellscript_scheduler_witnesses() {
                if let Ok(witness) = witness_result {
                    for access in &witness.accesses {
                        let mode = AccessMode::from_operation(access.operation);
                        let entry = ConflictEntry { node_id, mode };
                        conflict_hash_conflicts
                            .entry(access.conflict_hash)
                            .or_default()
                            .push(entry);
                    }
                }
                // If decode fails, skip — the transaction won't have valid
                // scheduler metadata, so it can't participate in typed-cell
                // parallel scheduling. It still has OutPoint-level dependencies.
            }
        }

        // Apply conflict rules: add dependency edges where needed
        for (_conflict_hash, entries) in &conflict_hash_conflicts {
            // For each pair of entries sharing the same conflict_hash,
            // add a dependency edge if at least one is a Write.
            for i in 0..entries.len() {
                for j in (i + 1)..entries.len() {
                    let a = &entries[i];
                    let b = &entries[j];

                    // READ + READ → no edge (same layer)
                    if a.mode == AccessMode::Read && b.mode == AccessMode::Read {
                        continue;
                    }

                    // READ + WRITE or WRITE + WRITE → dependency edge
                    // Earlier transaction must come first
                    let (from, to) = if a.node_id < b.node_id {
                        (a.node_id, b.node_id)
                    } else {
                        (b.node_id, a.node_id)
                    };

                    // Avoid duplicate edges
                    let already_has_edge = dag
                        .edges
                        .get(&from)
                        .map_or(false, |succs| succs.iter().any(|(s, _)| *s == to));

                    if !already_has_edge {
                        dag.edges.entry(from).or_default().push((to, DagEdge::Dependency));
                        dag.reverse_edges.entry(to).or_default().push(from);
                    }
                }
            }
        }

        dag.conflict_hash_conflicts = conflict_hash_conflicts;

        // Recompute layers with the new edges
        dag.layers = Self::compute_layers(dag.node_count, &dag.edges, &dag.reverse_edges)?;

        Ok(dag)
    }

    /// Extract the conflict_hash accesses from a transaction's scheduler witness.
    ///
    /// Returns a vector of (conflict_hash, AccessMode) pairs.
    /// Returns an empty vector if the transaction has no valid scheduler witness.
    pub fn extract_conflict_accesses(
        tx: &CellTx,
    ) -> Vec<([u8; 32], AccessMode)> {
        let mut result = Vec::new();
        for witness_result in tx.decoded_cellscript_scheduler_witnesses() {
            if let Ok(witness) = witness_result {
                for access in &witness.accesses {
                    let mode = AccessMode::from_operation(access.operation);
                    result.push((access.conflict_hash, mode));
                }
            }
        }
        result
    }

    /// Check if two transactions can be placed in the same layer
    /// based on their conflict_hash access patterns.
    ///
    /// Returns `true` if there is no Write-based conflict between them.
    pub fn can_parallel(
        accesses_a: &[( [u8; 32], AccessMode )],
        accesses_b: &[( [u8; 32], AccessMode )],
    ) -> bool {
        // Build a map of conflict_hash → AccessMode for A
        let mut a_map: BTreeMap<[u8; 32], AccessMode> = BTreeMap::new();
        for &(hash, mode) in accesses_a {
            // If we've seen this hash before, upgrade to Write if either access is Write
            let entry = a_map.entry(hash).or_insert(AccessMode::Read);
            if mode == AccessMode::Write {
                *entry = AccessMode::Write;
            }
        }

        // Check for conflicts with B
        for &(hash, mode_b) in accesses_b {
            if let Some(mode_a) = a_map.get(&hash) {
                // READ + READ is fine; anything involving a Write creates a conflict
                if *mode_a == AccessMode::Write || mode_b == AccessMode::Write {
                    return false;
                }
            }
        }

        true
    }

    /// Compute topological layers for parallel execution
    ///
    /// Uses Kahn's algorithm with layer tracking:
    /// - Layer 0: nodes with no predecessors
    /// - Layer N: nodes whose all predecessors are in layers < N
    fn compute_layers(
        node_count: usize,
        edges: &BTreeMap<NodeId, Vec<(NodeId, DagEdge)>>,
        reverse_edges: &BTreeMap<NodeId, Vec<NodeId>>,
    ) -> Result<Vec<Vec<NodeId>>, DagError> {
        if node_count == 0 {
            return Ok(Vec::new());
        }

        let mut in_degree = vec![0usize; node_count];
        let mut layers = Vec::new();
        let mut current_layer = Vec::new();
        let mut processed = 0usize;

        // Compute in-degrees
        for (node, degree) in in_degree.iter_mut().enumerate().take(node_count) {
            *degree = reverse_edges.get(&node).map_or(0, |preds| preds.len());
            if *degree == 0 {
                current_layer.push(node);
            }
        }

        while !current_layer.is_empty() {
            current_layer.sort_unstable();
            layers.push(current_layer.clone());
            processed += current_layer.len();

            let mut next_layer = Vec::new();
            for node in current_layer {
                if let Some(successors) = edges.get(&node) {
                    for &(successor, _) in successors {
                        let degree = in_degree
                            .get_mut(successor)
                            .ok_or_else(|| DagError::InvalidRWSet(format!("successor node {successor} is out of bounds")))?;
                        if *degree == 0 {
                            return Err(DagError::InvalidRWSet(format!(
                                "successor node {successor} reached zero in-degree too early"
                            )));
                        }
                        *degree -= 1;
                        if *degree == 0 {
                            next_layer.push(successor);
                        }
                    }
                }
            }

            current_layer = next_layer;
        }

        if processed != node_count {
            return Err(DagError::CycleDetected);
        }

        Ok(layers)
    }

    /// Get all conflicts in the DAG
    pub fn get_conflicts(&self) -> Vec<(&OutPoint, &[NodeId])> {
        self.conflicts.iter().map(|(op, nodes)| (op, nodes.as_slice())).collect()
    }

    /// Get successors of a node
    pub fn successors(&self, node: NodeId) -> Option<&[(NodeId, DagEdge)]> {
        self.edges.get(&node).map(|v| v.as_slice())
    }

    /// Get predecessors of a node
    pub fn predecessors(&self, node: NodeId) -> Option<&[NodeId]> {
        self.reverse_edges.get(&node).map(|v| v.as_slice())
    }

    /// Check if there's a dependency path from A to B
    pub fn has_path(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }

        let mut visited = BTreeSet::new();
        let mut stack = vec![from];

        while let Some(node) = stack.pop() {
            if node == to {
                return true;
            }
            if visited.insert(node) {
                if let Some(succs) = self.edges.get(&node) {
                    for &(succ, _) in succs {
                        stack.push(succ);
                    }
                }
            }
        }

        false
    }
}

/// DAG node metadata
#[derive(Debug, Clone)]
pub struct DagNode {
    /// Node ID
    pub id: NodeId,
    /// Transaction
    pub tx: CellTx,
    /// Layer in topological sort
    pub layer: usize,
}

/// DAG construction errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum DagError {
    /// Cycle detected in dependency graph
    #[error("Cycle detected in transaction DAG")]
    CycleDetected,

    /// Invalid RW-Set (missing declarations)
    #[error("Invalid RW-Set: {0}")]
    InvalidRWSet(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::types::{CellInput, CellOutput, Script};

    fn create_test_tx(inputs: Vec<OutPoint>, outputs_count: usize) -> CellTx {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let inputs = inputs.into_iter().map(|op| CellInput::new(op, 0)).collect();
        let outputs = vec![CellOutput { lock: lock.clone(), type_: None, capacity: 1000 }; outputs_count];
        let outputs_data = vec![vec![]; outputs_count];
        CellTx::new(inputs, vec![], outputs, outputs_data, vec![]).unwrap()
    }

    #[test]
    fn test_dag_simple_chain() {
        // tx0 → tx1 → tx2 (simple chain)
        let tx0 = create_test_tx(vec![], 1);
        let tx0_hash = crate::celltx::sighash::compute_wtxid(&tx0);

        let tx1 = create_test_tx(vec![OutPoint::new(tx0_hash, 0)], 1);
        let tx1_hash = crate::celltx::sighash::compute_wtxid(&tx1);

        let tx2 = create_test_tx(vec![OutPoint::new(tx1_hash, 0)], 1);

        let dag = CellDAG::build(&[tx0, tx1, tx2]).unwrap();

        assert_eq!(dag.node_count, 3);
        assert_eq!(dag.layers, vec![vec![0], vec![1], vec![2]]);
        assert!(dag.has_path(0, 2));
        assert!(!dag.has_path(2, 0));
    }

    #[test]
    fn test_dag_conflict_detection() {
        // tx0 produces Cell
        // tx1 and tx2 both try to consume it (conflict)
        let tx0 = create_test_tx(vec![], 1);
        let tx0_hash = crate::celltx::sighash::compute_wtxid(&tx0);
        let out = OutPoint::new(tx0_hash, 0);

        let tx1 = create_test_tx(vec![out.clone()], 1);
        let tx2 = create_test_tx(vec![out.clone()], 1);

        let dag = CellDAG::build(&[tx0, tx1, tx2]).unwrap();

        // Should detect conflict between tx1 and tx2
        let conflicts = dag.get_conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].1.len(), 2); // tx1 and tx2
    }

    #[test]
    fn test_dag_parallel_branches() {
        // tx0 produces 2 outputs
        // tx1 consumes output 0
        // tx2 consumes output 1
        // (parallel, no conflict)
        let tx0 = create_test_tx(vec![], 2);
        let tx0_hash = crate::celltx::sighash::compute_wtxid(&tx0);

        let tx1 = create_test_tx(vec![OutPoint::new(tx0_hash, 0)], 1);
        let tx2 = create_test_tx(vec![OutPoint::new(tx0_hash, 1)], 1);

        let dag = CellDAG::build(&[tx0, tx1, tx2]).unwrap();

        // No conflicts (different outputs)
        assert!(dag.conflicts.is_empty());
        assert_eq!(dag.layers, vec![vec![0], vec![1, 2]]);

        // Both tx1 and tx2 depend on tx0
        assert!(dag.has_path(0, 1));
        assert!(dag.has_path(0, 2));
        assert!(!dag.has_path(1, 2)); // tx1 and tx2 are independent
    }

    #[test]
    fn test_compute_layers_detects_cycle() {
        let mut edges = BTreeMap::new();
        let mut reverse_edges = BTreeMap::new();

        edges.insert(0, vec![(1, DagEdge::Dependency)]);
        edges.insert(1, vec![(0, DagEdge::Dependency)]);
        reverse_edges.insert(0, vec![1]);
        reverse_edges.insert(1, vec![0]);

        let result = CellDAG::compute_layers(2, &edges, &reverse_edges);
        assert!(matches!(result, Err(DagError::CycleDetected)));
    }

    #[test]
    fn test_compute_layers_empty_dag() {
        let edges = BTreeMap::new();
        let reverse_edges = BTreeMap::new();

        let layers = CellDAG::compute_layers(0, &edges, &reverse_edges).unwrap();
        assert!(layers.is_empty());
    }

    // ─── Typed Cell Conflict Hash Tests ─────────────────────────────────────────

    use crate::celltx::types::{
        CellScriptSchedulerWitness, CellScriptSchedulerAccessWitness,
        TYPED_CELL_SCHEDULER_WITNESS_VERSION,
        CELLSCRIPT_SCHEDULER_EFFECT_MUTATING, CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
        CELLSCRIPT_SCHEDULER_OP_CONSUME, CELLSCRIPT_SCHEDULER_OP_READ_REF,
        CELLSCRIPT_SCHEDULER_SOURCE_INPUT, CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
        encode_cellscript_scheduler_witness_molecule,
    };

    fn create_typed_test_tx_with_witness(
        outputs_count: usize,
        access: Option<CellScriptSchedulerAccessWitness>,
        effect_class: u8,
    ) -> CellTx {
        let lock = Script::new([0x00; 32], 0, vec![]);
        let outputs = vec![CellOutput { lock: lock.clone(), type_: None, capacity: 1000 }; outputs_count];
        let outputs_data = vec![vec![]; outputs_count];
        let mut tx = CellTx::new(vec![], vec![], outputs, outputs_data, vec![]).unwrap();

        if let Some(acc) = access {
            let witness = CellScriptSchedulerWitness {
                magic: 0xCE11,
                version: TYPED_CELL_SCHEDULER_WITNESS_VERSION,
                effect_class,
                parallelizable: false,
                estimated_cycles: 500,
                access_count: 1,
                accesses: vec![acc],
            };
            let encoded = encode_cellscript_scheduler_witness_molecule(&witness);
            tx.push_cellscript_scheduler_witness(encoded).unwrap();
        }

        tx
    }

    #[test]
    fn test_typed_dag_write_write_same_conflict_hash_creates_dependency() {
        // Two transactions both WRITE to the same conflict_hash → dependency edge
        let pool_hash = [0xAA; 32];

        let tx0 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_hash,
                typed_data_hash: [0x01; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let tx1 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_hash,
                typed_data_hash: [0x02; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let dag = CellDAG::build_from_typed(&[tx0, tx1]).unwrap();

        // WRITE + WRITE same conflict_hash → dependency edge
        assert!(dag.has_path(0, 1), "WRITE+WRITE same conflict_hash must create dependency");
        assert_eq!(dag.layers.len(), 2, "WRITE+WRITE must be in different layers");
    }

    #[test]
    fn test_typed_dag_read_read_same_conflict_hash_same_layer() {
        // Two transactions both READ the same conflict_hash → same layer
        let config_hash = [0xBB; 32];

        let tx0 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                index: 0,
                conflict_hash: config_hash,
                typed_data_hash: [0x00; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
        );

        let tx1 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                index: 0,
                conflict_hash: config_hash,
                typed_data_hash: [0x00; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
        );

        let dag = CellDAG::build_from_typed(&[tx0, tx1]).unwrap();

        // READ + READ same conflict_hash → same layer (no dependency)
        assert!(!dag.has_path(0, 1), "READ+READ same conflict_hash must NOT create dependency");
        assert_eq!(dag.layers.len(), 1, "READ+READ must be in same layer");
    }

    #[test]
    fn test_typed_dag_read_write_same_conflict_hash_creates_dependency() {
        // One READ, one WRITE same conflict_hash → dependency edge
        let pool_hash = [0xCC; 32];

        let tx0 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                index: 0,
                conflict_hash: pool_hash,
                typed_data_hash: [0x00; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
        );

        let tx1 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_hash,
                typed_data_hash: [0x01; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let dag = CellDAG::build_from_typed(&[tx0, tx1]).unwrap();

        // READ + WRITE same conflict_hash → dependency edge
        assert!(dag.has_path(0, 1), "READ+WRITE same conflict_hash must create dependency");
        assert_eq!(dag.layers.len(), 2, "READ+WRITE must be in different layers");
    }

    #[test]
    fn test_typed_dag_different_conflict_hash_parallel() {
        // Two transactions with different conflict_hash → parallel (same layer)
        let pool_a_hash = [0xDD; 32];
        let pool_b_hash = [0xEE; 32];

        let tx0 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_a_hash,
                typed_data_hash: [0x01; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let tx1 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_b_hash,
                typed_data_hash: [0x02; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let dag = CellDAG::build_from_typed(&[tx0, tx1]).unwrap();

        // Different conflict_hash → parallel (no dependency)
        assert!(!dag.has_path(0, 1), "different conflict_hash must be parallel");
        assert_eq!(dag.layers.len(), 1, "different conflict domains must be in same layer");
    }

    #[test]
    fn test_typed_dag_can_parallel_utility() {
        let pool_hash = [0xAA; 32];
        let other_hash = [0xBB; 32];

        // READ + READ same hash → can parallel
        let reads_a: Vec<([u8; 32], AccessMode)> = vec![(pool_hash, AccessMode::Read)];
        let reads_b: Vec<([u8; 32], AccessMode)> = vec![(pool_hash, AccessMode::Read)];
        assert!(CellDAG::can_parallel(&reads_a, &reads_b));

        // READ + WRITE same hash → cannot parallel
        let write_b: Vec<([u8; 32], AccessMode)> = vec![(pool_hash, AccessMode::Write)];
        assert!(!CellDAG::can_parallel(&reads_a, &write_b));

        // Different hashes → can parallel
        let other: Vec<([u8; 32], AccessMode)> = vec![(other_hash, AccessMode::Write)];
        assert!(CellDAG::can_parallel(&reads_a, &other));

        // WRITE + WRITE same hash → cannot parallel
        let write_a: Vec<([u8; 32], AccessMode)> = vec![(pool_hash, AccessMode::Write)];
        assert!(!CellDAG::can_parallel(&write_a, &write_b));
    }

    #[test]
    fn test_typed_dag_mixed_conflict_domains() {
        // 4 transactions: 2 swap pool A, 1 quote pool A, 1 swap pool B
        // Pool A swaps → sequential (WRITE+WRITE)
        // Pool A quote + swap → sequential (READ+WRITE)
        // Pool A + Pool B → parallel (different conflict_hash)
        let pool_a = [0xAA; 32];
        let pool_b = [0xBB; 32];

        // tx0: swap pool A (WRITE)
        let tx0 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_a,
                typed_data_hash: [0x01; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        // tx1: swap pool B (WRITE) — parallel with tx0
        let tx1 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_b,
                typed_data_hash: [0x02; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        // tx2: quote pool A (READ) — must be after tx0 (READ after WRITE)
        let tx2 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_READ_REF,
                source: CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP,
                index: 0,
                conflict_hash: pool_a,
                typed_data_hash: [0x00; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY,
        );

        // tx3: swap pool A again (WRITE) — must be after tx2 (WRITE after READ)
        let tx3 = create_typed_test_tx_with_witness(
            1,
            Some(CellScriptSchedulerAccessWitness {
                operation: CELLSCRIPT_SCHEDULER_OP_CONSUME,
                source: CELLSCRIPT_SCHEDULER_SOURCE_INPUT,
                index: 0,
                conflict_hash: pool_a,
                typed_data_hash: [0x03; 32],
            }),
            CELLSCRIPT_SCHEDULER_EFFECT_MUTATING,
        );

        let dag = CellDAG::build_from_typed(&[tx0, tx1, tx2, tx3]).unwrap();

        // tx0 and tx1 are parallel (different conflict domains)
        assert!(!dag.has_path(0, 1), "pool A swap and pool B swap are parallel");

        // tx0 → tx2 (WRITE → READ on pool A)
        assert!(dag.has_path(0, 2), "pool A swap must precede pool A quote");

        // tx2 → tx3 (READ → WRITE on pool A)
        assert!(dag.has_path(2, 3), "pool A quote must precede pool A swap");

        // tx0 → tx3 transitive (pool A chain)
        assert!(dag.has_path(0, 3), "pool A first swap must precede pool A second swap");
    }
}
