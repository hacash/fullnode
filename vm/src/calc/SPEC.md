# Hacash calc VM specification

Normative semantics for the HVM calc-func RISC-V micro-VM. The x86-64
AOT (`calc-aot`) is the **normative consensus engine**; the interpreter
is launch-time bug insurance that must stay bit-identical to it (outputs,
register file, memory, cycle totals, abort class, image ID and receipt)
and is removed once production is stable — see "v1 semantics". This
channel is not yet activated on mainnet; the wire format here is **v1**.

Three layers:

1. **Off-chain ELF** — static LP64 RISC-V, produced by a mature toolchain.
2. **On-chain container** — lossless projection of ELF `PT_LOAD` (this wire).
3. **Loaded machine** — RV64IM(+C)(+B), flat memory, HALT trampoline, host
   window at the top of `mem_kib`; profile 0 additionally is the machine a
   zk prover can prove with a standardized circuit.

Target compatibility: profile byte = three additive ISA bits (bit 0 B, bit 1
RVC, bit 2 HASH). Profile 0 matches the Ethereum zkVM baseline
`riscv64im_zicclsm-unknown-none-elf` (eth-act/zkvm-standards): RV64I + M +
Zicclsm, little-endian, flat memory, **no C, no B, no F/D, no A, no OS
syscalls**. **v1 admits every B / RVC combination (bytes 0..=3) and rejects
the HASH bit**; zk portability is a per-profile property — only profile 0 is
the zk-standardized subset, and nothing requires every profile to be
zk-provable.

## 1. Container wire format

20-byte big-endian header, then `nseg × 16` segment headers, then payloads
in table order. Magic `HCF1`. There is no checksum; consensus commits the
bytes. The ABI is fixed by `version = 1` (bytes → bytes). Bytes
`[0, 16)` are the chain-policy prefix (magic through `output_max`). The
ELF projection starts at `entry` (offset 16). `.bss` is `memsz - filesz`
and is **not** stored.

| Offset | Size | Field |
|--------|------|--------|
| 0 | 4 | magic `HCF1` |
| 4 | 1 | version = 1 |
| 5 | 1 | `profile` — ISA bits (see §2); bit 2 (HASH) must be 0 |
| 6 | 1 | `nseg` (1..=16) |
| 7 | 1 | reserved, must be 0 |
| 8 | 4 | `mem_kib` (u32, `decode` requires 4..=2097147) |
| 12 | 2 | `input_max` (u16) |
| 14 | 2 | `output_max` (u16, at least 1) |
| 16 | 4 | `entry` — **absolute guest PC** |

Each segment header (16 bytes, big-endian):

| Offset | Size | Field |
|--------|------|--------|
| 0 | 4 | `vaddr` |
| 4 | 4 | `filesz` |
| 8 | 4 | `memsz` (`>= filesz`) |
| 12 | 1 | `flags` (ELF `PF_*`: X=1, W=2, R=4) |
| 13 | 3 | reserved, must be 0 |

Then `nseg` payloads of `filesz` bytes each. `memsz - filesz` is zero-filled
at load. Executable segments require `filesz == memsz` (no executable BSS).
W^X: a segment cannot be both W and X. Every segment is readable. Segments
are sorted, non-overlapping, `vaddr >= CODE_BASE`, and end at or below the
host window (`guest_end`). Holes between segments are **unmapped**,
including instruction fetch: sequential execution follows `pc + inst_len`,
not the concatenated RX stream.

`entry` is an absolute guest PC inside some executable segment, on an
instruction boundary (4-byte, or 2-byte on RVC profiles).

Chain deploy admits the encoded container under `SpaceCap::calc_image_max`
(default 16383) with a `u16` body prefix, so the stored image cannot exceed
65535 bytes. A sidechain may raise `calc_image_max` up to 65535.

## 2. ISA profiles

Header byte 5 is the profile id — three additive bits:

| Bit | Constant | Meaning |
|-----|----------|---------|
| 0 | `CALC_PROFILE_BITMANIP` | Zba/Zbb/Zbs/Zbc encodings are part of the image |
| 1 | `CALC_PROFILE_COMPRESSED` | RVC (16-bit) encodings are part of the image |
| 2 | `CALC_PROFILE_HASH` | hash ecalls (`a7` = 1..=6) are part of the image |

