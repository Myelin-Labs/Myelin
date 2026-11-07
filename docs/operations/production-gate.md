# Production validation

Use one entry point with a new output directory:

```bash
scripts/myelin_production_gate.sh --profile core --out target/validation
scripts/myelin_production_gate.sh --lane browser --out target/browser-validation
scripts/myelin_production_gate.sh --lane core --lane bundle-container --out target/container-validation
```

The shell launcher delegates to `scripts/validation.py`; it contains no second
set of checks. The old `RUN_*` environment switches are rejected. Unselected
lanes report `skipped`, with reason `not selected`. Every selected lane is required:
a missing tool, failed subprocess, timeout or unsupported platform fails the run.
After a failure, remaining selected lanes are recorded as unexecuted skips and
the profile cannot pass. No previous report is imported as a current pass.

## Profiles and independent lanes

| Profile | Required lanes |
| --- | --- |
| `core` (default) | core |
| `ckb` | core, ckb-devnet |
| `games` | core, teeworlds, browser, bundle-container, bundle-build |
| `full` | all seven lanes |

`--lane` may be repeated to select individual required lanes without rerunning
core. It cannot be combined with `--profile`. Profiles specify executed scope,
not approval to publish a release or a claim that every P1 deliverable is done.
A core pass never counts as a CKB/game release profile pass.

| Lane | Commands and requirements |
| --- | --- |
| core | Tool versions, Rust fmt, diff whitespace, one all-target/all-feature Clippy pass, one workspace test pass with VM IPC and all four process fixtures, Python boundary tests, transfer CLI restart drill |
| ckb-devnet | Strict Rust microgame court, actual transactions and offline replay; pinned `CKB_BIN`, Rust 1.95.0 with RISC-V target, Clang, Python cryptography and free loopback ports. See [court](../architecture/ckb-microgame.md). |
| teeworlds | Actual external replay workload and reproducibility report; `TEEWORLDS_ROOT` (default sibling `teeworlds`) with its compiled replayer, map and config |
| browser | Native/WASM exact roots/results; `MYELIN_BROWSER`, `MYELIN_WASM_BINDGEN`, Node 22+, Rust wasm32 target; binding CLI must match Cargo.lock (0.2.118) |
| bundle-container | Fresh offline container restore; Linux namespaces, bubblewrap and Podman; evidence from selected core, or explicit `MYELIN_BUNDLE_EVIDENCE` and operator-trusted `MYELIN_TRUSTED_BUNDLE_CLI` |
| bundle-build | Complete archived kit restoration, two offline builds and transfer/three-game replay; Linux namespaces, bubblewrap, Podman, `MYELIN_BUILD_KIT`, independent `MYELIN_BUILD_KIT_SHA256`, original `MYELIN_BUILD_OUTPUTS`; trusted CLI from selected core or explicit `MYELIN_TRUSTED_BUNDLE_CLI` |

Clippy already type-checks all workspace targets; there is no redundant standalone
`cargo check` step. The transfer CLI drill uses the example already built by
the workspace test pass; there is no second `cargo build` with a different feature
set. Debug tests do not enable every feature: MuHash's optional
Rayon feature includes exhaustive cases intended for release runs. Process and
isolation cases run once inside the workspace suite. Native worker/bundle
isolation is exercised only on Linux x86_64/aarch64; a macOS core pass makes no
Linux isolation claim. The bundle-build lane uses original executables, removes
its fixture databases and compares isolated roots and complete receipts. Typed
current file packages are mandatory; there is no separate game-export gate.

## Machines and dependencies

Use Python 3.11+, Git and the Rust toolchain in `rust-toolchain.toml` (including
rustfmt and Clippy). Linux builds need a C/C++ compiler, Clang/libclang, CMake,
protoc, pkg-config/OpenSSL development headers, and the openssl executable for
ephemeral TLS fixtures. Install bubblewrap with `--disable-userns` support and
permit unprivileged user/mount/PID/network namespaces for Linux isolation tests.
Their failure fails the core lane. Hosted Ubuntu 22.04 builds bubblewrap 0.13.1
from the upstream release with its pinned SHA-256, selects that executable through
`MYELIN_BWRAP`, and keeps all sandbox restrictions enabled; its distro package is
too old for the current isolation policy.

