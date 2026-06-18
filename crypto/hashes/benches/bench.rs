use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rand::{thread_rng, Rng, RngCore};
use spora_hashes::*;
use std::any::type_name;

fn test_bytes_hasher<H: Hasher>(c: &mut Criterion) {
    let mut rng = thread_rng();
    let buf: [u8; 32] = rng.gen();
    c.bench_function(&format!("32 bytes: {}", type_name::<H>()), |b| {
        b.iter(|| {
            let buf = black_box(buf);
            black_box(H::hash(buf));
        })
    });

    let mut buf = vec![0u8; 1024];
    rng.fill_bytes(&mut buf);
    c.bench_function(&format!("1024 bytes: {}", type_name::<H>()), |b| {
        b.iter(|| {
            black_box(buf.as_mut_slice());
            black_box(H::hash(&buf));
        })
    });
}

fn bench_hashers(c: &mut Criterion) {
    test_bytes_hasher::<CellTxHash>(c);
    test_bytes_hasher::<CellTxId>(c);
    test_bytes_hasher::<CellTxSigningHash>(c);
    test_bytes_hasher::<BlockHash>(c);
    test_bytes_hasher::<MerkleBranchHash>(c);
    test_bytes_hasher::<MuHashElementHash>(c);
    test_bytes_hasher::<MuHashFinalizeHash>(c);
    test_bytes_hasher::<CellTxSigningHashEcdsa>(c);
}

criterion_group!(benches, bench_hashers);
criterion_main!(benches);