| Value | Constant | ISA | v1 admission |
|-------|----------|-----|--------------|
| 0 | `CALC_PROFILE_RV64IM` | RV64I + M + Zicclsm | **admitted** — the zk-standard subset (eth-act baseline) |
| 1 | `CALC_PROFILE_RV64IM_B` | + Zba/Zbb/Zbs/Zbc | **admitted** — outside the zk-standard subset |
| 2 | `CALC_PROFILE_RV64IMC` | + RVC | **admitted** — outside the zk-standard subset |
| 3 | `CALC_PROFILE_RV64IMC_B` | + RVC + bitmanip | **admitted** — outside the zk-standard subset |
| 4..=7 | — | any + HASH | **rejected** — hash activation is its own hard fork with its own pricing table |

zk portability is declared by the profile byte itself, not by a separate
switch: profile 0 is the only profile whose ISA subset is zk-standardized,
and the system never requires every admitted profile to be zk-provable.
Every profile shares one gas weight table; a future repricing may only
attach a new table to a NEW profile value (§8).

The HALT trampoline is always plain RV64I (profile 0 stream), regardless of
the guest profile.

### Base whitelist (every profile)

RV64I: LUI, AUIPC, JAL, JALR, BEQ/BNE/BLT/BGE/BLTU/BGEU, LB/LH/LW/LD/LBU/LHU/LWU,
SB/SH/SW/SD, ADDI/SLTI/SLTIU/XORI/ORI/ANDI, SLLI/SRLI/SRAI, ADD/SUB/SLL/SLT/SLTU/XOR/SRL/SRA/OR/AND,
ADDIW/SLLIW/SRLIW/SRAIW, ADDW/SUBW/SLLW/SRLW/SRAW, FENCE (nop), ECALL,
EBREAK and canonical UNIMP (both `Inst::Trap`, 1 cycle, then trap).

RV64M: MUL/MULH/MULHSU/MULHU/DIV/DIVU/REM/REMU and W-forms.

### Bit 0 (B): Zba/Zbb/Zbs/Zbc

Admitted when bit 0 is set; a profile-0 image containing these encodings
fails verify. `add.uw`, `sh1add`/`sh2add`/`sh3add` and `.uw` forms,
`andn`/`orn`/`xnor`, `rol`/`ror`/`rolw`/`rorw`/`rori`/`roriw`,
`min`/`minu`/`max`/`maxu`, `clz`/`ctz`/`cpop`/`clzw`/`ctzw`/`cpopw`,
`sext.b`/`sext.h`/`zext.h`, `orc.b`/`rev8`, `bset`/`bclr`/`binv`/`bext`
and immediate forms, `slli.uw`, `clmul`/`clmulh`/`clmulr`.

### Bit 1 (C): RVC

Admitted when bit 1 is set; executable segments then align to 2 bytes and
entries may be 2-byte aligned. `c.addi4spn`, `c.lw`/`c.ld`/`c.sw`/`c.sd`,
`c.addi`/`c.nop`/`c.addiw`/`c.li`, `c.addi16sp`/`c.lui`, `c.srli`/`c.srai`/
`c.andi`, the `c.sub`/`c.xor`/`c.or`/`c.and`/`c.subw`/`c.addw` group,
`c.j`, `c.beqz`/`c.bnez`, `c.slli`, `c.lwsp`/`c.ldsp`/`c.jr`/`c.mv`/
`c.jalr`/`c.add`, `c.swsp`/`c.sdsp`. `c.ebreak` (`0x9002`) and `c.unimp`
(the all-zero halfword) decode to `Inst::Trap`: 1 cycle, then trap. Jump
and link addresses use the encoded length (2 or 4). Without bit 1, every
compressed encoding (including the all-zero halfword) is rejected.

### Bit 2 (HASH): not admitted

`profile & 0b100 != 0` is rejected at container admission, alone or
combined. Hash ecalls (`a7` = 1..=6) therefore always trap on v1 images,
charging the static ecall cost only; the implementations and price table
stay in the tree for a future profile value.

### Bits 3..=7: reserved and disabled