On Debian/Ubuntu, install `clang libclang-dev cmake protobuf-compiler pkg-config
libssl-dev openssl bubblewrap`. On macOS, install Xcode command-line tools and
Homebrew `llvm protobuf cmake`; set `LIBCLANG_PATH` to the installed LLVM library
directory when discovery requires it. Apple Silicon's standard Homebrew location
is detected. No developer-home path is assumed.

TLS tests create a minimal, fixture-owned OpenSSL configuration and declare all
certificate extensions explicitly. The host's `openssl.cnf` must not add a
duplicate `basicConstraints` extension to the test CA.

`CARGO_BUILD_JOBS` defaults to 4. `CARGO_TARGET_DIR` may use a cache, but checks still
execute and report exact source identities. Put `TMPDIR` on a filesystem with
several GiB available for archive fixtures. Fresh native builds also need enough
space for C++ objects and test binaries. `--timeout-seconds` gives each command a
finite wall limit (default 7200, maximum 14400); commands run in owned process
groups which are terminated on cancellation/timeout. Existing application,
bundle and container controls retain their own tighter resource limits.

`scripts/ci/Containerfile` prepares the core dependency image, including the pinned
Clippy/rustfmt components and bubblewrap. Build it with network access:

```bash
podman build --pull=never -f scripts/ci/Containerfile -t myelin-core-validation scripts/ci
```

The pinned base image must already be present for `--pull=never`. The resulting
image identity must be recorded for an executed container drill.

Clean containers need the same dependencies, explicit input source/vendor bytes
and namespace support. Nested Linux sandbox tests require an outer container
policy that permits namespace creation; keep the inner application sandbox and
its syscall/resource restrictions enabled. For Podman, the clean core drill uses
`--security-opt=seccomp=unconfined --security-opt=unmask=/proc/*` on the outer
container so bubblewrap can mount its own proc filesystem; quote the latter
argument in a shell. It still uses an unprivileged UID, no capabilities, no network
and no host bind mounts. Container input completeness and
reproducible builds remain the separate [build-kit contract](../architecture/session-bundles.md#complete-reference-build-kits).

Ordinary core CI uses development fixture keys and ephemeral TLS material. It
requires no production signing keys, funded accounts or private external checkout.
Compiler preparation may use the network; bundle reproduction and replay
containers explicitly disable it. Browser conformance serves local test content.

## Evidence and CI

`report.json` uses `myelin-validation-v1`. It records the requested profile/lanes,
pass/fail/skip states and reasons, exact commands, wall times, exit codes, tool
versions in logs, platform/CPU information and GitHub run identifiers when present.
The source identity binds HEAD, dirty state and a SHA-256 inventory of the actual
Git-listed working-tree files, including uncommitted files. This includes Cargo.lock,
Rust toolchain locks and fixture bytes. The complete source identity is
checked again at the end; changes during validation fail the overall report.
A dirty-tree pass does not assert that HEAD alone contains the tested changes.

Logs and generated artifacts have relative paths, byte lengths, modes and SHA-256
values. Nested reports are included. Podman's private expanded image/run stores
are explicitly excluded container scratch; their approved image exports,
manifests, commands and outputs remain inventoried. No environment dump or
credential store is included. `report.json` itself is excluded from its own
recursive hash list. Output must be new and outside source or under `target/`.

`.github/workflows/validation.yml` runs the core profile on Linux and macOS for
pull requests and main-branch pushes, and supports manual dispatch. It installs
Python 3.11 explicitly rather than assuming the runner's system version. It pins the
[checkout](https://github.com/actions/checkout),
[Python setup](https://github.com/actions/setup-python) and
[artifact upload](https://github.com/actions/upload-artifact) actions by full commit
SHA, uses read-only repository permissions and uploads evidence even after a
failed command. No cache hit substitutes for validation. The committed workflow
configuration alone is not an executed hosted-CI result; retained run URLs and
artifacts establish those results after execution.
