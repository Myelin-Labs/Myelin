# Myelin Standard Scripts

This directory contains standard lock and type scripts for Myelin.

## Lock Scripts

### 1. Time Lock Scripts (CKB-VM Based)

**Replaces**: Legacy `OP_CHECKLOCKTIMEVERIFY` and `OP_CHECKSEQUENCEVERIFY`

**Files**:
- `timelock.rs` - Rust helper module for constructing time lock Scripts
- `timelock_absolute.c` - C source for absolute timestamp lock, reading target args via `LOAD_SCRIPT`
- `timelock_relative.c` - C source for relative block number lock, reading target args via `LOAD_SCRIPT`

**Features**:
- Absolute timestamp lock (Unix timestamp)
- Relative block number lock (block count)
- Absolute block number lock
- Relative timestamp lock

**Usage**:
```rust
use myelin_exec::scripts::timelock;

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

// Relative block number
let since = timelock::encode_relative_block_number_since(delta_blocks);
// Bits: 10xxxxxxxx (bit63=1, bit62=0, bits0-55=delta)
```

**Migration from Legacy**:

| Legacy Script | Cell Model Replacement |
|--------------|------------------------|
| `pay_to_pub_key_with_lock_time` | `timelock::absolute_timestamp_lock` + proper `since` |
| `htlc_script` | Custom CKB-VM script with `since` + signature verification |
| `OP_CHECKSEQUENCEVERIFY` | `timelock::relative_block_number_lock` + `encode_relative_block_number_since` |

### 2. Time Lock Fixtures (CKB-VM)

**Absolute Timestamp Lock**
- Source: `fixtures/timelock_absolute.rs`
- Binary: `fixtures/timelock_absolute.elf`
- Target: 2025-01-01 00:00:00 UTC (1735689600)
- Verifies: `since` >= target with bit63=0, bit62=1

**Relative block number Lock**
- Source: `fixtures/timelock_relative.rs`
- Binary: `fixtures/timelock_relative.elf`
- Target: 100 blocks
- Verifies: `since` >= 100 with bit63=1, bit62=0

**Usage:**
```rust
use myelin_exec::scripts::{timelock_absolute_code_hash, TIMELOCK_ABSOLUTE_SCRIPT};
use myelin_exec::scripts::timelock::encode_absolute_timestamp_since;

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
use myelin_exec::scripts::{load_ecdsa_signature_hash_code_hash, LOAD_ECDSA_SIGNATURE_HASH_SCRIPT};

let code_hash = load_ecdsa_signature_hash_code_hash();
let expected_digest = [0u8; 32];
let witness = expected_digest.into_iter().chain([0x01]).collect::<Vec<_>>();
```

### 5. Always Success (Testing Only)

**Code**: real RISC-V ELF fixture

Source file:
`fixtures/always_success.rs`

**Usage**:
```rust
use myelin_exec::scripts::{ALWAYS_SUCCESS_SCRIPT, always_success_code_hash};

let lock = Script {
    code_hash: always_success_code_hash(),
    hash_type: 0,  // Data hash type
    args: vec![],
};
```

The obsolete custom-ABI HTLC and secp256k1 lock fixtures have been removed.
Fixtures use the canonical CKB group-source encoding. Signature syscalls are
covered by their VM tests; these test scripts do not constitute deployable locks.

## Key Differences from CKB

| Feature | CKB | Myelin |
|---------|-----|-------|
| Sighash | blake2b | **blake3** |
| VM syscalls | 9 standard | 9 standard + **blake3_hash** |
| Binary format | Same RISC-V | Same RISC-V ✅ |

**Important**: CKB scripts need to be **recompiled** for Myelin because:
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
    use myelin_exec::vm::{TransactionScriptVerifier, SimpleDataProvider};
    
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

1. Write script in C (using Myelin syscalls)
2. Compile to RISC-V binary
3. Compute blake3 code hash
4. Deploy as cell data in genesis or via transaction
5. Reference in lock/type scripts

---

**Last Updated**: 2025-10-22  
**See Also**: `../vm/syscalls/` for syscall implementations
