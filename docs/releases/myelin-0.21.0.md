# Myelin 0.21.0

- Release date: 2026-08-25
- Tag: `v0.21.0`
- Previous release: `v0.20.0`
- Functional baseline: `c8b036ca97feba29df7b4ae5a8af6f922b2042b7`

Myelin 0.21.0 gives a continuous session an explicit application identity and
an explicit way to end, continue, or transfer work. Version 0.20 established
the modular runtime and finality boundary. This release fixes what each chain
means, what every block executed, how an operator can replay or inspect it,
and how external claims move forward without being promoted on trust.

This release is for controlled sessions, integration work, and benchmarking.
It is not a CKB full node, a new L1, a public-testnet court, a permissionless
bridge, or a finished permissionless L2.

## What changed

- Session genesis now binds the full application profile: program, input
  schema, state codec, logical-time and entropy policies, strict VM capability,
  resource bounds, court procedure, and handoff policy.
- Every finalised block now binds the session id and an execution frame with
  the exact input range and bytes root, logical-time range, state roots,
  ordered raw transaction ids, and measured resources.
- Read-only inspection returns a receipt tied to an immutable snapshot and has
  no capability to produce blocks, reserve work, consume handoffs, or emit
  outbox messages.
- Bounded range replay starts from a retained checkpoint before the requested
  range, deterministically reexecutes the intervening frames, rechecks their
  proofs and links, and returns an exact replay receipt.
- Evidence pipelines advance one locally verified stage at a time with durable
  revision CAS. The outbox item is acknowledged only after the terminal
  receipt is stored, so restart cannot silently skip or inflate a claim.
- A final predecessor block can declare exactly one successor, seal itself,
  and atomically create the target genesis from the exact state, cursor, time,
  codec, and predecessor reference.
- Source-committed handoffs carry a bounded payload to an exact target or
  intake policy, expire, may require a minimum evidence stage, and are consumed
  at most once in the target block transaction.
- RocksDB stores and audits the new frame, evidence, successor, lineage, and
  handoff state. CLI status commands expose each surface without writing it.

## Commit audit

The functional delta reviewed for this release is `bd59370..c8b036c`. It is
one implementation commit; the 0.20 release merge only places the already
reviewed 0.20 release commit and tag in the ancestry of this release.

| Commit | Review | Release effect |
| --- | --- | --- |
| `c8b036c` | Accepted | Adds application profiles and execution frames, inspection, range replay, staged evidence, successors, handoffs, atomic persistence, CLI status surfaces, documentation, and production-gate corrections. |
| `71c4974` | Accepted | Merges the reviewed 0.20 release metadata into `main`; it does not replace or weaken the 0.21 implementation. |

## Review conclusions

- A session is cell-based at the execution and state layers, but its chain no
  longer pretends that state roots alone describe an application. The profile
  and execution-frame commitments close that semantic gap.
- Successor chains need not be homogeneous. The target may bind a new program,
  consensus module, or policy, but it must preserve the state codec, start from
  the exact declared predecessor head, and carry an auditable reverse reference.
- A successor and a handoff are deliberately different. A successor continues
  one state lineage; a handoff transfers a bounded application payload between
  histories that remain separate.
- External collectors do not get to assert a higher evidence stage. The stage
  changes only after the named local verifier accepts the exact evidence and
  the store links it to the prior receipt.
- Inspection is structurally read-only, not merely a command convention.

## Validation record

The release tree passed the following checks on 2026-08-25:

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
ALLOW_SKIP_TEEWORLDS=1 scripts/myelin_production_gate.sh
```

The production gate passed its parent CKB devnet smoke, including live
deployment, submission, commitment checks, and rejection of replayed or
tampered settlement evidence. Teeworlds is an external, non-vendored workload;
its checkout and replayer were unavailable, so that workload was explicitly
skipped. The skip is not a passing Teeworlds integration result.

## Upgrade note

Myelin had not made a public compatibility commitment for the 0.20 session
store and report shapes. Version 0.21 replaces those internal schemas directly.
Existing local session databases and JSON reports should be regenerated rather
than passed through an alias, legacy reader, or one-off migration.

## Remaining work

- Connect real application adapters to the new profile, input, inspection, and
  replay ports and exercise them under long-running recovery tests.
- Deploy and exercise the exact final verifier path on public CKB testnet.
- Complete disputed-chunk adjudication and court economics.
- Integrate durable external DA with independent retrieval and failure drills.
- Establish production validator/operator custody, rotation, monitoring, and
  incident-response procedures.
