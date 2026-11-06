#!/bin/bash
# Build the riscv-tests conformance corpus for the calc VM.
#
# Upstream sources are NOT vendored (see vm/tests/archtest/README.md); clone:
#   git clone --depth 1 https://github.com/riscv-software-src/riscv-tests.git
#   git clone --depth 1 https://github.com/riscv/riscv-test-env.git  (its env/)
#
# Suites and their calc profiles (container header byte 5):
#   rv64ui            -> 0  RV64IM
#   rv64um            -> 0  RV64IM
#   rv64uzbb/zba/zbs/zbc -> 1  RV64IM + Zba/Zbb/Zbs/Zbc (profile B)
#   rv64ui/rv64um compiled with -march=rv64imc (c_* prefix)
#                     -> 2  RV64IMC (profile C): clang auto-compresses the
#                            RV64I tests into RVC forms, giving profile-C
#                            execution conformance without rvc.S
#   (rvc.S itself is excluded: its first case embeds .dword data inside the
#    executable segment to test 4-byte fetch across a page boundary; the
#    calc verifier deliberately requires every X-segment byte to decode to a
#    whitelisted instruction, so that image cannot pack. See README.)
#
# Requires: clang >= 18 with the riscv64 backend, and lld (a rust-lld copy
# symlinked as ld.lld works: see ldshim below).
set -eu
cd "$(dirname "$0")"
SRC=${1:?usage: build.sh /path/to/riscv-tests}
MACROS=$SRC/isa/macros/scalar
OUT=elf
CC="clang --target=riscv64-unknown-elf -mno-relax -static"
CC0="$CC -march=rv64im"
CCB="$CC -march=rv64im_zbb_zba_zbs_zbc"
CCC="$CC -march=rv64imc"
LD="clang --target=riscv64-unknown-elf -nostdlib -fuse-ld=lld -B$PWD/ldshim \
    -Wl,--image-base=0x0 -T calc-tests.ld -Wl,--build-id=none"
# Excluded from rv64ui: fence_i (fence.i is rejected by the profile-0
# decoder), ma_data (expects M-mode misaligned trap fixups; the calc VM is
# Zicclsm), and rvc.S in rv64uc (see above).
EXCLUDE="fence_i ma_data rvc"
mkdir -p "$OUT" ldshim
RUST_LLD=$(find "$HOME/.rustup/toolchains" -path "*x86_64-unknown-linux-gnu/bin/rust-lld" 2>/dev/null | head -1)
[ -n "$RUST_LLD" ] && ln -sf "$RUST_LLD" ldshim/ld.lld
rm -f "$OUT"/*.elf "$OUT"/profiles.txt
build_suite() { # dir cc profile
  local dir=$1 cc=$2 prof=$3 t name skip
  for t in "$SRC"/isa/$dir/*.S; do
    name=$(basename "${t%.S}")
    skip=0; for e in $EXCLUDE; do [ "$name" = "$e" ] && skip=1; done
    [ $skip = 1 ] && continue
    $cc -I "$PWD" -I "$MACROS" "$t" -c -o "$OUT/$name.o"
    $LD -Wl,-e,_start -o "$OUT/$name.elf" "$OUT/$name.o"
    echo "$name=$prof" >> "$OUT/profiles.txt"
    rm "$OUT/$name.o"
  done
}
for suite in rv64ui rv64um; do build_suite "$suite" "$CC0" 0; done
for suite in rv64uzbb rv64uzba rv64uzbs rv64uzbc; do build_suite "$suite" "$CCB" 1; done
# profile C: rebuild the RV64I/M suites with RVC enabled under a c_ prefix
# (upstream rvc.S is excluded above: it embeds .dword data inside the
# executable segment, which the calc verifier deliberately rejects)
for suite in rv64ui rv64um; do
  for t in "$SRC"/isa/$suite/*.S; do
    name=$(basename "${t%.S}")
    skip=0; for e in $EXCLUDE; do [ "$name" = "$e" ] && skip=1; done
    [ $skip = 1 ] && continue
    $CCC -I "$PWD" -I "$MACROS" "$t" -c -o "$OUT/c_$name.o"
    $LD -Wl,-e,_start -o "$OUT/c_$name.elf" "$OUT/c_$name.o"
    echo "c_$name=2" >> "$OUT/profiles.txt"
    rm "$OUT/c_$name.o"
  done
done
# calc-native RVC kernels (sources live HERE, not upstream): rvc_forms.S
# carries one checked case per whitelisted compressed form; rvc_ebreak.S /
# rvc_unimp.S probe the two trap encodings in dedicated images (see the
# file headers).
for t in rvc_forms rvc_ebreak rvc_unimp; do
  $CCC -I "$PWD" -I "$MACROS" "$PWD/$t.S" -c -o "$OUT/$t.o"
  $LD -Wl,-e,_start -o "$OUT/$t.elf" "$OUT/$t.o"
  echo "$t=2" >> "$OUT/profiles.txt"
  rm "$OUT/$t.o"
done
ls "$OUT"/*.elf | wc -l
