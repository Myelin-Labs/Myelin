// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Serialization Performance Benchmarks

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use spora_exec::celltx::{ResolvedCellMeta, TransactionInfo};
use spora_exec::{CellInput, CellOutput, CellTx, OutPoint, ResolvedHeader, Script, VersionedEnvelope, VmSerializable};

fn create_sample_tx() -> CellTx {
    let lock_script = Script::new([0x00; 32], 0, vec![0xAB; 20]);
    let type_script = Script::new([0x11; 32], 1, vec![0xCD; 10]);

    let outputs: Vec<CellOutput> = (0..10)
        .map(|i| CellOutput {
            lock: lock_script.clone(),
            type_: if i % 2 == 0 { Some(type_script.clone()) } else { None },
            capacity: 1000 + i as u64,
        })
        .collect();

    let inputs: Vec<CellInput> = (0..5).map(|i| CellInput::new(OutPoint::new([i as u8; 32], i as u32), i as u64)).collect();

    CellTx::new(inputs, vec![], outputs, vec![vec![0x11; 100]; 10], vec![vec![0x22; 65]; 5]).expect("valid transaction")
}

fn create_sample_resolved_header() -> ResolvedHeader {
    ResolvedHeader {
        hash: [0xAA; 32],
        version: 1,
        parents_by_level: vec![vec![[0xBB; 32]; 3], vec![[0xCC; 32]; 2]],
        hash_merkle_root: [0xDD; 32],
        accepted_id_merkle_root: [0xEE; 32],
        cell_commitment: [0xFF; 32],
        cell_root: [0x11; 32],
        segment_root: [0x22; 32],
        timestamp: 1234567890,
        bits: 0x1d00ffff,
        nonce: 42,
        daa_score: 1000,
        blue_work: [0x33; 24],
        blue_score: 500,
        pruning_point: [0x44; 32],
    }
}

fn create_sample_resolved_cell_meta() -> ResolvedCellMeta {
    ResolvedCellMeta {
        cell_output: CellOutput {
            lock: Script::new([0xAA; 32], 0, vec![0xBB; 20]),
            type_: Some(Script::new([0xCC; 32], 1, vec![0xDD; 10])),
            capacity: 1000,
        },
        out_point: OutPoint::new([0xEE; 32], 0),
        transaction_info: Some(TransactionInfo { tx_hash: [0xFF; 32], daa_score: 100, block_hash: [0x11; 32], is_cellbase: false }),
        data_bytes: 100,
        mem_cell_data: Some(vec![0x22; 100]),
        mem_cell_data_hash: Some([0x33; 32]),
    }
}

fn bench_celltx_serialization(c: &mut Criterion) {
    let tx = create_sample_tx();
    let size = std::mem::size_of_val(&tx);

    let mut group = c.benchmark_group("celltx_serialization");
    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&tx)).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("versioned_envelope_create", |b| {
        b.iter(|| {
            let envelope = VersionedEnvelope::new(black_box(&tx)).unwrap();
            black_box(envelope);
        })
    });

    // Pre-serialize for deserialization benchmarks
    let tx_bytes = borsh::to_vec(&tx).unwrap();
    let envelope = VersionedEnvelope::new(&tx).unwrap();
    let envelope_bytes = borsh::to_vec(&envelope).unwrap();

    group.bench_function("borsh_deserialize", |b| {
        b.iter(|| {
            let tx: CellTx = borsh::from_slice(black_box(&tx_bytes)).unwrap();
            black_box(tx);
        })
    });

    group.bench_function("versioned_envelope_parse", |b| {
        b.iter(|| {
            let envelope: VersionedEnvelope<CellTx> = borsh::from_slice(black_box(&envelope_bytes)).unwrap();
            let tx = envelope.parse().unwrap();
            black_box(tx);
        })
    });

    group.finish();
}

fn bench_resolved_header_serialization(c: &mut Criterion) {
    let header = create_sample_resolved_header();
    let size = std::mem::size_of_val(&header);

    let mut group = c.benchmark_group("resolved_header_serialization");
    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&header)).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("vm_serializable_to_bytes", |b| {
        b.iter(|| {
            let bytes = header.to_vm_bytes();
            black_box(bytes);
        })
    });

    // Pre-serialize for deserialization benchmarks
    let header_bytes = borsh::to_vec(&header).unwrap();
    let vm_bytes = header.to_vm_bytes();

    group.bench_function("borsh_deserialize", |b| {
        b.iter(|| {
            let header: ResolvedHeader = borsh::from_slice(black_box(&header_bytes)).unwrap();
            black_box(header);
        })
    });

    group.bench_function("vm_serializable_from_bytes", |b| {
        b.iter(|| {
            let header = ResolvedHeader::from_vm_bytes(black_box(&vm_bytes)).unwrap();
            black_box(header);
        })
    });

    group.finish();
}

fn bench_resolved_cell_meta_serialization(c: &mut Criterion) {
    let cell = create_sample_resolved_cell_meta();
    let size = std::mem::size_of_val(&cell);

    let mut group = c.benchmark_group("resolved_cell_meta_serialization");
    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("borsh_serialize", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&cell)).unwrap();
            black_box(bytes);
        })
    });

    group.bench_function("versioned_envelope_create", |b| {
        b.iter(|| {
            let envelope = VersionedEnvelope::new(black_box(&cell)).unwrap();
            black_box(envelope);
        })
    });

    // Pre-serialize for deserialization benchmarks
    let cell_bytes = borsh::to_vec(&cell).unwrap();
    let envelope = VersionedEnvelope::new(&cell).unwrap();
    let envelope_bytes = borsh::to_vec(&envelope).unwrap();

    group.bench_function("borsh_deserialize", |b| {
        b.iter(|| {
            let cell: ResolvedCellMeta = borsh::from_slice(black_box(&cell_bytes)).unwrap();
            black_box(cell);
        })
    });

    group.bench_function("versioned_envelope_parse", |b| {
        b.iter(|| {
            let envelope: VersionedEnvelope<ResolvedCellMeta> = borsh::from_slice(black_box(&envelope_bytes)).unwrap();
            let cell = envelope.parse().unwrap();
            black_box(cell);
        })
    });

    group.finish();
}

fn bench_versioned_envelope_overhead(c: &mut Criterion) {
    let tx = create_sample_tx();

    let mut group = c.benchmark_group("versioned_envelope_overhead");

    group.bench_function("raw_borsh", |b| {
        b.iter(|| {
            let bytes = borsh::to_vec(black_box(&tx)).unwrap();
            let _: CellTx = borsh::from_slice(&bytes).unwrap();
        })
    });

    group.bench_function("with_versioned_envelope", |b| {
        b.iter(|| {
            let envelope = VersionedEnvelope::new(black_box(&tx)).unwrap();
            let bytes = borsh::to_vec(&envelope).unwrap();
            let envelope: VersionedEnvelope<CellTx> = borsh::from_slice(&bytes).unwrap();
            let _ = envelope.parse().unwrap();
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_celltx_serialization,
    bench_resolved_header_serialization,
    bench_resolved_cell_meta_serialization,
    bench_versioned_envelope_overhead
);
criterion_main!(benches);
