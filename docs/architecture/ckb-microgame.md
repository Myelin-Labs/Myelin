# A strict CKB microgame court

The reference is a two-player game: players alternately add one or two to a
counter, and the player reaching exactly five wins. The same deployed RISC-V
program runs in an isolated offline CKB-VM replay and in actual CKB transactions.
A valid authority signature, a successful carrier transaction and a correct game
result remain distinct facts. This fixture demonstrates the last property by
having CKB reject an incorrect finalized result with game error 20.

This is a public-key test-asset devnet reference. It uses CKB capacity, two fixed
participant secp locks and Dummy PoW. It supplies neither a permissionless L2
consensus model nor production/mainnet custody evidence. Its public-testnet
counterpart requires a separately reviewed network genesis, participant keys,
funding/capacity/fee choices, deployed code outpoint and execution receipt. The
devnet runner creates a fresh isolated chain and refuses an existing destination.
It has no option that silently turns its test RPCs or public keys into a live
network deployment.

## One program and a precise state

[The contract](../../fixtures/ckb-microgame/contract/src/main.rs) is a strict
365-byte, current-only state verifier. State fixes the deployed program's CKB
BLAKE2b code hash, schema hash, Data2/VM2 execution profile, unique game TypeID,
both participant lock hashes, stake, relative timeout, sequence, turn, counter,
proposal input and claimed post-root. Reserved bytes must be zero. Roots bind all
immutable rules and participants plus the logical game state. A proposal retains
the exact pre-root, proposed input/post-root and a retrievable input commitment.

The contract uses a fixed buffer to load state. It verifies every transition;
there is no native fallback, general-purpose witness oracle or authority override
for an incorrect game result. The offline runner executes this original binary
against retained exact input/dependency Cells, transaction bytes and signatures.
Its result is CKB-VM script validity. Actual transaction inclusion, capacity
conservation, spent-input checks and relative-since maturity are checked by the
node separately. A deliberately premature timeout passes script replay and is
then rejected by the node for immaturity.