Profile values 8..=255 decode to nothing: `profile_known` requires the
byte to be a subset of bits 0..=2, so `decode` rejects every value with
any of bits 3..=7 set ("unknown isa profile"). They are **disabled, not
planned**: no behavior is assigned to them in v1. A future hard fork may
first **define** one of these values — a definition names the value's ISA
membership and, per §8, attaches that value's own pricing table — and
only then does admission grow to accept containers carrying it. Until
such a definition exists, every value with bits 3..=7 set is rejected,
and no already-admitted value (0..=3) can ever be redefined.

## 3. Memory map

`CODE_BASE = 0x1000`. `HALT_STUB = 0x0FF0` (16 bytes of real RV64I, always
decoded as profile 0): `mv a1,a0; li a0,0; li a7,0; ecall`.
`HALT_STUB + 16 == CODE_BASE`; there is no gap between the stub and
guest IMAGE. Addresses `[0, HALT_STUB)` are **unmapped**.

`mem_kib` sizes the usable region **above** `CODE_BASE`:

```
usable_len = mem_kib * 1024
mem_len    = CODE_BASE + usable_len     // exclusive end of guest space
```

Resource and setup charges use `mem_kib` / `usable_len`. The interpreter
allocates exactly `mem_len` bytes.

`decode` accepts any `mem_kib` from 4 through `CALC_MEM_KIB_ENGINE_MAX`
(2_097_147 KiB) whose regions do not overlap. The ceiling is a frozen
consensus constant of the **loaded machine**, not an engine implementation
detail: the whole guest memory must satisfy
`CODE_BASE + mem_kib * 1024 <= 0x7FFF_FFFF`, so every address the machine
can form fits a signed 32-bit integer. That address width is part of the
architecture — a zk circuit, an independent re-implementation and the AOT
alike standardize on it — and no engine may widen or narrow it on its own.
`verify_container_with_cap` rejects `mem_kib > cap.calc_mem_kib_max`.
`SpaceCap::calc_mem_kib_max` defaults to 64. Layout always follows the
container's own `mem_kib`.

Layout (absolute addresses):

```
[0, HALT_STUB)                              unmapped
[HALT_STUB, HALT_STUB+16)                   HALT trampoline  RX (injected, not in the wire)
each PT_LOAD [vaddr, vaddr+memsz)           guest segment, flags as declared
holes between segments                      unmapped (load, store, **and fetch**)
[guest_end, guard_end)                      stack-overflow guard — unmapped, any access traps
[stack_bottom, stack_top)                   stack  RW, grows down; sp = stack_top
[output_base, output_base+output_max)       output RW
[input_base, input_base+input_max)          input  RW
[input_base+input_max, mem_len)             tail   RW
stack_top    = output_base
stack_bottom = guard_end = guest_end + CALC_STACK_GUARD
guest_end    = max(segment ends), or CODE_BASE if empty
```

There is a **stack-overflow guard band**: `[guest_end, guest_end +
CALC_STACK_GUARD)` is unmapped, and any load or store that intersects it
traps. A call frame whose writes cross the image end therefore fails
loudly instead of silently corrupting guest data. The band is not complete
protection — a single frame larger than the band can move `sp` past it in
one step and land inside a writable guest segment — so pack tools still
verify frame budgets against the usable stack (`fitshc` warns when the
declared window leaves a thin stack).

Host IO/stack sit at the **top** of `mem_kib`. `output_base =
align16_down(input_base - output_max)`. `input_base =
align16_down(mem_len - input_max)`. The stack is every byte between the
guard band and the output region. Of the mandatory free span,
`CALC_STACK_GUARD` (2 KiB) is the trap band and `CALC_STACK_SIZE` (2 KiB)
is the minimum usable stack. The fit rule is `guest_end +
CALC_MIN_FREE_RW_GAP <= stack_top`, where `CALC_MIN_FREE_RW_GAP =
CALC_STACK_GUARD + CALC_STACK_SIZE`.

Stores into unmapped holes or RX/RO segments trap. Loads/stores/fetches of
unmapped holes trap. Fall-through from an RX segment into a hole is a fetch
trap, not execution of the next concatenated instruction.

## 4. Misaligned access (Zicclsm)

