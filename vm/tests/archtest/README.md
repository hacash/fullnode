# Official RISC-V architecture tests against the calc interpreter

The calc VM does **not** vendor `riscv-arch-test`. Fetch and compile the
suite yourself, then point the ignored harness at the resulting images.

## Fetch and build

```
# clones old-framework-2.x into target/riscv-arch-test (gitignored)
# unless CALC_ARCHTEST_SRC is set. 2.x still ships `*.reference_output`;
# default `act4` / 3.x moved those off-repo (Sail / Google Drive).
# writes images to target/calc-archtest-images unless CALC_ARCHTEST_DIR is set
sh vm/tests/archtest/build.sh
```

Needs a RISC-V toolchain (`riscv64-unknown-elf-gcc`, `riscv64-linux-gnu-gcc`,
or clang with a RISC-V backend + `rust-lld`) and network access for the first
clone. Compile flags: `-march=rv64im -mabi=lp64`, no C. The in-tree plugin
(`vm/tests/archtest/env/`) links at `CODE_BASE=0x1000` and implements
`RVMODEL_HALT` as calc `ecall a7=0`. The harness dumps guest memory between
ELF symbols `begin_signature` and `end_signature` (signatures exceed
`output_max=4096`) and compares against the suite `*.reference_output`.

The official headers use `.option rvc` for alignment nops; `build.sh` rewrites
those to `.option norvc` so the image stays 4-byte-only.

Suites: `riscv-test-suite/rv64i_m/I` and `.../M`. The C suite is not built
(both calc profiles reject C). Bitmanip (`B`) tests belong on profile 1 only.

## Run

```
CALC_ARCHTEST_DIR=/absolute/path/to/images cargo test -p vm --offline --features calc-testkit --test calc_archtest -- --ignored --nocapture
```

`CALC_ARCHTEST_DIR` must be absolute: cargo runs this integration test with
`vm/` as the working directory, so a workspace-relative path does not resolve.
After `build.sh`, the image directory is `target/calc-archtest-images` under
the workspace root (pass that absolute path).

The harness maps each `*.elf` (or `*.bin` + optional `*.dat`/`*.rodata`/
`*.entry`) as a profile-0 container (`input_max=0`, `output_max=4096`).
`mem_kib` is the smallest value whose layout clears that image, rounded up
to a KiB, and is written into the container header. It must be within the
testkit ceiling of 16384 KiB (16 MiB). Verify uses
`CalcConfig::testkit(16384)` (`calc-testkit`, non-default), which is
`all_enabled` capabilities plus that cap. Production builds cannot call
`testkit`; consensus v1 still caps `mem_kib` at 64. Layout, gas and the
interpreter follow the container's own `mem_kib` — there is no separate
usable-length override.

HALT output is compared against `<name>.sig` (bytes, or hex words from the
suite `*.reference_output`). Tests without a `.sig` only check HALT 0.

`*.exclude` files are printed as `EXCLUDE <name>: <reason>` — never
dropped silently. Branch/jal covergroups (`I_beq-01`, `I_bge-01`,
`I_bgeu-01`, `I_blt-01`, `I_bltu-01`, `I_bne-01`, `I_jal-01`) no longer
need an exclude: their `mem_kib` is above 64 and they run under
`CalcConfig::testkit`. An image that still exceeds the 16 MiB ceiling is
excluded by `build.sh`. CSR /
`ebreak` / `ecall`-as-trap / `fence.i` / misaligned-trap tests live in
other suite folders (privilege, Zifencei) and are not part of I/M.

`act4` / 3.x do not ship `*.reference_output` in the git tree (Sail or a
Google Drive dump). This harness does not run that suite unless those
signatures are already available without installing Sail.

This is a conformance aid, not a consensus test.
