"""Canonical fixture bytes, CKB transactions and public test-key signing.

No compatibility formats. The CKB program, rather than these host builders,
judges game transitions. Acceptance submits mutated builders to the actual VM.
"""
import copy
import hashlib
import hmac
import json
import struct
from cryptography.hazmat.primitives.asymmetric import ec

C = 100_000_000
STAKE = 1000 * C
TIMEOUT = 8
FEE = 1_000_000
N = 365
SCHEMA = b"myelin:microgame:state365:program-schema-vm-game-players-stake-timeout-seq-phase-counter-turn-delta-claimed-root-post-da:v1"
VM = b"ckb:Data2:VM2:riscv64imc:strict-no-spawn:no-oracle:10000000-cycles:v1"
ROOT = b"myelin:microgame:logical-state:v1"
MOVE = b"myelin:microgame:move-da:v1"
ALWAYS = "0x28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5"
ORDER = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141
KEYS = [bytes([0x11]) * 32, bytes([0x22]) * 32]  # PUBLIC test credentials


def raw(value):
    assert value.startswith("0x") and len(value) % 2 == 0
    return bytes.fromhex(value[2:])


def hx(value):
    return "0x" + bytes(value).hex()


def h(value):
    return hashlib.blake2b(bytes(value), digest_size=32, person=b"ckb-default-hash").digest()


def u32(n):
    return struct.pack("<I", n)


def u64(n):
    return struct.pack("<Q", n)


def table(fields):
    offset = 4 * (len(fields) + 1)
    offsets = []
    for field in fields:
        offsets.append(offset)
        offset += len(field)
    return u32(offset) + b"".join(u32(n) for n in offsets) + b"".join(fields)


def dynvec(fields):
    return table(fields) if fields else u32(4)


def fixvec(fields):
    return u32(len(fields)) + b"".join(fields)


def blob(value):
    return u32(len(value)) + bytes(value)


def script(value):
    return table([raw(value["code_hash"]), bytes([{"data": 0, "type": 1, "data1": 2, "data2": 4}[value["hash_type"]]]), blob(raw(value["args"]))])


def point(value):
    return raw(value["tx_hash"]) + u32(int(value["index"], 16))


def cell_input(value):
    return u64(int(value["since"], 16)) + point(value["previous_output"])


def cell_output(value):
    return table([u64(int(value["capacity"], 16)), script(value["lock"]), script(value["type"]) if value.get("type") else b""])


def raw_tx(value):
    return table([u32(int(value["version"], 16)),
                  fixvec([point(dep["out_point"]) + bytes([1 if dep["dep_type"] == "dep_group" else 0]) for dep in value["cell_deps"]]),
                  fixvec([raw(dep) for dep in value["header_deps"]]), fixvec([cell_input(item) for item in value["inputs"]]),
                  dynvec([cell_output(item) for item in value["outputs"]]), dynvec([blob(raw(item)) for item in value["outputs_data"]])])


def transaction(value):
    return table([raw_tx(value), dynvec([blob(raw(item)) for item in value["witnesses"]])])


def witness(lock=None, action=None):
    return table([blob(lock) if lock is not None else b"", blob(bytes([action])) if action is not None else b"", b""])


def public(secret):
    p = ec.derive_private_key(int.from_bytes(secret, "big"), ec.SECP256K1()).public_key().public_numbers()
    return bytes([2 + p.y % 2]) + p.x.to_bytes(32, "big")


def sign(secret, message):
    # RFC6979 deterministic nonce, low-S recoverable CKB signature. The real
    # system sighash script independently verifies every generated signature.
    z = int.from_bytes(message, "big") % ORDER
    seed = secret + z.to_bytes(32, "big")
    v, k = bytes([1]) * 32, bytes(32)
    k = hmac.new(k, v + b"\x00" + seed, hashlib.sha256).digest()
    v = hmac.new(k, v, hashlib.sha256).digest()
    k = hmac.new(k, v + b"\x01" + seed, hashlib.sha256).digest()
    v = hmac.new(k, v, hashlib.sha256).digest()
    while True:
        v = hmac.new(k, v, hashlib.sha256).digest()
        nonce = int.from_bytes(v, "big")
        if 0 < nonce < ORDER:
            p = ec.derive_private_key(nonce, ec.SECP256K1()).public_key().public_numbers()
            r = p.x % ORDER
            s = pow(nonce, -1, ORDER) * (z + r * int.from_bytes(secret, "big")) % ORDER
            if r and s:
                recovery = p.y % 2 + (2 if p.x >= ORDER else 0)
                if s > ORDER // 2:
                    s, recovery = ORDER - s, recovery ^ 1
                return r.to_bytes(32, "big") + s.to_bytes(32, "big") + bytes([recovery])
        k = hmac.new(k, v + b"\x00", hashlib.sha256).digest()
        v = hmac.new(k, v, hashlib.sha256).digest()


