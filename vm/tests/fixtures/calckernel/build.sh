#!/bin/sh
# Build the calc kernel fixture for both targets:
#  - container image (text+rodata as code, .data as init) -> image_code.bin / image_init.bin
#  - qemu-user oracle ELF -> oracle.elf (run: qemu-riscv64-static oracle.elf)
set -e
cd "$(dirname "$0")"
OBJCP=$(ls ~/.rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-objcopy | head -1)
NM=$(ls ~/.rustup/toolchains/*/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-nm | head -1)
# container image
RUSTFLAGS="-C link-arg=-Tcontainer.ld -C relocation-model=static" \
  cargo build --release --target riscv64imac-unknown-none-elf
ELF=target/riscv64imac-unknown-none-elf/release/calckernel
$OBJCP -O binary --only-section=.text $ELF image_code.bin
$OBJCP -O binary --only-section=.rodata $ELF image_rodata.bin
$OBJCP -O binary --only-section=.data $ELF image_init.bin
$NM $ELF | awk '$3=="calc_entry"{printf "0x%s\n", $1}' > entry.txt
# qemu oracle ELF (with `_start` harness)
RUSTFLAGS="-C link-arg=-Tqemu-oracle.ld -C relocation-model=static" \
  cargo build --release --features oracle --target riscv64imac-unknown-none-elf
cp target/riscv64imac-unknown-none-elf/release/calckernel oracle.elf
echo "built: image_code.bin image_rodata.bin image_init.bin entry.txt oracle.elf"
