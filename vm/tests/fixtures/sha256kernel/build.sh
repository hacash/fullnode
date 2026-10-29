#!/bin/sh
# Build the sha256 kernel fixture: container image + qemu oracle ELF.
set -e
cd "$(dirname "$0")"
TOOLS=$HOME/.rustup/toolchains/1.92.0-x86_64-unknown-linux-gnu/lib/rustlib/x86_64-unknown-linux-gnu/bin
RUSTFLAGS="-C link-arg=-Tcontainer.ld -C relocation-model=static" \
  cargo build --release --target riscv64imac-unknown-none-elf
ELF=target/riscv64imac-unknown-none-elf/release/sha256kernel
$TOOLS/llvm-objcopy -O binary --only-section=.text $ELF image_code.bin
$TOOLS/llvm-objcopy -O binary --only-section=.rodata $ELF image_rodata.bin
$TOOLS/llvm-objcopy -O binary --only-section=.data $ELF image_init.bin
$TOOLS/llvm-nm $ELF | awk '$3=="calc_entry"{printf "0x%s\n", $1}' > entry.txt
RUSTFLAGS="-C link-arg=-Tqemu-oracle.ld -C relocation-model=static" \
  cargo build --release --features oracle --target riscv64imac-unknown-none-elf
cp target/riscv64imac-unknown-none-elf/release/sha256kernel oracle.elf
echo "built sha256kernel fixtures"
