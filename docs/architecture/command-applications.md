# Native command and fixed-tick applications

`myelin-session-journal::application` is an optional adapter for new, explicitly
identified host-command profiles. Applications implement `CommandProgram` and
`CommandAuthenticator`; their rules remain outside the session domain. The
adapter supplies profile construction, original-command frames, exact-head
reservation conversion, snapshots, result decoding and bounded state queries.

The adapter supports single-player save commands, two-player turn commands and
fixed-tick input packets. A browser/WASM differential checker runs the examples'
same rules and snapshot codec. The examples export portable offline bundles and
serve as their own pinned historical replayers. Handoffs, successors and
settlement require separate implementations.
It does not reinterpret historical game profiles or claim CKB-VM execution.

## Profile and deterministic execution

`CommandProfile::build` binds the actual program artifact digest, command schema,
authorization policy, entropy policy, explicit clock and resource envelope. The input-schema
commitment includes both the schema and authorization policy. The codec, clock,
ordering and host-evidence policies have explicit identities below.
The profile requires command identities and disabled protocol effects.

`CommandProgram::apply` sees the original authenticated command, input position
and logical time. It does not receive block height, commit timestamp or the host
clock. Under `ApplicationClock::CommandSteps`, each admitted command advances the
cursor and logical time by one, including a committed business rejection. Grouping the same commands into
different blocks therefore preserves the application snapshot and state root.
Block hashes and receipts still describe their particular block boundaries.

The application must make cloning isolate mutable state and perform no external
effects during `apply`. A successful outcome installs the cloned program. A
business rejection discards its mutations but records the input and result.
An execution/authentication error rejects the entire candidate, including earlier
commands in that candidate. Every command is authenticated again during replay.
The session atomically commits the resulting snapshot, block, command receipts
and result outbox. Duplicate IDs cannot create another committed input.

`ApplicationClock::FixedTicks` binds the nonzero tick duration in microseconds
and the numeric, seat-admission and replay-policy identities. The program must
declare the same clock as its profile. Each `InputTape` packet advances exactly
one simulation tick and one input position, regardless of the commit batch.
Fixed-tick programs must execute every admitted tick successfully: a business
rejection rejects the candidate instead of advancing time with unchanged physics.
Commands and standalone system events cannot enter a fixed-tick profile; control
events belong inside authenticated tick packets. The example starts at tick zero.

Native rules execute trusted host code. Byte limits bound protocol payloads;
they do not preempt arbitrary CPU, allocation or side effects inside an
application callback. Applications requiring termination limits must use the
[native worker boundary](native-workers.md). Reported CKB-VM cycles are zero;
host CPU/RSS must be measured separately.

## Wire and snapshot contract

Frames are `MCAF`, byte version `1`, then canonical Postcard encoding of the
ordered original commands. Empty frames use an empty payload. At most 256
commands are admitted per frame, also bounded by the genesis frame byte limit.
The frame identities must exactly match the embedded command identities. Each
original command retains session/genesis/profile scope, input kind, payload hash,
canonical command bytes, authorization context and authorization bytes.
Command-step profiles reject `InputTape`; fixed-tick profiles require it.

Snapshots are `MCAS`, byte version `1`, then canonical Postcard fields in order:
profile commitment, input cursor, logical time, application state bytes.
The root is BLAKE3 of the state domain followed by the complete snapshot.
The portable `application::snapshot` module borrows the decoded state slice, so
a forged byte length cannot allocate an attacker-declared buffer. Restore checks
format, size and profile before invoking the application decoder;
it then re-encodes and requires byte equality. A new program cannot use this
adapter to silently reinterpret an old snapshot under another profile.

Result outbox messages use topic `myelin-command-result-v1` and encode the
command identity and `BusinessOutcome`. `business_result` checks the exact
receipt position/block binding and rejects duplicate matching results. Absence
returns `None`, not an invented success. ACK follows durable result recovery.

`reserve_commands` retains original commands in canonical form; a batch budget
too small for the next command leaves that command queued. The request builder
preserves the reservation's head and ordered identities. Commit/reconnect uses
the existing [journal contracts](command-journal.md) and
[writer fencing](writer-fencing.md), including Unknown retention.

## Queries and capabilities

`command_inspector` checks session/profile and the restored snapshot root before
calling the application's bounded query method. Use `SessionChain::inspect_head`
or `CanonicalAuthority::inspect_head` for the current checkpoint. A historical
block's embedded snapshot may already have been pruned; loading the newest block
is not an equivalent current-state query.

