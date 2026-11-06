#!/bin/sh
# Build the calc kernel fixture for both targets:
#  - container image (text+rodata as code, .data as init) -> image_code.bin / image_init.bin
#  - qemu-user oracle ELF -> oracle.elf (run: qemu-riscv64-static oracle.elf)
#
# Profile 0 is RV64IM + Zicclsm (no C). rustc has no tier-2 `riscv64im`
# target, and `riscv64imac` plus `-C target-feature=-c` still links
# prebuilt compiler-builtins that contain C. The custom target JSON
# plus `-Zbuild-std=core` rebuilds core without C.
set -e
cd "$(dirname "$0")"
# rustc does not hash -T scripts; touch so a linker-script edit relinks.
touch src/main.rs
OBJCP=$(ls ~/.rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objcopy | head -1)
NM=$(ls ~/.rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-nm | head -1)
TARGET="$(pwd)/riscv64im_zicclsm.json"
# container image
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C link-arg=-Tcontainer.ld -C link-arg=-n -C relocation-model=static" \
  cargo build -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --release --target "$TARGET"
ELF=target/riscv64im_zicclsm/release/calckernel
cp "$ELF" kernel.elf
$OBJCP -O binary --only-section=.text $ELF image_code.bin
$OBJCP -O binary --only-section=.rodata $ELF image_rodata.bin
$OBJCP -O binary --only-section=.data $ELF image_init.bin
$NM $ELF | awk '$3=="calc_entry"{printf "0x%s\n", $1}' > entry.txt
OBJDUMP=$(ls ~/.rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objdump | head -1)
python3 - "$OBJDUMP" "$ELF" <<'PY'
import os, subprocess, re, sys
objdump, elf = sys.argv[1], sys.argv[2]
out = subprocess.check_output([objdump, "-h", elf], text=True)
secs = {}
for line in out.splitlines():
    m = re.match(r"\s+\d+\s+(\S+)\s+([0-9a-fA-F]+)\s+([0-9a-fA-F]+)", line)
    if m:
        secs[m.group(1)] = (int(m.group(2), 16), int(m.group(3), 16))
text = secs.get(".text")
if not text:
    sys.exit("layout check: missing .text")
text_size, text_vma = text
if text_vma != 0x1000:
    sys.exit(f"layout check: .text VMA {text_vma:#x} != 0x1000")
ro_size, ro_vma = secs.get(".rodata", (0, 0))
rodata_base = (0x1000 + text_size + 15) & ~15
data_base = (rodata_base + ro_size + 15) & ~15
if ro_size and ro_vma != rodata_base:
    sys.exit(f"layout check: .rodata VMA {ro_vma:#x} != {rodata_base:#x}")
data = secs.get(".data")
bss = secs.get(".bss")
if data and data[0]:
    start = data[1]
    name = ".data"
elif bss and bss[0]:
    start = bss[1]
    name = ".bss"
else:
    start = None
    name = None
image_end = 0x1000 + 0xD000
bss_end = 0
for bname in (".bss", ".sbss"):
    if bname in secs and secs[bname][0]:
        bss_end = max(bss_end, secs[bname][1] + secs[bname][0])
if bss_end > image_end:
    sys.exit(f"layout check: .bss end {bss_end:#x} exceeds IMAGE {image_end:#x}")
print(f"layout ok: .text@0x1000 kernel.elf (bss stays off-chain, end={bss_end:#x})")
PY
# qemu oracle ELF (with `_start` harness)
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C link-arg=-Tqemu-oracle.ld -C relocation-model=static" \
  cargo build -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --release --features oracle --target "$TARGET"
cp target/riscv64im_zicclsm/release/calckernel oracle.elf
echo "built: kernel.elf image_code.bin image_rodata.bin image_init.bin entry.txt oracle.elf"
