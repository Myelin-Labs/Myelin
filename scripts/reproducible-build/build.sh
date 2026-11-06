#!/bin/sh
# Runs as an unprivileged user in a fresh, network-disabled build container.
set -eu
umask 022
export CARGO_HOME=/work/cargo
export CARGO_TARGET_DIR=/work/target
export RUSTUP_TOOLCHAIN=1.92.0
export CARGO_NET_OFFLINE=true
export CARGO_INCREMENTAL=0
export CARGO_BUILD_JOBS=4
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_DEV_OPT_LEVEL=0
export SOURCE_DATE_EPOCH=315532800
export TZ=UTC
export LC_ALL=C
export RUSTFLAGS='--remap-path-prefix=/work=/myelin-build'
export CFLAGS='-g0 -ffile-prefix-map=/work=/myelin-build'
export CXXFLAGS='-g0 -ffile-prefix-map=/work=/myelin-build'
mkdir -p "$CARGO_HOME" /work/source /work/vendor /work/result
tar -xf /inputs/source.tar -C /work/source
tar -xf /inputs/vendor.tar -C /work/vendor
cat > "$CARGO_HOME/config.toml" <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "/work/vendor"
[net]
offline = true
EOF
cd /work/source
rustc -vV > /work/result/rustc.txt
cargo -V > /work/result/cargo.txt
gcc --version > /work/result/gcc.txt
clang --version > /work/result/clang.txt
target="$(rustc -vV | sed -n 's/^host: //p')"
cargo build --frozen --offline --target "$target" \
    -p myelin-session-bundle -p myelin-session-journal --features myelin-session-bundle/reference-replayer \
    --bin myelin-reference-transfer-replayer --bin myelin-session-bundle \
    --example inventory --example turn_based --example fixed_tick
python3 /work/source/scripts/reproducible-build/collect.py "$target"
