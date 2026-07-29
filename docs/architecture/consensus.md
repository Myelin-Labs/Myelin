# Closed-validator finality

Myelin finalises finite-session `MyelinBlock`s with one of three configured engines:

- `static-closed-committee` — one weighted commit certificate;
- `proof-of-authority` — one height-bound seal from the deterministically
  scheduled authority;
- `tendermint` — deterministic proposer selection plus signed proposal,
  prevote, precommit, locking, nil votes, round changes, and a final
  precommit decision certificate.

All use secp256k1 Schnorr signatures with distinct domains. They are
closed-validator mechanisms, not permissionless consensus protocols.

| Engine | Finality proof | Safety assumption | Typical use |
| --- | --- | --- | --- |
| Static committee | Weighted quorum certificate | Configured quorum does not sign conflicting blocks | Small controlled sessions |
| Proof of authority | One scheduled authority seal | Scheduled key is honest and available at its height | Simple operator-led sessions |
| Tendermint | Greater-than-two-thirds precommit decision | Less than one third of voting power is Byzantine | Multi-validator sessions needing BFT rounds |

The public dispatch type is `FinalityProof`. Its three variants are structurally
different, so a PoA seal cannot be silently interpreted as a committee
certificate or Tendermint decision.

## Proof-of-authority configuration

Authority order is consensus-critical. For `N` configured authorities:

```text
scheduled_authority(height) = authorities[height mod N]
seal_digest = BLAKE3(domain || height || authority_id || block_hash)
```

```toml
kind = "proof-of-authority"

[proof_of_authority]

[[proof_of_authority.authorities]]
id = "validator-0"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"

[[proof_of_authority.authorities]]
id = "validator-1"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
```

The engine rejects empty sets, duplicate ids or keys, invalid keys, the wrong
scheduled signer, signer/key mismatches, and any height, hash, or signature
drift. `poa` is accepted as an input alias; reports emit
`proof-of-authority`.

## Tendermint configuration

The quorum must be strictly greater than two thirds of total voting power.
For four equal validators the smallest safe quorum is three:

```toml
kind = "tendermint"

[tendermint]
quorum_power = 3

[[tendermint.validators]]
id = "validator-0"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[tendermint.validators]]
id = "validator-1"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1

[[tendermint.validators]]
id = "validator-2"
public_key = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"
weight = 1

[[tendermint.validators]]
id = "validator-3"
public_key = "e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13"
weight = 1
```

The old `weighted-precommit` config spelling is accepted only as a migration
alias. Reports emit the canonical `tendermint` name.

## Round state

```mermaid
stateDiagram-v2
    [*] --> Propose
    Propose --> PrevoteBlock: valid proposal and lock rule permits
    Propose --> PrevoteNil: missing or invalid proposal
    PrevoteBlock --> PrecommitBlock: greater-than-two-thirds prevote value
    PrevoteNil --> PrecommitNil: greater-than-two-thirds prevote nil
    PrecommitBlock --> Decided: greater-than-two-thirds precommit value
    PrecommitNil --> NextRound: nil quorum or timeout
    NextRound --> Propose: retain lock and valid value
```

`TendermintRoundState` is serializable and is the WAL boundary before signing
the next message. It retains `locked_value/locked_round` and
`valid_value/valid_round`, rejects proposal/vote equivocation, and verifies a
proposal's `valid_round` against retained prevote quorum evidence. Networking,
peer discovery, timeout scheduling, validator-set changes, and durable WAL I/O
are operator/runtime responsibilities outside the deterministic state machine.

`MyelinBlock` commits to session lineage, consensus kind, real pre/post state
roots, ordered raw txids, DA commitments, and the scheduler commitment. All
three engines must produce identical transaction and state fields for the same
workload; their consensus-bound block hashes and proof material differ.
