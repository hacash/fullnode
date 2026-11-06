#!/bin/sh
# Fetch (git-ignored) riscv-arch-test old-framework-2.x and compile rv64i_m
# I+M images for the calc interpreter harness. Does not vendor the suite.
#
# 2.x still ships `*.reference_output`. Default `act4` / 3.x moved those to
# Sail-generated self-check ELFs and Google Drive.
#
# The harness writes mem_kib into the container header: the smallest KiB
# count whose layout clears the image, at most 16384 (16 MiB, testkit
# ceiling). An image that does not fit in that ceiling is excluded.
set -e
ROOT="$(CDPATH= cd -- "$(dirname "$0")/../../.." && pwd)"
PLUGIN="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
SRC="${CALC_ARCHTEST_SRC:-$ROOT/target/riscv-arch-test}"
OUT="${CALC_ARCHTEST_DIR:-$ROOT/target/calc-archtest-images}"

fetch() {
  if [ -f "$SRC/riscv-test-suite/env/arch_test.h" ] \
      && [ -d "$SRC/riscv-test-suite/rv64i_m/I/src" ] \
      && [ -d "$SRC/riscv-test-suite/rv64i_m/I/references" ]; then
    echo "suite: $SRC"
    return
  fi
  echo "cloning riscv-arch-test old-framework-2.x into $SRC"
  rm -rf "$SRC"
  git clone --depth 1 --branch old-framework-2.x --filter=blob:none --sparse \
    https://github.com/riscv-non-isa/riscv-arch-test "$SRC"
  git -C "$SRC" sparse-checkout set \
    riscv-test-suite/env \
    riscv-test-suite/rv64i_m/I \
    riscv-test-suite/rv64i_m/M
}

find_cc() {
  if command -v riscv64-unknown-elf-gcc >/dev/null 2>&1; then
    CC=riscv64-unknown-elf-gcc
    return
  fi
  if command -v riscv64-linux-gnu-gcc >/dev/null 2>&1; then
    CC=riscv64-linux-gnu-gcc
    return
  fi
  RUST_LLD="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld/ld.lld"
  if [ ! -x "$RUST_LLD" ]; then
    RUST_LLD="$(find "$(rustc --print sysroot)/lib/rustlib" -name ld.lld 2>/dev/null | head -1)"
  fi
  if command -v clang >/dev/null 2>&1 && clang -print-targets 2>/dev/null | grep -qi riscv && [ -n "$RUST_LLD" ] && [ -x "$RUST_LLD" ]; then
    CC="clang --target=riscv64-unknown-none-elf -fuse-ld=$RUST_LLD"
    return
  fi
  echo "missing RISC-V toolchain: need riscv64-unknown-elf-gcc, riscv64-linux-gnu-gcc, or clang with RISC-V backend + rust-lld" >&2
  exit 2
}

exclude_reason() {
  name="$1"
  src="$2"
  case "$name" in
    *MISALIGN_LDST*|*misalign-ldst*|*misalign_ldst*)
      echo "Zicclsm: misaligned loads/stores execute, they do not trap"
      return ;;
    *MISALIGN_JMP*|*misalign-jmp*|*misalign_jmp*)
      echo "jump targets must be 4-byte aligned; suite expects a privileged trap handler"
      return ;;
    *EBREAK*|*ebreak*)
      echo "ebreak is rejected at decode (not in the calc whitelist)"
      return ;;
    *ECALL*|*ecall*)
      echo "ecall a7=0 is HALT, not an environment-call trap with mtvec"
      return ;;
    *FENCE.I*|*fence.i*|*Zifencei*|*zifencei*)
      echo "fence.i is not in the calc whitelist (plain FENCE is a nop)"
      return ;;
  esac
  if grep -Eiq 'csrr|csrw|csrrs|csrrc|csrrwi|csrrsi|csrrci|mret|sret|mtvec|mcause|mepc|mstatus' "$src" 2>/dev/null; then
    echo "CSR / privileged instructions (no CSRs in calc)"
    return
  fi
  echo ""
}

ref_to_sig() {
  ref="$1"
  sig="$2"
  python3 - "$ref" "$sig" <<'PY'
import sys, pathlib, struct
ref, sig = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
out = bytearray()
for line in ref.read_text().splitlines():
    s = line.strip()
    if not s or s.startswith("#"):
        continue
    s = s.lower().replace("0x", "")
    if len(s) == 8:
        out += struct.pack("<I", int(s, 16))
    elif len(s) == 16:
        out += struct.pack("<Q", int(s, 16))
    else:
        sys.exit(f"unrecognized reference word {s!r} in {ref}")
sig.write_bytes(out)
PY
}

fetch
find_cc
mkdir -p "$OUT"
ENV="$SRC/riscv-test-suite/env"
if [ ! -f "$ENV/arch_test.h" ]; then
  echo "suite missing arch_test.h at $ENV" >&2
  exit 1
