# Native execution workers

`myelin-session-worker::ProcessExecutor` implements `TransitionExecutor` using
one new child process for each restore or execution. The parent keeps the
snapshot, finality verifier, store and commit authority. A failed or cancelled
child cannot advance a `SessionChain`: preparation uses a candidate clone, and
only a validated outcome followed by finality and atomic persistence replaces
the live state.

The Linux x86_64 implementation is exercised locally. The syscall filter also
defines the aarch64 ABI, but that architecture has not been exercised here.
The kernel must support executable sealed memfds and seccomp filters. Other
platforms return `UnsupportedPlatform`; there is no in-process fallback.

## Execution and trust boundary

The operator supplies a local ELF path, trusted BLAKE3 digest and fixed argument
list to `WorkerProgram::load`. The loader bounds the artifact size, checks the
digest, and copies it to a sealed memfd. The child executes that immutable copy;
replacing or editing the original path does not replace the running program.
Dynamic system libraries remain part of the deployment's trusted environment.
Complete historical artifact/build manifests remain the responsibility of #13.

The application provides a child entry point using
`wire::serve_stdio(expected_profile_commitment, restore_executor)`. It restores
the supplied snapshot, verifies its root, executes the ordered frame, and returns
the outcome and complete new snapshot. It must reserve stdin/stdout for the
binary protocol and keep ordinary diagnostics on stderr. The worker has no
store handle, authority key or inherited environment variables supplied by this
adapter. Its executor must be deterministic and must not perform external side
effects; those belong in the parent's transactional outbox.

The current worker envelope is `MWK2`; older packets are rejected. Canonical
session execution and replay use one `Block` operation carrying the complete
source-effect context and an explicit lifecycle-action list, which may be empty.
The child checks pre-root/input bindings and invokes `execute_block_with_lifecycle`;
an application without that policy rejects nonempty actions. `Frame` serves the
current low-level `execute_frame`/`TransitionExecutor::execute_block` API and does
not substitute for canonical block execution. `Restore` validates a snapshot.
There is no old-record execution branch or protocol downgrade. See
[source-effect authorization](protocol-effects.md).

This is resource isolation for trusted application executables. It is **not**
a filesystem or network sandbox. The child retains the user's operating-system
permissions and working directory. Do not use it to execute unknown downloaded
native code without an additional sandbox/container policy. That import boundary
is provided separately by #13's [archive runner](session-bundles.md#isolated-native-replay).
That runner does not silently change the live worker's permissions or ABI.

## Operational limits

`WorkerSettings` combines the exact application profile, genesis-bound commit
limits and a separate `OperationalLimits`. Pass the session's actual profile and
commit limits when constructing the executor. The protocol budgets described in
[aggregate commit limits](commit-resources.md) still apply after worker execution.

| Operational limit | Enforcement |
| --- | --- |
| Wall time | Parent monotonic deadline; nonblocking pipe reads/writes; process termination and reaping |
| CPU seconds | Kernel `RLIMIT_CPU`, soft limit at the configured whole seconds and hard limit one second later |
| Address-space bytes | Kernel `RLIMIT_AS`; applies to the worker's virtual address space, not a claimed RSS quota |
| Input/output bytes | Count before encoding and during pipe transport; nested arrays and strings use bounded decoders |
| Stderr bytes | Bounded capture; excess output terminates the invocation |
| Cancellation | Shared, permanent cancellation handle checked during supervision; child terminated and reaped |

Seccomp rejects process/thread creation, ABI switching, namespace changes and
leaving the process group. The execution model is deliberately single-threaded;
applications requiring child threads must use another explicitly bounded worker
implementation. This prevents an application from bypassing a per-process limit
by creating additional processes. Core dumps are disabled.

The parent owns each child's lifetime. It keeps the leader unreaped while
terminating its group, then obtains `wait4` resource usage and reaps it. The host
must not install a competing global child reaper or ignore `SIGCHLD`. Deadline
precision depends on OS scheduling; this is not a real-time deadline guarantee.
As with any SIGKILL-based supervisor, a kernel task stuck in uninterruptible I/O
cannot be made immediately reapable by an asynchronous Rust timeout.

Protocol validity remains independent of these settings: a local timeout or
memory-constrained process failure does not declare a block invalid under a
different machine's policy. It prevents this host from committing that attempt.
Queue quotas remain local producer/network policy; RocksDB's per-direction
network queue message/byte quotas are separate from the genesis commitment.
Physical disk capacity requires operator filesystem quotas/headroom and the
retention/archive work in #18; `max_record_bytes` is not a total database quota.

## Integration

Create a `WorkerProgram` once from the trusted artifact and reuse its immutable
handle. Construct `WorkerSettings` from the session config and local operational
policy. `ProcessExecutor::restore(program, settings, snapshot, expected_root)`
performs a bounded child restore before returning an executor. Pass that executor
to `SessionChain` or `RuntimeHost` as usual. The same constructor works in a range
replay restore callback, so historical execution follows the same process limits.

Keep `executor.cancellation()` and `executor.monitor()` before moving the
executor into the session. Call the cancellation handle when abandoning its
execution scope; cancellation is permanent for that executor and its clones.
Recovery with a fresh executor creates a new cancellation scope. Because the
trait is synchronous, asynchronous hosts should call preparation on a blocking
worker thread; dropping an async future alone is not a cancellation signal.

`ExecutionMonitor` retains one bounded report shared by candidate clones. Reports
bind the artifact and request hashes and operational limits, and expose exit
status, signal, bounded stderr and a typed failure category. `TransitionExecutor`
maps failures to its existing string error contract; the report preserves the
structured category even when the candidate is discarded. Direct callers can
use `execute_frame` to receive a typed error with its own invocation report;
concurrent candidates cannot substitute another request's process statistics.

The transport checks magic/version, exact length, checksum, nested sizes and
trailing bytes. Replies must bind the exact request hash and pre-state root.
Successful process exit alone is insufficient to accept a reply.

## Measurements and acceptance evidence

Reports contain wall time, user/system CPU time and RSS independently of
`ExecutionOutcome.cycles`. Linux `wait4` reports lifetime peak RSS, which can
include the forked parent's pre-exec memory image. It therefore must not be
compared directly to the post-exec `RLIMIT_AS` limit.

The parent also samples `/proc/PID/status` `VmHWM` after exec. The reported maximum
is an observed lower bound on post-exec peak RSS, with an explicit sample count.
It is `None` when no live sample was obtained; missing data is never reported as
zero memory usage. CPU and lifetime RSS include process startup/teardown costs;
wall time covers spawn through reaping, not parent artifact loading or request
serialization.

Run the Linux acceptance lane:

```bash
MYELIN_WORKER_METRICS_OUT=/tmp/myelin-worker-metrics.json \
  cargo test --locked -p myelin-session-worker --features test-fixture
```

The feature compiles a test-only executable. Tests exercise CPU and wall-time
infinite loops, explicit cancellation, a denied 1 GiB allocation under a 64 MiB
address-space limit, stdout/stderr floods, abnormal exit, malformed/stale replies,
wrong restored state, blocked descendant creation, immutable artifact loading,
independent workers, native/worker equality, and replay after restart. Real
RocksDB checks prove failed execution preserves the head, checkpoint, command
receipt, handoff consumption marker and outbox, followed by one successful retry.

The core profile includes these tests on supported Linux hosts and writes its
metrics artifact. Each run emits its own measurements. These debug smoke measurements are not
a game throughput benchmark, capacity estimate or substitute for #20.
