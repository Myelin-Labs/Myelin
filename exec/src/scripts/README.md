# Spora Standard Scripts

This directory contains standard lock and type scripts for Spora.

## Lock Scripts

### 1. Time Lock Scripts (CKB-VM Based) ⭐ NEW

**Replaces**: Legacy `OP_CHECKLOCKTIMEVERIFY` and `OP_CHECKSEQUENCEVERIFY`

**Files**:
- `timelock.rs` - Rust helper module for constructing time lock Scripts
- `timelock_absolute.c` - C source for absolute timestamp lock, reading target args via `LOAD_SCRIPT`
- `timelock_relative.c` - C source for relative DAA score lock, reading target args via `LOAD_SCRIPT`

**Features**:
- Absolute timestamp lock (Unix timestamp)
- Relative DAA score lock (block count)
- Absolute DAA score lock
- Relative timestamp lock

**Usage**:
```rust
use spora_exec::scripts::timelock;

// Absolute timestamp lock (e.g., lock until 2025-01-01)
let target_timestamp = 1735689600u64;
let lock = timelock::absolute_timestamp_lock(target_timestamp);

// Create input with proper `since` encoding
let since = timelock::encode_absolute_timestamp_since(target_timestamp);
let input = CellInput::new(outpoint, since);
```

**Since Encoding**:
```rust
// Absolute timestamp
let since = timelock::encode_absolute_timestamp_since(timestamp);
// Bits: 01xxxxxxxx (bit63=0, bit62=1, bits0-55=timestamp)

// Relative DAA score
let since = timelock::encode_relative_daa_since(delta_blocks);
// Bits: 10xxxxxxxx (bit63=1, bit62=0, bits0-55=delta)
```

**Migration from Legacy**:

| Legacy Script | Cell Model Replacement |
|--------------|------------------------|
| `pay_to_pub_key_with_lock_time` | `timelock::absolute_timestamp_lock` + proper `since` |
| `htlc_script` | Custom CKB-VM script with `since` + signature verification |
| `OP_CHECKSEQUENCEVERIFY` | `timelock::relative_daa_lock` + `encode_relative_daa_since` |

### 2. Time Lock Fixtures (CKB-VM)

**Absolute Timestamp Lock**
- Source: `fixtures/timelock_absolute.rs`
- Binary: `fixtures/timelock_absolute.elf`
- Target: 2025-01-01 00:00:00 UTC (1735689600)
- Verifies: `since` >= target with bit63=0, bit62=1

**Relative DAA Lock**
- Source: `fixtures/timelock_relative.rs`
- Binary: `fixtures/timelock_relative.elf`
- Target: 100 blocks
- Verifies: `since` >= 100 with bit63=1, bit62=0

**Usage:**
```rust
use spora_exec::scripts::{timelock_absolute_code_hash, TIMELOCK_ABSOLUTE_SCRIPT};
use spora_exec::scripts::timelock::encode_absolute_timestamp_since;

let code_hash = timelock_absolute_code_hash();
let since = encode_absolute_timestamp_since(1735689600);
```

### 3. Signature Hash Fixture (CKB-VM)

**Source**: `fixtures/load_ecdsa_signature_hash.rs`  
**Binary**: `fixtures/load_ecdsa_signature_hash.elf`

**Purpose**:
- Exercises VM syscall `3004`
- Loads the canonical per-input ECDSA sighash for the first group input
- Compares it against an expected digest carried in witness 0

**Witness Format**:
- `[0..32]`: expected canonical ECDSA sighash
- `[32]`: sighash type byte passed to syscall `3004`

**Usage**:
```rust
use spora_exec::scripts::{load_ecdsa_signature_hash_code_hash, LOAD_ECDSA_SIGNATURE_HASH_SCRIPT};

let code_hash = load_ecdsa_signature_hash_code_hash();
let expected_digest = [0u8; 32];
let witness = expected_digest.into_iter().chain([0x01]).collect::<Vec<_>>();
```

### 4. HTLC (Hash Time Locked Contract)

**Source**: `fixtures/htlc.rs`
**Binary**: `fixtures/htlc.elf` (768KB)

**Features**:
- Two spending paths:
  1. Recipient path: secret preimage + signature
  2. Sender timeout path: signature after timelock expires
- Supports all four lock types (absolute/relative DAA/timestamp)
- Uses blake3 for secret hash verification
- Uses a deterministic fixture-only signature rule in tests, not real secp256k1 verification

**Script Args** (105 bytes):
- `[0..32]`: secret_hash (blake3)
- `[32..64]`: recipient_pubkey (32 bytes)
- `[64..96]`: sender_pubkey (32 bytes)
- `[96]`: lock_type (0-3)
- `[97..105]`: lock_value (u64)

**Witness Format**:
- Recipient: `<signature (64)> <secret (32)> <0x01>`
- Sender: `<signature (64)> <0x00>`

**Usage:**
```rust
use spora_exec::scripts::{htlc_code_hash, HTLC_SCRIPT};

let code_hash = htlc_code_hash();
let args = build_htlc_args(secret_hash, recipient_pubkey, sender_pubkey, lock_type, lock_value);
let lock = Script::new(code_hash, 0, args);
```

### 5. Always Success (Testing Only)

**Code**: real RISC-V ELF fixture

Source file:
`fixtures/always_success.rs`

**Usage**:
```rust
use spora_exec::scripts::{ALWAYS_SUCCESS_SCRIPT, always_success_code_hash};

let lock = Script {
    code_hash: always_success_code_hash(),
    hash_type: 0,  // Data hash type
    args: vec![],
};
```

### 6. Secp256k1 + Blake3 Lock (Production-Ready)