Loads and stores at **any alignment** execute correctly: little-endian,
byte-wise copy of the accessed width. Bounds and permission checks apply
to the **full** `[addr, addr+size)` range. There is **no extra cycle
charge** for misalignment. Instruction fetch and jump targets must be
instruction boundaries. `jalr` still clears bit 0 of the target.

## 5. Register state at entry

All 32 registers are 0 except:

| Reg | Value |
|-----|--------|
| `ra` (x1) | `HALT_STUB` |
| `sp` (x2) | `stack_top` |
| `a0` (x10) | `input_base` |
| `a1` (x11) | input length in bytes |
| `a2` (x12) | `output_base` |
| `a3` (x13) | `output_max` |

This is the LP64 `extern "C"` four-argument ABI. Returning from
`calc_entry` (`ret` / `jalr x0, ra, 0`) lands on the trampoline: `a0`
(returned length) is moved to `a1`, exit code is 0, then HALT. The stub
bytes are part of the RISC-V trace, so a zk proof of `return n` is a
proof of those four instructions plus HALT.

## 6. ecall ABI

Syscall number in `a7`.

| a7 | Name | Arguments | Result |
|----|------|-----------|--------|
| 0 | HALT | `a0` = exit code, `a1` = output length | stop |
| 1 | SHA-256 | `a0` src, `a1` len, `a2` dst | `a0 = 0`; 32-byte digest at dst |
| 2 | SHA3-256 | same | 32 bytes |
| 3 | RIPEMD-160 | same | 20 bytes |
| 4 | Keccak-256 | same | 32 bytes |
| 5 | BLAKE2s-256 | same | 32 bytes |
| 6 | BLAKE2b-256 | same | 32 bytes |
| 7..=15 | reserved | | trap |
| 16..=31 | HINT / private input | | **trap** |
| ≥ 32 | unknown | | trap |

HALT always works. Hash ecalls (`a7` = 1..=6) **trap** (static ecall cost
only). The implementations stay in the tree for a later profile id; v1
does not admit HASH. Bounds checks use the same permission map as loads
(the HALT stub is readable).

HALT with `a0 == 0` is success: the host copies `a1` bytes from the output
region (`<= output_max`). HALT with `a0 != 0` is **Trap**
(`CalcAbort2::Trap`), receipt `exit_code = a0`, empty output. Cycles
consumed so far are still charged. Write the output **before** HALT (or
before `return`).

## 7. Traps

