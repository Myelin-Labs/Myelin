# Cross-session commitment vectors

`cross-session-v1.json` publishes synthetic inputs, ordered canonical fields,
full preimage hex and BLAKE3 hashes for the ADR-008 proposed protocol.
These are serialization/commitment vectors, not signed transfers or evidence of
source debit, finality, availability or CKB settlement. Repeated-byte hash fields
are independent test inputs; they do not form a linked live history.

Each vector starts with the UTF-8 domain, without a length prefix. Encodings:

- `hash32`: exactly 32 decoded hex bytes.
- `u8`, `u16`, `u64`: unsigned little-endian integers. JSON stores decimal
  strings so other languages do not lose u64 precision.
- `bytes`: u64 byte length followed by the decoded hex bytes.

List lengths and enum/option tags appear as explicit fields in their canonical
order. Semantic type definitions and trust requirements are in
[ADR-008](../../docs/architecture/cross-session-protocol.md).

Check the fixed corpus:

```bash
cargo run --locked -p myelin-session --example cross_session_vectors
cargo test --locked -p myelin-session --test protocol_vectors
```

With dependencies cached, both commands also accept `--offline`.
The Rust reference encoder has an explicit `--emit` mode for protocol authors.
It writes to stdout and never overwrites the corpus. A format change requires
review of the field order, preimages and hashes before updating golden files.
It is not a bounded production importer for untrusted evidence bundles.

## Signed reference transfers

`reference-transfer-v1.json` fixes synthetic scarce-transfer and copyable-receipt
payloads, owner signatures, canonical authorization preimages and transfer IDs.
It uses the implemented `myelin:session-effect-handoff-authorization:v1` domain
and postcard field encoding, not the proposed ADR-008 encoding above. Source
identities are synthetic inputs; the public development key is not a production
credential. These vectors do not prove source locking, finality or custody.

```bash
cargo test --locked -p myelin-session-transfer --test vectors
python3 fixtures/protocol/verify_reference_transfer.py
```

The Python check requires `cryptography` for public-key derivation. It implements
its own postcard field encoder and BIP340 verification. BLAKE3 checks are in the
Rust golden test; the Python script does not independently implement that hash.
`cargo run --locked -p myelin-session-transfer --example transfer_vectors`
prints the corpus for reviewed regeneration without overwriting it.

## File package vectors

`file-package-v1.json` fixes the `MFPK` prefix, canonical JSON field order,
ordered/repeated chunk identities, complete-file digest and package-root digest
for the bytes `abcdefabc`. Python produced the field encoding independently;
a standalone BLAKE3 helper supplied the digests without using the package
encoder. The Rust inventory tests compare the complete fixed bytes and digests
and reject reordered chunks, changed complete-file digests and removed edges.
The vector is a file-integrity contract, not a build or execution proof.

```bash
cargo test --locked -p myelin-session-bundle --test inventory
```

`successor-transition-v1.json` fixes independently assembled preimages and BLAKE3
commitments for exact continuation, compatible rules and state conversion under
`myelin:successor-declaration-v2`. These are synthetic commitment fields, not
executed migration evidence; see [the succession contract](../../docs/architecture/session-succession.md).


`retirement.json` fixes the opt-in consumed-lock retirement action commitment,
command identity/payload/schema and command/event envelopes. Its explicit postcard
bytes were independently assembled in Python; a small Rust BLAKE3-only helper
computed digests. Rust tests compare the production encoders and commitments.
The source handoff and block references are synthetic, not deployed history.