**File**: `secp256k1_blake3_lock.c`

**Functionality**:
- Verifies secp256k1 signatures using **blake3** for hashing (Spora-specific!)
- Args: pubkey hash (20 bytes, blake3 of pubkey)
- Witness: recoverable signature (65 bytes, r + s + v), optionally followed by 1-byte sighash flag
- Loads the canonical per-input ECDSA sighash from VM syscall `3004`
- Verifies every witness in the current input group against syscall `3002`
- Fail-closed semantics: returns 1 (failure) on any error path

**Security Features**:
- Strict `LOAD_SCRIPT` args boundary validation (`args_len == 20` + out-of-bounds rejection)
- Low-S signature enforcement (syscall 3002 rejects non-canonical high-S signatures)
- Canonical ECDSA sighash binding (per-input via syscall 3004)

**Build**:
```bash
# Using RISC-V GNU toolchain
riscv64-unknown-elf-gcc -O3 -nostdlib -nostartfiles \
    -fno-builtin-printf -fno-builtin-memcmp \
    -Wl,-Ttext=0x0 \
    -o secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.c

riscv64-unknown-elf-objcopy -O binary \
    secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.bin

# Get code hash
cargo run -p spora-exec --example fixture_hashes -- secp256k1_blake3_lock.bin
```

**Verification**:
- Script-level regression tests: `cargo test -p spora-exec --lib`
- Consensus integration: `cargo test -p spora-consensus --features vm --lib`
- Syscall 3002 tests: `exec/src/vm/syscalls/secp256k1_verify.rs` (5 test cases covering valid signatures, tampered signatures, invalid recovery ids, high-S attacks)

**Build**:
```bash
# Install RISC-V toolchain
# https://github.com/riscv-collab/riscv-gnu-toolchain

# Compile
riscv64-unknown-elf-gcc -O3 -nostdlib -nostartfiles \
    -fno-builtin-printf -fno-builtin-memcmp \
    -Wl,-Ttext=0x0 \
    -o secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.c

# Extract binary
riscv64-unknown-elf-objcopy -O binary \
    secp256k1_blake3_lock.elf \
    secp256k1_blake3_lock.bin

# Get code hash (for use in transactions)
blake3sum secp256k1_blake3_lock.bin

# If blake3sum/b3sum is unavailable, use workspace helper
cargo run -p spora-exec --example fixture_hashes -- secp256k1_blake3_lock.bin

# For fixture ELFs, use the batch builder (also writes CODE_HASHES.blake3)
bash exec/src/scripts/fixtures/build_fixtures.sh
```

**Usage**:
```rust
// In transaction
let pubkey = /* secp256k1 public key (33 bytes compressed) */;
let pubkey_hash = &blake3::hash(&pubkey).as_bytes()[0..20];

let lock = Script {
    code_hash: blake3::hash(&secp256k1_lock_binary).into(),
    hash_type: 0,
    args: pubkey_hash.to_vec(),
};

let output = CellOutput {
    capacity: 10000,
    lock,
    type_: None,
};
```

## Key Differences from CKB

| Feature | CKB | Spora |
|---------|-----|-------|
| Sighash | blake2b | **blake3** |
| VM syscalls | 9 standard | 9 standard + **blake3_hash** |
| Binary format | Same RISC-V | Same RISC-V ✅ |

**Important**: CKB scripts need to be **recompiled** for Spora because:
1. Sighash uses blake3 (not blake2b)
2. Tx hash uses blake3
3. Script hash uses blake3

But the **logic** can be reused!

## Type Scripts

### 1. Capacity Type (Future)

Ensures capacity conservation:
```
sum(inputs.capacity) == sum(outputs.capacity)
```

### 2. UDT (User Defined Token)

Standard token contract (CKB-compatible logic).

---

## Development Guide

### Testing Scripts

Use the always-success script for initial testing:

```rust
#[test]
fn test_always_success() {
    use spora_exec::vm::{TransactionScriptVerifier, SimpleDataProvider};
    
    // Create provider with always-success script
    let mut provider = SimpleDataProvider::new();
    provider.add_script(
        always_success_code_hash(),
        ALWAYS_SUCCESS_SCRIPT.to_vec(),
    );
    
    let input_out_point = OutPoint::new([0x11; 32], 0);
    provider.add_cell(
        input_out_point.tx_hash,
        input_out_point.index,
        ResolvedCell {
            cell_output: CellOutput {
                capacity: 1000,
                lock: Script {
                    code_hash: always_success_code_hash(),
                    hash_type: 0,
                    args: vec![],
                },
                type_: None,
            },
            data: Some(vec![]),
        },
    );

    // Create transaction spending an input with the always-success lock
    let tx = CellTx {
        inputs: vec![CellInput::new(input_out_point, 0)],
        deps: vec![],
        header_deps: vec![],
        outputs: vec![
            CellOutput {
                capacity: 1000,
                lock: Script {
                    code_hash: always_success_code_hash(),
                    hash_type: 0,
                    args: vec![],
                },
                type_: None,
            }
        ],
        // ...
    };
    
    // Verify
    let verifier = TransactionScriptVerifier::new(
        Arc::new(tx),
        Arc::new(provider),
    );
    
    assert!(verifier.verify().is_ok());
}
```

### Building Custom Scripts

1. Write script in C (using Spora syscalls)
2. Compile to RISC-V binary
3. Compute blake3 code hash
4. Deploy as cell data in genesis or via transaction
5. Reference in lock/type scripts

---

**Last Updated**: 2025-10-22  
**See Also**: `../vm/syscalls/` for syscall implementations