def output(capacity, lock, type_=None):
    return {"capacity": hex(capacity), "lock": copy.deepcopy(lock), "type": copy.deepcopy(type_)}


def new_tx(inputs, outputs, data, deps, action=None, since=0):
    return {"version": "0x0", "cell_deps": copy.deepcopy(deps), "header_deps": [],
            "inputs": [{"since": hex(since if i == 0 else 0), "previous_output": item["out_point"]} for i, item in enumerate(inputs)],
            "outputs": outputs, "outputs_data": [hx(item) for item in data],
            "witnesses": [hx(witness(action=action if i == 0 else None)) for i in range(len(inputs))]}


def sign_tx(tx, inputs, player_locks):
    tx_hash = h(raw_tx(tx))
    for secret, lock in zip(KEYS, player_locks):
        group = [i for i, item in enumerate(inputs) if item["output"]["lock"] == lock]
        if not group:
            continue
        first = group[0]
        action = None
        original = raw(tx["witnesses"][first])
        # Preserve the type-script action when a participant is also input zero.
        offset = int.from_bytes(original[8:12], "little")
        end = int.from_bytes(original[12:16], "little")
        if end > offset:
            action = original[offset + 4]
        placeholder = witness(lock=bytes(65), action=action)
        digest = hashlib.blake2b(digest_size=32, person=b"ckb-default-hash")
        digest.update(tx_hash)
        for index in group:
            w = placeholder if index == first else raw(tx["witnesses"][index])
            digest.update(u64(len(w)))
            digest.update(w)
        for w in tx["witnesses"][len(inputs):]:
            w = raw(w)
            digest.update(u64(len(w)))
            digest.update(w)
        tx["witnesses"][first] = hx(witness(lock=sign(secret, digest.digest()), action=action))
    return tx


def root(state):
    return h(ROOT + state[4:212] + state[212:220] + bytes([int(state[220] == 2), state[221], state[222]]))


def da_preimage(state):
    return MOVE + state[4:212] + state[212:220] + state[225:257] + state[222:225] + state[257:289]


def initial(program, game_id, locks):
    state = bytearray(N)
    state[:4] = b"MGC1"
    state[4:36], state[36:68], state[68:100], state[100:132] = raw(program), h(SCHEMA), h(VM), game_id
    state[132:164], state[164:196] = [h(script(lock)) for lock in locks]
    state[196:204], state[204:212] = u64(STAKE), u64(TIMEOUT)
    state[225:257] = root(state)
    return bytes(state)


def moved(state):
    result = bytearray(state)
    count = state[221] + state[223]
    assert count <= 5
    result[212:220] = u64(int.from_bytes(state[212:220], "little") + 1)
    result[220], result[221], result[222] = int(count == 5) * 2, count, state[222] if count == 5 else state[222] ^ 1
    result[223:225], result[257:321] = bytes(2), bytes(64)
    result[225:257] = root(result)
    return bytes(result)


def propose(state, delta, invalid_counter=False, invalid_root=False):
    result = bytearray(state)
    result[220], result[223] = 1, delta
    next_state = moved(result)
    result[224], result[257:289] = next_state[221], next_state[225:257]
    if invalid_counter:
        result[224] ^= 1
    if invalid_root:
        result[257] ^= 1
    result[289:321] = h(da_preimage(result))
    return bytes(result)


def fraud(state):
    result = bytearray(state)
    result[220], result[222] = 2, state[222] ^ 1
    result[212:220] = u64(int.from_bytes(state[212:220], "little") + 1)
    result[223:225], result[257:321] = bytes(2), bytes(64)
    result[225:257] = root(result)
    return bytes(result)


def proof(tx, cells, program, genesis):
    return {"format": "myelin-ckb-microgame-replay-v1", "genesis": genesis, "program_hash": program,
            "transaction": hx(transaction(tx)), "cells": [{"out_point": hx(point(item["out_point"])),
                                                           "output": hx(cell_output(item["output"])), "data": item["data"]} for item in cells]}


def write_json(path, value):
    path.write_text(json.dumps(value, sort_keys=True, indent=2) + "\n")