fi

# `.option rvc` + `.align` in the official headers emits C.NOP; both calc
# profiles reject compressed encodings. Force norvc for this harness only.
HDR="$OUT/hdr"
mkdir -p "$HDR"
for h in arch_test.h encoding.h test_macros.h; do
  if [ -f "$ENV/$h" ]; then
    sed 's/\.option[[:space:]]\{1,\}rvc/.option norvc/g' "$ENV/$h" > "$HDR/$h"
  fi
done

CFLAGS="-march=rv64im -mabi=lp64 -nostdlib -static -fno-pic -mno-relax -DXLEN=64 -DTEST_CASE_1= -I$PLUGIN/env -I$HDR -I$ENV"
LDFLAGS="-Wl,-n -Wl,--no-relax -Wl,-T,$PLUGIN/env/link.ld"

fits_mem() {
  python3 - "$1" <<'PY'
import sys
from pathlib import Path
b = Path(sys.argv[1]).read_bytes()
e_phoff = int.from_bytes(b[32:40], "little")
e_phentsize = int.from_bytes(b[54:56], "little")
e_phnum = int.from_bytes(b[56:58], "little")
code = ro = ini = 0
for i in range(e_phnum):
    o = e_phoff + i * e_phentsize
    if int.from_bytes(b[o:o+4], "little") != 1:
        continue
    flags = int.from_bytes(b[o+4:o+8], "little")
    filesz = int.from_bytes(b[o+32:o+40], "little")
    memsz = int.from_bytes(b[o+40:o+48], "little")
    if flags & 1:
        code = filesz
    elif flags & 2:
        ini = memsz
    else:
        ro = memsz
CODE_BASE = 0x1000
STACK, OUT_MAX = 2048, 4096  # no guard band: the stack starts at guest_end
CEIL = 16384  # CalcConfig::testkit hard ceiling, KiB

def fits(kib):
    mem_len = CODE_BASE + kib * 1024
    rodata_base = (CODE_BASE + code + 15) & ~15
    data_base = (rodata_base + ro + 15) & ~15
    stack_top = mem_len & ~15
    stack_bottom = max(0, stack_top - STACK)
    output_base = max(0, stack_bottom - OUT_MAX) & ~15
    input_base = output_base  # input_max = 0
    return input_base >= data_base + ini and input_base >= CODE_BASE

sys.exit(0 if any(fits(k) for k in range(4, CEIL + 1)) else 1)
PY
}

n_ok=0
n_ex=0
n_fail=0
for ext in I M; do
  srcdir="$SRC/riscv-test-suite/rv64i_m/$ext/src"
  refdir="$SRC/riscv-test-suite/rv64i_m/$ext/references"
  [ -d "$srcdir" ] || { echo "missing $srcdir" >&2; exit 1; }
  for src in "$srcdir"/*.S; do
    [ -f "$src" ] || continue
    base="$(basename "$src" .S)"
    name="${ext}_${base}"
    reason="$(exclude_reason "$base" "$src")"
    if [ -n "$reason" ]; then
      printf '%s\n' "$reason" > "$OUT/$name.exclude"
      echo "EXCLUDE $name: $reason"
      n_ex=$((n_ex + 1))
      continue
    fi
    elf="$OUT/$name.elf"
    # shellcheck disable=SC2086
    if ! $CC $CFLAGS $LDFLAGS -o "$elf" "$src" >/tmp/calc-archtest-cc.log 2>&1; then
      if grep -Eiq 'csrr|csrw|illegal|unrecognized|unknown instruction|fence.i|ebreak' /tmp/calc-archtest-cc.log; then
        reason="compile failed (CSR/privileged/unrecognized insn); see build log"
        printf '%s\n' "$reason" > "$OUT/$name.exclude"
        echo "EXCLUDE $name: $reason"
        n_ex=$((n_ex + 1))
        continue
      fi
      echo "FAIL compile $name" >&2
      cat /tmp/calc-archtest-cc.log >&2
      n_fail=$((n_fail + 1))
      continue
    fi
    if ! fits_mem "$elf"; then
      reason="image exceeds the calc-testkit 16MiB mem_kib ceiling (code+data would overlap io/stack)"
      printf '%s\n' "$reason" > "$OUT/$name.exclude"
      rm -f "$elf" "$OUT/$name.sig"
      echo "EXCLUDE $name: $reason"
      n_ex=$((n_ex + 1))
      continue
    fi
    rm -f "$OUT/$name.exclude"
    ref="$refdir/${base}.reference_output"
    if [ -f "$ref" ]; then
      ref_to_sig "$ref" "$OUT/$name.sig"
    fi
    echo "built $name"
    n_ok=$((n_ok + 1))
  done
done

echo "arch-test images: built=$n_ok exclude=$n_ex compile_fail=$n_fail -> $OUT"
[ "$n_fail" -eq 0 ]
echo "$OUT"
