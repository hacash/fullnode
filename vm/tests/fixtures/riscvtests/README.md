# riscv-tests conformance corpus for the calc VM

Prebuilt RV64 test images from the upstream riscv-tests ISA suites,
executed by `vm/tests/calc_riscvtests.rs` through the production pack path
(`pack_elf_min_mem` → verify → CalcFnObj build → run, with the full-state
interpreter/AOT lockstep under `calc-aot`):

- `rv64ui` + `rv64um` — RV64I/M (profile 0)
- `rv64uzbb` / `rv64uzba` / `rv64uzbs` / `rv64uzbc` — bitmanip
  (profile B = Zba/Zbb/Zbs/Zbc)
- `c_*` — the RV64I/M suites rebuilt with `-march=rv64imc`
  (profile C: clang auto-compresses them into RVC forms)
- `rvc_forms` / `rvc_ebreak` / `rvc_unimp` — calc-native kernels
  (sources in this directory, see below)

## Provenance

- Upstream sources are **not vendored** (same policy as
  `vm/tests/archtest`): https://github.com/riscv-software-src/riscv-tests
  (commit `3df703cd`, MIT/BSD-style upstream license) +
  https://github.com/riscv/riscv-test-env.
- Built with `./build.sh /path/to/riscv-tests` using clang 18
  (`--target=riscv64-unknown-elf -mno-relax`, `-march=` per suite above)
  and lld (rust-lld symlinked as `ld.lld`), linked by `calc-tests.ld` at
  CODE_BASE 0x1000 with an RX image and a separate RW image. The
  name→profile mapping is written to `elf/profiles.txt`.

## calc adaptation

`riscv_test.h` (this directory) replaces the upstream M-mode environment:
no CSRs, no tohost. A test HALTs via the calc ecall ABI —
`a7=0, a0=exit code, a1=0`. Pass exits 0; a failed case exits with its
upstream test number, which the harness reports from
`CalcExec::exit_code` (GuestExit).

## calc-native RVC kernels

Upstream `rvc.S` is excluded (see below), so the dedicated per-form RVC
conformance duty falls to three kernels assembled from THIS directory:

- `rvc_forms.S` — one checked case per whitelisted compressed form (all
  32 non-trap RV64C forms: c.addi4spn/c.ld/c.lw/c.sd/c.sw/c.nop/c.addi/
  c.addiw/c.li/c.addi16sp/c.lui/c.srli/c.srai/c.andi/c.sub/c.xor/c.or/
  c.and/c.subw/c.addw/c.j/c.beqz/c.bnez/c.slli/c.swsp+c.lwsp/c.sdsp+
  c.ldsp/c.jr/c.mv/c.jalr/c.add). Each case compares its result against
  the expected value and exits with the case number on mismatch.
- `rvc_ebreak.S` — C.EBREAK trap probe: must abort with `CalcAbort2::Trap`.
- `rvc_unimp.S` — the all-zero halfword (canonical c.unimp, which LLVM
  emits on dead paths): admitted as a Trap instruction, ends execution.

Execution-level completeness is proven by `vm/tests/calc_isa_forms.rs`:
the interpreter's per-instruction PC trace is classified per form —
all 43 bitmanip forms must be executed by the 43 profile-B images, and
all 34 admitted RV64C encodings by the 68 profile-C images (the
`rvc_forms` kernel alone deterministically covers the 32 non-trap forms,
so the guarantee does not depend on what clang happens to compress).

## Excluded from the build

- `fence_i.S` — `fence.i` is rejected by the profile-0 decoder.
- `ma_data.S` — expects M-mode misaligned-access trap fixups; the calc VM
  is Zicclsm (misaligned accesses execute natively, no traps).
- `rv64uc/rvc.S` — its first case embeds `.dword` data inside the
  executable segment (a 4-byte-fetch-across-page-boundary probe). The
  calc verifier deliberately requires every X-segment byte to decode to a
  whitelisted instruction, so that image cannot pack; the calc-native
  kernels above take over its form-coverage role. This is a verifier
  design tradeoff, not a bug.

Every other upstream image of the built suites is in `elf/` — together
they execute every opcode form of the admitted whitelist (see the
manifest test `corpus_covers_the_whole_admitted_whitelist`).
