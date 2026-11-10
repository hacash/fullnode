#!/bin/sh
# Rebuild the workload ELFs pinned by vm/tests/calc_workload_pin.rs.
# Needs rust-src and the rv64im custom target (no C, no B, no F/D, no A).
# rustc does not hash -T scripts; the touch forces a relink.
set -e
cd "$(dirname "$0")/kernel"
touch src/bin/*.rs src/*.rs
RUSTC_BOOTSTRAP=1 cargo build \
  -Zbuild-std=core,compiler_builtins \
  -Zbuild-std-features=compiler-builtins-mem \
  --release
OUT=target/riscv64im_zicclsm/release
DEST="$(cd .. && pwd)/elf"
mkdir -p "$DEST"
for b in uni_v2 uni_v3 curve rpow aave bpow black_scholes settle median \
         sha256 keccak poseidon rsa2048 secp256k1; do
  cp "$OUT/$b" "$DEST/$b.elf"
done
echo "rebuilt workload elves in $DEST"
