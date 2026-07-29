# Closed-validator finality

Myelin finalises finite-session `MyelinBlock`s with one of two configured engines:

- `static-closed-committee` — configured quorum weight;
- `weighted-precommit` — configured quorum power plus height/round/precommit evidence.

Both verify secp256k1 Schnorr signatures and use different signature domains. They are closed-validator mechanisms, not permissionless consensus protocols.

Example weighted-precommit configuration:

```toml
kind = "weighted-precommit"

[weighted_precommit]
quorum_power = 2

[[weighted_precommit.validators]]
id = "validator-0"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[weighted_precommit.validators]]
id = "validator-1"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1
```

Configuration is exact: unknown fields, old names, and aliases are rejected.

`MyelinBlock` commits to the session lineage, consensus kind, real pre/post state roots, ordered raw txids, DA commitments, and scheduler commitment. For the same workload, both engines must produce identical transaction/state fields. Certificate hashes must differ because the signature domains differ.

Weighted precommit is intentionally not branded as a full external consensus protocol: the implementation verifies one finite-session weighted certificate and does not implement networking, proposer selection, validator-set changes, locking rounds, or a permissionless membership mechanism.