The fixture pins Rust 1.95.0 and its own Cargo lock, separately from Myelin's host
workspace toolchain. RISC-V compilation uses `-C target-feature=-a -C
passes=lower-atomic`: CKB-VM does not execute the atomic instructions which a
normal `riscv64imac` build can generate through Molecule/bytes. This follows the
[official CKB script templates](https://github.com/nervosnetwork/ckb-script-templates#noticeable-changes).
The evidence records both the contract artifact and the actual compiler/flags.
The devnet block cycle ceiling is also 10,000,000, matching the replay budget;
the archive binds the complete chain specification separately from its genesis
block hash.
CKB BLAKE2b code/logical-state hashes are not Myelin's BLAKE3 executable or
CellStateTree commitments; applications integrating the fixture must preserve
those algorithm and representation boundaries.

## Transactions, authorization and custody

Each player stakes 1,000 test CKB. The game Cell holds exactly 2,000 CKB and uses
a public always-success lock with the strict game **type script**. This public
lock does not authorize an exit: the type script constrains every update and
burn. The game TypeID permits at most one live game Cell in its group; an exit
spends it. Creating a second game cannot recreate the same original TypeID.

A move is authorized by an additional input whose actual lock hash matches the
current participant. CKB executes that input's secp256k1 lock, including its
signature over the complete transaction and witnesses. Supplying a participant
name or hash in a witness cannot substitute for this input. Updates preserve
both escrow capacity and immutable game rules. An additional participant/sponsor
Cell pays the explicitly recorded transaction fee; settlement cannot deduct it
from the required escrow payout.

| Action | Required input and verification | Output / irreversible result |
| --- | --- | --- |
| Open | Both participant input lock groups, fresh TypeID, initial counter/root and exact capacity | One Ready game Cell |
| Commit | Ready state, current participant's actual lock input, delta one/two and input commitment | One Proposed Cell retaining the pre-state and claimed result; the result is still a proposal |
| Finalize | Proposed state, recompute the rule and exact claimed post-root in CKB-VM | One next Ready Cell, or Won at exactly five; incorrect math/proof fails with error 20 |
| Challenge | Incorrect proposal and the opponent's actual lock input | One Won Cell for the opponent; challenging a correct proposal fails with error 21 |
| Settle | Won state | Burn that game Cell; one exact 2,000-CKB payout to the winner |
| Timeout | The same game Cell and a relative-block `since` of at least eight, matured by actual CKB consensus | A finished win pays the winner; an incorrect pending proposal pays the opponent; an unfinished game refunds exactly 1,000 CKB each |
| Abort | Initial Ready state and both actual participant lock groups | Burn the same Cell and return the two stakes |

Every normal and recovery exit consumes the same game outpoint. A won game
cannot use timeout to obtain two refunds instead of the winner's payout. Stale
and duplicate exits are rejected as spent inputs. There is no parallel host
nullifier ledger whose reset could create another withdrawal. A proposed result
is intentionally not treated as a completed result: the invalid-proposal cases
show a proposal committing, its finalization being rejected by CKB, and an actual
successful challenge and settlement.

These rules define a small test game, including an unfinished-game refund rule.
They do not automatically give a larger game an open economy, conversion policy,
participant admission, cross-session exchange or cross-ledger custody protocol.

## Data retrieval and receipts

A proposal's `da_root` is the CKB hash of exact program/schema/VM/game bindings,
participants/stake/timeout, sequence, pre-root, actor/delta and claimed result.
The type script recomputes that commitment. The evidence package retains the
original input bytes, exact transactions/witnesses, code and resolved Cells,
full blocks, canonical headers, local VM verdicts and receipt observations.

Each receipt reports three separate stages: RPC acceptance, committed block
membership, and the configured confirmation depth. The local replayer rehashes
the complete block header and transaction/witness/proposal/uncle commitments.
The recovery verifier also reconstructs canonical input single-spends, checks
receipt ancestry/depth, matches resolved Cells to their retained creating
transactions, reexecutes every game proof and verifies each proposal input root.
A trusted genesis/tip and devnet consensus assumption remain explicit; hashing a
retained block body does not by itself establish public-chain fork choice.

The drill actually truncates the devnet after normal settlement. Its old receipt
is then reported as reorged, and a recovery timeout spends the revived same game
Cell on the new canonical history. Resubmitting the old normal exit is rejected.
A historical configured-depth observation is therefore never a permanent exit
right on an orphaned branch.

Local operator files are **not independent DA**. CI publishes the content-hashed
package to GitHub Actions' external artifact service. A fresh recovery job
receives the manifest identity through job outputs, downloads the artifact on a
new host, verifies its original replayer before execution, and repeats proof,
receipt and input-root verification without the operator node or database. It
publishes a separate retrieval receipt naming the provider and artifact identity.
The producer's immutable report retains `pending-external-provider-retrieval`;
the independent receipt supplies the actual result. Retention is seven days in
this drill. This demonstrates retrieval at that point in time, rather than
indefinite availability, public DA consensus or a production recovery SLA.

## Reproduce and inspect

Install Rust 1.95.0 with the RISC-V target, clang, Python 3.11+ and cryptography
50.0.0. Obtain the official CKB 0.209.0 Linux binary and verify the release archive
SHA-256 `cf6766e7ffe8096740f33491748dec14ec5ec302bc7051cdb59996930138cacd`.
The existing selected CKB lane builds both artifacts, runs transactions and
packages their evidence:

```bash
CKB_BIN=/owned/ckb scripts/myelin_production_gate.sh \
  --lane ckb-devnet --out /owned/fresh-court-validation
```

Inspect `ckb-devnet/court/evidence/report.json`, `deployment.json`, `transactions/`,
`proofs/`, `inputs/` and `blocks/`. `ckb-devnet/package.json` supplies the archive
manifest identity. Reverify an independently retrieved package with a separately
obtained manifest digest:

```bash
python3 scripts/myelin_microgame_archive.py verify \
  --evidence /owned/retrieved-package --replayer /owned/trusted-replayer \
  --manifest-sha256 EXPECTED_DIGEST --provider ACTUAL_PROVIDER \
  --artifact-id ACTUAL_ARTIFACT --out /owned/retrieval-receipt.json
```

The automated negatives cover forged participant signatures, a wrong VM profile,
an unauthorized move, incorrect game math, wrong claimed post-proof, a false
challenge, winner/refund substitution, missing timeout evidence, actual premature
maturity, duplicate/stale claims, conflicting normal/recovery exits and a real
reorg. Old compact-carrier smoke remains an explicit adapter fixture; it does not
supply this game's rule-verification claim. The CKB lane now runs this complete
court reference instead of adding another overlapping gate.