The capability manifest binds the profile and advertises original input storage.
It advertises tick tapes for fixed-tick profiles, or standalone authenticated
system events for command-step profiles. The application authenticator still
decides which events it accepts. It reports false for handoffs, successors,
settlement and CKB-VM execution. Corresponding
unsupported execution paths reject rather than silently ignore inputs.

## Runnable examples

All three examples use **public development keys** and a local single-authority
gateway. They are reproducible integration examples, not production key setups,
HTTP servers or wallet flows.

```bash
cargo run --locked -p myelin-session-journal --example inventory -- /tmp/inventory-demo demo
cargo run --locked -p myelin-session-journal --example inventory -- /tmp/inventory-demo show
cargo run --locked -p myelin-session-journal --example inventory -- /tmp/inventory-demo replay
cargo run --locked -p myelin-session-journal --example turn_based -- /tmp/turn-demo demo
cargo run --locked -p myelin-session-journal --example turn_based -- /tmp/turn-demo replay
cargo run --locked -p myelin-session-journal --example fixed_tick -- /tmp/tick-demo demo
cargo run --locked -p myelin-session-journal --example fixed_tick -- /tmp/tick-demo replay
```

`inventory` starts with 100 credits, buys one potion for 25, saves checkpoint 3,
and records an invalid purchase as a business rejection. `turn_based` runs a
two-seat 3x3 board, checks seat signatures and stale turns, rejects an early
timeout, accepts an authorized timeout and rejects a move after the game ends.
Both examples use integer rules and no random draws. They have fixed admitted
players and no join/leave, AI takeover or hidden-information policy.

`fixed_tick` runs twelve 20 ms ticks for two fixed seats using integer positions
and velocities. Its replayable packets disconnect seat 1 at tick 2, transfer
control to AI at tick 4 and reconnect the same player at tick 8. AI takeover
requires two disconnected ticks. Control events have distinct seats in ascending
order. Disconnected/AI seats must supply neutral external input; stale/skipped
ticks, unauthorized packets and invalid transitions reject the entire candidate.
The sequencer signs each complete packet; this does not prove individual player
signatures on the embedded inputs. New player admission is unsupported.

The tick example pins BLAKE3 draws over a domain, a public 32-byte seed and a
little-endian counter, consuming two draws per tick in seat order. Snapshots
include the seed/counter and control state. Position uses integer millipixels,
velocity is bounded to ±12 per tick, acceleration is 3 per tick and position
clamps at ±1,000,000. Simulation rules live in the example's `tick_game.rs`,
outside the reusable adapter. Public deterministic RNG is not an economic
fairness protocol. All inputs and state are public; no hidden-information claim
is made. Browser prediction, animation and visibility handling are not implemented.

Timeouts are explicit replayable events signed by a dedicated system key. The
elapsed duration is an operator assertion checked against the configured rule;
the signature does not prove that physical time elapsed. A player's signature
cannot authorize the system event or the other player's move.

The program digest hashes the complete running executable. Preserve that exact
binary to reopen old example stores: recompilation can change its profile even
when the application source looks unchanged. `show` and `replay` require an
existing canonical store. Committed history with a missing original journal
fails instead of creating a replacement empty journal.

The demo reconciles a retained submission before retrying its original IDs.
After a crash, a signed later-epoch absence observation may safely requeue it.
The explicit local administrator can advance the epoch only after reopening the
exclusive canonical store. Copying the authority database/key to another host
does not establish safe failover.

## Executable acceptance

```bash
cargo test --locked -p myelin-session-journal --test applications
```

Twenty-one real child exits cover seven boundaries in each example: enqueue,
reserve, submission bind, canonical commit, result recovery, ACK and response.
Each case reopens twice and replays the store, checking state, results and ACK.
These cases run once as part of the workspace suite; the duplicate Python runner
has been removed.

The application tests also vary batch sizes and block timestamps, restore after
every batch, reject forged actors/events and wrong profiles, check whole-batch
rollback and rejected-program mutation isolation, and query a pruned head through
the audited current checkpoint. Chess and Neon Miner adopt these contracts in
independent repositories; their real HTTP recovery and independent source/image
release evidence is mapped in the [P1 acceptance record](../plans/p1-delivery.md)
and [downstream release procedure](../operations/downstream-release.md).

`recover_local_commands` supplies the same startup recovery used by these examples
and downstream services. It first checks the journal's pinned authority policy,
then reconciles any durable submission. Held work may be released; unresolved
submitted work requires an explicitly administrator-signed later epoch before
Missing can requeue the original commands. An existing matching worker grant is
retained when no unresolved work requires revocation. The authority must own the
exclusive canonical databases. This helper does not make a cloned authority
database safe for concurrent operation or replace remote migration.

## Portable game archives

The same executable that created a store exports it and replays its archive:

