# Independently packaged game releases

Chess and Neon Miner keep their game rules, browser/host and native executable in
their own repositories. A game source release includes its own source tree and a
`framework/` directory containing the exact `myelin-source.tar` used to build it.
The game's `deploy/myelin-source.sha256` is the trusted dependency pin. A version
label or a mutable sibling checkout cannot substitute for that digest.

Export a framework package from the approved working tree:

```bash
python scripts/myelin_reproducible_build.py pack-source --source . --out /absolute/new/framework
```

The existing archive encoder records normalized modes, hashes every file, rejects
links and unsafe paths, excludes Git metadata/ignored files/deletions and checks
that the source inventory stayed unchanged. `source.json` records the baseline
commit, dirty-tree digest and exact archive members. An uncommitted candidate is
identified as such; it is not represented as a published framework commit. Keep
this manifest and the tar together. Updating a game's dependency requires an
explicit change to its checked-in checksum and a new release build.

For a release laid out as `myelin-chess/` (or `myelin-neon-miner/`) beside
`framework/`, build from the game directory:

```bash
docker build --build-context myelin=../framework -f deploy/Standalone.Dockerfile -t game:release .
```

If the Docker client's download proxy listens only on host loopback, add
`--network=host` to the build command so build steps can reach that configured
proxy. This changes build-time networking only; the running game still uses its
own container network. Record this environment choice with the build command.

The Dockerfile verifies the independently pinned checksum before extracting or
compiling any framework source. Rust 1.92.0 and Node 22.23.1 base image digests are
pinned, Cargo uses `--locked` and npm uses the game's lockfile. Builds default to
four Cargo jobs. BuildKit caches contain disposable compiler/download data, not
the authoritative framework input. No UTXOPlay store checkout, host build target,
framework checkout or runtime data is copied into the build.

Each final image includes the source checksum and the exact native executable.
Run it with its own persistent data volume and stable server-side credentials;
the native port stays on container loopback. Retain that exact image/binary with
exports because application profiles bind the running executable. A different
build cannot reinterpret an existing signed save. Release rollback selects the
corresponding data namespace; old storage formats are unsupported.

For example, verify a Chess export with the retained image after removing the
producing service and its data volume:

```bash
docker run --rm --network=none --read-only --cap-drop=ALL \
  --security-opt=no-new-privileges --tmpfs /tmp:rw,nosuid,nodev,size=64m \
  --mount type=bind,src=/absolute/evidence,dst=/evidence,readonly \
  --entrypoint /app/target/release/myelin-chess game:release verify /evidence/export.json
```

The verifier needs writable temporary storage for its disposable replay database;
the input export and image remain read-only. Miner uses its own image and
`/app/target/release/myelin-neon-miner` entrypoint. This verifies application replay
and ordinary PoA under the export's declared authority, not CKB settlement or
independently pinned canonical-fencing membership.

This is a source dependency pin and independent build path. It is not the complete
offline toolchain/vendor kit described in [session bundles](../architecture/session-bundles.md),
and does not claim bit-identical container rebuilding: package repositories remain
build-time inputs. Record the built image digest, platform, source inventories,
commands and runtime checks for each release. Build and run a game package without
the original checkouts before describing it as independently runnable.