Deterministic Trap-class aborts (cycles consumed up to the failing
instruction's block charge, excluding a charge that does not fit):

- unmapped address (below `HALT_STUB`, the gap below `CODE_BASE`, holes
  between segments, the stack guard band `[guest_end, guest_end +
  CALC_STACK_GUARD)`, or `>= mem_len`)
- load/store/ecall address overflow
- store / digest into a non-writable region (RX/RO, HALT stub)
- `jal`/`jalr`/`branch` target not an instruction boundary in an RX
  segment or the HALT stub
- `ebreak` / canonical `unimp` / `c.ebreak` / `c.unimp` (`Inst::Trap`)
- hash ecall (`a7 = 1..=6`)
- HINT ecall / unknown ecall
- output length > `output_max` on HALT
- guest HALT with nonzero exit code
- input longer than `input_max` (pre-flight, 0 cycles)

OutOfGas: remaining budget cannot cover the next prepaid block, the hash
ecall cost, or the setup charge. The setup shortfall reports
`cycles_used = setup` and is a terminal failure.

**OutOfResource is pre-flight only (0 cycles).** The resource charge of a
call is fully determined before it runs: `ceil(input_len/16)` +
`ceil(output_max/16)` + `2 per declared KiB`. There is no post-run
resource check. The returned `resource` field still charges the *actual*
output length.

## 8. Gas

`CYCLES_PER_COMPUTE = 56` (a chosen policy value, not a unit derivation).
On the 2026-11-04 warm same-machine calibration (HVM ~33 ns/compute, AOT
~0.38 ns/cycle) AOT parity is N ≈ 87, so N=56 is roughly +55% over AOT
hardware cost. Against the interpreter/lockstep it remains a large
discount (roughly 7x). The 200,863-cycle HIP5 kernel costs 3,587 compute
at N=56. N is consensus-frozen: any later change is a hard fork. Setup
(always kept, even on trap):

```
setup = CALC_SETUP_BASE_CYCLES + mem_kib * CALC_SETUP_PER_KIB_CYCLES
      = 64 + mem_kib * 16
```

Resource: `ceil(input_len/16) + ceil(output_len/16) + mem_kib * 2`.
Aborted runs charge 0 for output length.

The resource pre-flight reserves the declared `output_max` on the same
formula (see §7); this is what makes the charge known before execution.

HVM compute gas and zk cycle counts are different meters. Consensus
charges the table in this section. A zk proof of the same image uses the
prover's own cycle accounting.

### Static weights

One table (`decode::WEIGHTS`) serves every profile: repricing a profile
after activation would change the gas of already-deployed containers, so a
future price change may only attach a new table to a NEW profile value,
never reprice 0..=7.

| Class | Cycles |
|-------|--------|
| ALU, branches, jumps, FENCE, unary/bitmanip except clmul | 1 |
| load / store | 2 |
| MUL family | 3 |
| DIV/REM family | 12 |
| CLMUL family | 24 |
| ECALL static (`ecall_base`) | 1 (hash cost added at runtime when HASH is set) |

### Basic-block prepaid charging

The verifier splits code at every JAL/JALR/branch/ECALL/Trap (terminator)
and at the instruction after a terminator. Index 0 of the instruction
stream is the injected HALT stub (always uncompressed). Guest RX segments
follow. Entering a block charges `suffix_cycles[idx]`: the sum of static
weights from the **entry instruction** to the block end. A `jalr` into the
middle of a block therefore does **not** pay for skipped prefix
instructions.

On trap or OOG: if the remaining budget cannot cover the next suffix
charge, abort OutOfGas **without** taking that charge. A trap after a
successful charge keeps the charge. Hash ecall extra cost is charged
inside the ecall, after the static suffix has already been paid.

## 9. Image ID and receipt

```
image_id = SHA-256( b"hacash-calc-image-v1" || encode(container) )
```

`CalcContainer::image_id()` and `CalcFnObj::image_id()` return this. The
object hashes it on first use (`OnceLock`), never per call. If `encode`
fails, the hash covers the domain plus the sentinel `\xffencode-overflow`
instead of the container bytes.

The call receipt is a **derived artifact**, not part of consensus
execution. `CalcFnObj::run` returns `CalcExec { result, cycles_used,
resource, exit_code }` only. Callers that need a proof statement invoke
`CalcFnObj::receipt(&self, input, exec)`:

```
image_id, input_hash = SHA-256(input),
output_hash = SHA-256(output) on success else SHA-256([]),
exit_code, abort (None on success), cycles_used
```

Identical between interpreter and AOT when derived from equal `CalcExec`.

## v1 semantics (unconditional)

**Normative engine.** The x86-64 AOT (`calc-aot`) is the canonical
consensus engine, permanently. The interpreter is **temporary launch-time
bug insurance only**: it exists to catch AOT defects during the initial
period after activation, and once production has run quiet it is removed
and the AOT becomes the sole normative engine. That removal is not a
semantics change — the two engines are held bit-identical on the entire
observable surface (below), so consensus outputs are the same with and
without the interpreter.

Today, official x86-64 nodes build with `calc-aot` (the vm crate default).
Each call runs the interpreter and then AOT in order on the calling thread.
The call is kept only when `cycles_used`, the output bytes, the abort class
and the halt code match. A mismatch is `CalcEngineMismatch` (fault 156):
the transaction is rejected and does not enter a block. Builds without the
feature, or with the image unmounted, run the interpreter only.

The lockstep compares the **consensus-observable** surface (output bytes,
abort class, exit code, cycles) AND the full stop state — all 32 registers
and the whole memory image — so an engine bug that happens to agree on the
consensus surface alone is still surfaced while both engines run (this
caught a backend that skewed every load/store address by its own width).

A panic inside either engine is reported as `EngineMismatch` under unwind
builds (the test profiles). The workspace release profile sets
`panic = "abort"`, where a panic terminates the process before any handler
runs; the supervisor restarts from durable state.

A host wall-clock deadline is observed by the interpreter only. The AOT
image **still runs**; a deadline stop is host policy, so the AOT verdict is
not compared and the call reports `Deadline`.

Every execution capability is unconditional. Chain admission is the
`SpaceCap` fields that already bound a container: `calc_mem_kib_max`,
`calc_image_max`, `call_data_size` (against `input_max`) and `value_size`
(against `output_max`). No legal future value of those fields can turn a
capability off, because that would reject or reinterpret a previously legal
container:

- Header byte 5: every B / RVC combination (0..=3) is admitted; the HASH
  bit is rejected.
- HALT (`a7` = 0) always works. Hash ecalls trap.
- Guest segments have the permissions they declare. Holes are unmapped
  for load, store, and fetch. The HALT stub is RX. The stack guard band
  `[guest_end, guest_end + CALC_STACK_GUARD)` is unmapped (any access
  traps); the stack region `[guest_end + CALC_STACK_GUARD, stack_top)` is
  RW.
- Any instruction-boundary entry inside an RX segment is accepted
  (4-byte boundaries, or 2-byte on RVC profiles).

There is no consensus config struct and no height-dependent execution
config. Admission grows by raising the current `SpaceCap` (or, for ISA
surface, by admitting a new profile value — never by reinterpreting an
existing one). The layout constants (`CODE_BASE`, `HALT_STUB`,
`CALC_STACK_GUARD`, `CALC_STACK_SIZE`, `CALC_MIN_FREE_RW_GAP`,
`CALC_MEM_KIB_MIN`) are frozen: after activation `CALC_STACK_GUARD` and
`CALC_MIN_FREE_RW_GAP` cannot change in **either** direction (raising
would reject legal containers; lowering would turn traps back into silent
writes or cut the guaranteed minimum stack), and the others may only be
relaxed.

## 10. Building a kernel

One recipe: static ELF that `pack_elf` projects as profile 0.

```
RUSTC_BOOTSTRAP=1 cargo build -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --release \
  --target riscv64im_zicclsm.json
RUSTFLAGS="-C relocation-model=static -C link-arg=-Tcontainer.ld -C link-arg=-n"
```

(`zicclsm` is a memory-model convention — misaligned loads/stores — not an
LLVM feature flag.) Linker script `MEMORY { IMAGE }` starts at `CODE_BASE`
(`0x1000`) and ends before the L1 host reserve (12 KiB under a 64 KiB
`mem_kib` cap). It emits `PT_LOAD` via `PHDRS`, linked with `-n`
(`--nmagic`) so segments are not page-aligned. Do not `ALIGN` inside
`.text` with zero fill: profiles without RVC reject compressed encodings,
including 4-byte zeros. Entry is `calc_entry`; that symbol (else
`e_entry`) is the absolute `entry` written into the header. Section headers
are optional; a stripped PT_LOAD-only `ET_EXEC` packs via `e_entry`.
`ET_DYN` is rejected.

The generated wrapper is `extern "C" fn calc_entry(a0, a1, a2, a3) -> u64`.
It returns the output length. `ra` is the HALT trampoline, so a compiled
`ret` is a successful HALT. Overflow of `out.len()` is a 32-bit `ebreak`.

`pack_elf` rejects `PT_DYNAMIC` / `PT_INTERP` / `PT_TLS`, `SHT_REL` /
`SHT_RELA` when present, `EF_RISCV_RVC` on a no-C profile, W^X, and
executable BSS. Pack tools use `pack_elf_min_mem`: the smallest `mem_kib`
that fits `guest_end + CALC_STACK_GUARD + CALC_STACK_SIZE + the declared
IO regions`.

## 11. RISC-V architecture tests

Two external conformance layers back the engine:

- `vm/tests/archtest/README.md` — the riscv-arch-test harness
  (`vm/tests/calc_archtest.rs`, `#[ignore]` until `CALC_ARCHTEST_DIR` is
  set; every image maps as a profile-0 container). Do not vendor the suite.
- `vm/tests/calc_riscvtests.rs` — the committed riscv-tests corpus
  (`rv64ui` + `rv64um`, 65 ELF images under
  `vm/tests/fixtures/riscvtests/elf/`, sources not vendored), each run
  through the production pack path and the engine lockstep.

The ISA generator suites (`calc_isa_suite.rs` + `calc_isa_gen.rs`) pin
host-reference semantics for the base, bitmanip (profile B) and
compressed (profile C / C+B) surfaces on top of these.