```bash
/absolute/inventory /absolute/inventory-store export /absolute/artifacts.json /absolute/new-bundle
```

The `turn_based` and `fixed_tick` executables accept the same command. The
existing canonical store must bind that exact executable and profile. The input
JSON requires `build` (the complete `PackedFiles` result from `pack-files`),
`runtime` (objects with absolute sandbox `path` and local file `source`),
`limits` (`BundleLimits`) and `work_budget` (`ArchiveBudget`). Local relative
paths resolve against the input JSON's directory. Both build and toolchain roots
point to the supplied current-format file package; complete build provenance
requires the source/vendor/script/toolchain kit described in
[session bundles](session-bundles.md#complete-reference-build-kits).

Export pins the current executable as both program and replayer, the serialized
application profile as rules, and every supplied runtime library. The report
includes the source-established development trust anchor, manifest commitment,
state root, block count and work usage. This public-key example identity does
not establish official deployment recognition. Work receives one explicit finite
budget; a pause fails the command and publishes no completed manifest. Large
build kits require accounting for copy, inventory and assembled-file validation,
as well as conservative store-read reservations.

The existing bundle CLI `replay` command uses independently recognized trust and
the configured Linux sandbox. Its pinned child is the archived original example
executable; it verifies its own artifact/profile and authenticates original
commands before applying the shared rules. Archive replay requires no source
RocksDB or command journal. The bundle preserves canonical history and business
results, not operational journal queues, ACKs or authority ownership.

The existing `bundle-build` lane builds all three examples, generates their
histories and exports them in a fresh offline container, removes those fixture
databases, then checks isolated archive replay against each original state root
and complete range receipt. The same lane compares two clean builds with the
original binaries and runtime. It adds no separate game gate or repeated native
test suite.

## Browser differential conformance

```bash
python3 scripts/myelin_browser_conformance.py \
  --browser /path/to/chrome \
  --wasm-bindgen /path/to/wasm-bindgen \
  --out /tmp/myelin-browser-evidence
```

Requires Node 22+, the `wasm32-unknown-unknown` target and wasm-bindgen CLI 0.2.118
(matching Cargo.lock). The existing production validation script exposes the same
runner through `--lane browser`; it does not repeat workspace tests.

`browser_vectors` executes the actual native `CommandExecutor` with authenticated
commands. The native program digest is the complete executing artifact hash.
`game_rules.rs` and `application_snapshot.rs` are compiled into both native and
WASM artifacts; there is no second copy of the game rules or snapshot encoding.
The WASM checker uses the authoritative native application's profile, then
independently applies the input bytes and compares complete snapshots, state
roots, business results and rejection outcomes with native observations. The
WASM artifact is separately retained and hashed; it does not change that profile.

Ten vectors cover the three games, repeated disconnect/AI/reconnect, a native
checkpoint resumed after disconnection, stale/skipped ticks, early takeover,
position clamping and RNG counter exhaustion. The last two start from explicitly
synthetic numeric-boundary checkpoints. Batch widths range from 1 to 64, with and
without snapshot restoration after each batch. Each vector starts a fresh WASM
instance, and the resumed vector continues a checkpoint produced by native
execution. The resulting 64 comparisons require exact bytes, with no numerical
tolerance. Each vector also rejects altered canonical input and a foreign-profile
snapshot. Native u64 values remain inside Rust or encoded bytes, including values
above JavaScript's exact Number range.

The runner waits for an explicit result from real Chrome/Chromium through CDP.
It creates a temporary browser profile and retains exact native/WASM binaries,
native vectors, browser observations, versions and SHA-256 inventory. A pending
page, failed navigation or WASM exception fails validation. Native and browser
execute debug artifacts; these results are not throughput measurements.

This checker receives native-authenticated public inputs. It does not implement
browser signature verification, journal persistence, consensus or settlement.
Native crash/recovery tests remain the evidence for those native journal paths.
This is example-rule conformance in Chrome/Chromium; other engines and a complete
game Web SDK have not been validated. Prediction, animation and visibility flows
remain game-owned.

## Registered identities

| Identity | Meaning |
| --- | --- |
| `myelin:application-codec:v1` | Command-step or fixed-tick snapshot mode |
| `myelin:application-clock:v1` | Explicit clock and fixed-tick numeric/admission/replay policies |
| `myelin:command-application-effects:v1` | Disabled protocol effects |
| `myelin:command-application-input:v1` | Command schema plus authorization policy |
| `myelin:command-application-evidence:v1` | Host replay capability |
| `myelin:command-application-state:v1` | Complete snapshot root |
| `myelin:command-application-order:v1` | Original input order |
| `myelin-command-result-v1` | Committed business result outbox topic |
