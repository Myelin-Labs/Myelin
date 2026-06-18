// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Secp256k1 + Blake3 Lock Script
// Compile to RISC-V binary for use in CKB-VM
//
// Build:
//   riscv64-unknown-elf-gcc -O3 -nostdlib -nostartfiles \
//     -fno-builtin-printf -fno-builtin-memcmp \
//     -o secp256k1_blake3_lock secp256k1_blake3_lock.c

#include <stdint.h>
#include <stddef.h>

// ============================================================================
// Myelin Syscall Definitions
// ============================================================================

#define LOAD_SCRIPT_HASH_SYSCALL 2062
#define LOAD_CELL_SYSCALL        2071
#define LOAD_INPUT_SYSCALL       2073
#define LOAD_WITNESS_SYSCALL     2074
#define LOAD_SCRIPT_SYSCALL      2075
#define BLAKE3_HASH_SYSCALL      3001  // ← Myelin extension!
#define SECP256K1_VERIFY_SYSCALL 3002  // ← Myelin extension!
#define LOAD_ECDSA_SIGHASH_SYSCALL 3004 // ← Myelin extension!

#define SUCCESS              0
#define INDEX_OUT_OF_BOUND   1
#define ITEM_MISSING         2
#define LENGTH_NOT_ENOUGH    3

// Source types
#define SOURCE_INPUT         0x01
#define SOURCE_OUTPUT        0x02
#define SOURCE_GROUP_INPUT   0x0100
#define SOURCE_GROUP_OUTPUT  0x0200

#define SIG_HASH_ALL         0x01

// ============================================================================
// Syscall Wrappers
// ============================================================================

static inline int syscall(
    uint64_t n,
    uint64_t a0,
    uint64_t a1,
    uint64_t a2,
    uint64_t a3,
    uint64_t a4,
    uint64_t a5
) {
    register uint64_t _a0 asm("a0") = a0;
    register uint64_t _a1 asm("a1") = a1;
    register uint64_t _a2 asm("a2") = a2;
    register uint64_t _a3 asm("a3") = a3;
    register uint64_t _a4 asm("a4") = a4;
    register uint64_t _a5 asm("a5") = a5;
    register uint64_t _a7 asm("a7") = n;
    
    asm volatile (
        "ecall"
        : "+r"(_a0)
        : "r"(_a1), "r"(_a2), "r"(_a3), "r"(_a4), "r"(_a5), "r"(_a7)
        : "memory"
    );
    
    return _a0;
}

// Load witness data
static inline int load_witness(
    uint8_t* buf,
    uint64_t* len,
    size_t offset,
    size_t index,
    size_t source
) {
    return syscall(LOAD_WITNESS_SYSCALL, (uint64_t)buf, (uint64_t)len, offset, index, source, 0);
}

// Load canonical ECDSA signature hash (32 bytes)
static inline int load_ecdsa_sighash(
    uint8_t* buf,
    uint64_t* len,
    size_t offset,
    size_t index,
    size_t source,
    uint8_t hash_type
) {
    return syscall(LOAD_ECDSA_SIGHASH_SYSCALL, (uint64_t)buf, (uint64_t)len, offset, index, source, hash_type);
}

// Load script args
static inline int load_script(
    uint8_t* buf,
    uint64_t* len,
    size_t offset
) {
    return syscall(LOAD_SCRIPT_SYSCALL, (uint64_t)buf, (uint64_t)len, offset, 0, 0, 0);
}

// Blake3 hash (Myelin-specific!)
static inline int blake3_hash(
    uint8_t* output,
    const uint8_t* input,
    size_t input_len
) {
    uint64_t output_len = 32;
    return syscall(BLAKE3_HASH_SYSCALL, (uint64_t)output, (uint64_t)&output_len, (uint64_t)input, input_len, 0, 0);
}

// Secp256k1 recover + verify against 20-byte blake3 pubkey hash
static inline int secp256k1_verify(
    const uint8_t* pubkey_hash,
    const uint8_t* signature,
    const uint8_t* message_hash
) {
    return syscall(
        SECP256K1_VERIFY_SYSCALL,
        (uint64_t)pubkey_hash,
        (uint64_t)signature,
        (uint64_t)message_hash,
        0,
        0,
        0
    );
}

static uint64_t read_u32_le(const uint8_t* buf) {
    uint64_t value = 0;
    for (int i = 0; i < 4; i++) {
        value |= ((uint64_t)buf[i]) << (8 * i);
    }
    return value;
}

// ============================================================================
// Secp256k1 Signature Verification
// ============================================================================

int verify_secp256k1_signature(
    const uint8_t* pubkey_hash,   // 20 bytes (blake3(pubkey)[0..20])
    const uint8_t* signature,     // 65 bytes (r + s + v)
    const uint8_t* message_hash   // 32 bytes
) {
    return secp256k1_verify(pubkey_hash, signature, message_hash);
}

// Read 20-byte pubkey hash from canonical Script serialization:
// code_hash(32) || hash_type(1) || args_len(u32 LE) || args
static int get_pubkey_hash(uint8_t out_pubkey_hash[20]) {
    uint8_t script_buf[64];
    uint64_t script_len = sizeof(script_buf);
    int ret = load_script(script_buf, &script_len, 0);
    if (ret != SUCCESS || script_len < 57) {
        return 1;
    }

    uint64_t args_len = read_u32_le(script_buf + 33);
    if (args_len != 20 || (37 + args_len) > script_len) {
        return 1;
    }

    for (int i = 0; i < 20; i++) {
        out_pubkey_hash[i] = script_buf[37 + i];
    }
    return 0;
}

// ============================================================================
// Lock Script Main Logic
// ============================================================================

int main() {
    uint8_t pubkey_hash[20];

    // 1. Load and validate script args (must be exactly 20-byte pubkey hash)
    if (get_pubkey_hash(pubkey_hash) != 0) {
        return 1;
    }

    // 2. Verify every input in the current lock-script group against the
    //    canonical per-input ECDSA signature hash exposed by syscall 3004.
    size_t group_index = 0;
    int saw_group_witness = 0;
    for (;;) {
        uint8_t witness[256];
        uint64_t witness_len = sizeof(witness);
        int ret = load_witness(witness, &witness_len, 0, group_index, SOURCE_GROUP_INPUT);
        if (ret == INDEX_OUT_OF_BOUND) {
            break;
        }
        if (ret != SUCCESS) {
            return 1;
        }

        saw_group_witness = 1;
        if (witness_len != 65 && witness_len != 66) {
            return 1;
        }

        uint8_t hash_type = (witness_len == 66) ? witness[65] : SIG_HASH_ALL;
        uint8_t sighash[32];
        uint64_t sighash_len = sizeof(sighash);
        ret = load_ecdsa_sighash(sighash, &sighash_len, 0, group_index, SOURCE_GROUP_INPUT, hash_type);
        if (ret != SUCCESS || sighash_len != 32) {
            return 1;
        }

        ret = verify_secp256k1_signature(pubkey_hash, witness, sighash);
        if (ret != 0) {
            return 1;
        }

        group_index++;
    }

    return saw_group_witness ? 0 : 1;
}
