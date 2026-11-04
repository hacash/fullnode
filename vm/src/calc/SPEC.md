# Hacash calc VM specification

Normative semantics for the HVM calc-func RISC-V micro-VM. Interpreter and
x86-64 AOT (`calc-aot`) **must be bit-identical** on outputs, register file,
memory, cycle totals, abort class, image ID and receipt. This channel is not
yet activated on mainnet; the wire format here is v1.

Target compatibility: **profile 0** matches Ethereum zkVM
`riscv64im_zicclsm-unknown-none-elf` (eth-act/zkvm-standards): RV64I + M +
Zicclsm, little-endian, flat memory, **no C, no F/D, no A**.

## 1. Container wire format

32-byte big-endian header, then `code || rodata || init`. Magic `HCF1`.
There is no checksum; consensus commits the bytes. There is no ABI byte and
no `bss` field: the ABI is fixed by `version = 1` (bytes -> bytes), and
zero-initialized data is part of the `init` image (see §3).

| Offset | Size | Field |
|--------|------|--------|
| 0 | 4 | magic `HCF1` |
| 4 | 1 | version = 1 |
| 5 | 1 | `profile` bitfield. Bit 0 = Zba/Zbb/Zbs/Zbc. Bit 1 = RVC. Bit 2 = no host ecalls (HALT only). Other bits rejected. Values 0..=7 |
| 6 | 2 | reserved, must be 0 |
| 8 | 4 | `mem_kib` (u32, `decode` requires 4..=2097147; the chain cap comes from `SpaceCap::calc_mem_kib_max`) |
| 12 | 4 | `entry` — **absolute guest PC** |
| 16 | 4 | `code_len` (> 0; multiple of 4 when bit 1 is clear, multiple of 2 when RVC is set) |
| 20 | 4 | `init_len` (initialized data **plus** the explicit zero span of `.bss`) |
| 24 | 2 | `input_max` (u16). Chain admission is `SpaceCap::call_data_size` (L1: 5120). The call input is flattened from a one-layer list under the same cap |
| 26 | 2 | `output_max` (u16, at least 1). Chain admission is `SpaceCap::value_size` (L1: 1280) |
| 28 | 4 | `rodata_len` |

`entry` is an absolute guest PC: `CODE_BASE <= entry < CODE_BASE + code_len`,
on an instruction boundary, aligned to 4 bytes when bit 1 is clear and to
2 bytes when RVC is set. It is **not** a code-relative offset. Pack tools,
verifier, interpreter and AOT all use this convention.

Chain deploy admits the encoded container under `SpaceCap::calc_image_max`
(default 16383, the same number as HVM `function_size`), and the contract
code body keeps a `u16` length prefix, so the stored image cannot exceed
65535 bytes. A sidechain may raise `calc_image_max` up to 65535. A larger
program is split across sibling calc functions in the same contract; that
call is not part of this wire format. `code_len`, `rodata_len` and
`init_len` inside the container are already `u32`. Because `.bss` is folded
into `init`, zero-initialized static data counts against `calc_image_max`;
a kernel with a very large static buffer must use explicit memory instead.

The two reserved header bytes are the only future wire extension point and
must be zero for v1; a new field needs a new `version`.

## 2. ISA profiles

Header byte 5 is an orthogonal matrix. Bit 0 and bit 1 are chosen by the
deployer; bit 2 is the deployer's zk declaration. zk proofs use a container
with bit 1 clear (profiles 0/1 and 4/5); profiles 2/3 (RVC) are a
toolchain-density convenience, not the industry RISC-V zkVM baseline.

| Value | Constant | ISA |
|-------|----------|-----|
| 0 | `CALC_PROFILE_RV64IM` | RV64I + M + Zicclsm. No C. No bitmanip. zkVM baseline |
| 1 | `CALC_PROFILE_RV64IM_B` | profile 0 plus Zba/Zbb/Zbs/Zbc. No C |
| 2 | `CALC_PROFILE_RV64IMC` | profile 0 plus RVC |
| 3 | `CALC_PROFILE_RV64IMC_B` | profile 2 plus Zba/Zbb/Zbs/Zbc |
| +4 | `CALC_PROFILE_NO_HOST_ECALL` | orthogonal bit 2: no host ecalls, only HALT |

Profiles 0 and 1: `code_len % 4 == 0`; every instruction is 4 bytes.
Compressed encodings (`inst[1:0] != 0b11`) are rejected. Profiles 2 and 3:
`code_len % 2 == 0`; instructions are 2 or 4 bytes; jump targets must be
instruction boundaries (2-byte aligned is not enough). `ebreak`
(`0x00100073`), canonical `unimp`
(`0xC0001073`, `csrrw x0, cycle, x0`), and C.EBREAK (`0x9002`, RVC
profiles only) decode to `Inst::Trap`: static cost 1 ALU cycle, then the
interpreter traps and AOT jumps to the trap stub. The all-zero 16-bit
encoding (C.UNIMP / reserved `C.ADDI4SPN` imm=0) stays a decode reject.
Other CSR, A, F/D, and unknown opcodes trap-reject at verify.

Bit 2 (`CALC_PROFILE_NO_HOST_ECALL`) is a promise that the container only
uses HALT (`a7 = 0`); the engine traps every other ecall number. It is the
zk-ready declaration and is enforced at run time (the ecall number is
computed). It is currently redundant with the v1 host-ecall disable (§6);
it exists so a proven subset stays HALT-only if the table is enabled later.

### Profile 0 whitelist

RV64I: LUI, AUIPC, JAL, JALR, BEQ/BNE/BLT/BGE/BLTU/BGEU, LB/LH/LW/LD/LBU/LHU/LWU,
SB/SH/SW/SD, ADDI/SLTI/SLTIU/XORI/ORI/ANDI, SLLI/SRLI/SRAI, ADD/SUB/SLL/SLT/SLTU/XOR/SRL/SRA/OR/AND,
ADDIW/SLLIW/SRLIW/SRAIW, ADDW/SUBW/SLLW/SRLW/SRAW, FENCE (nop), ECALL,
EBREAK and canonical UNIMP (both `Inst::Trap`, 1 cycle, then trap).

RV64M: MUL/MULH/MULHSU/MULHU/DIV/DIVU/REM/REMU and W-forms.

### Bit 0 additions (profiles 1 and 3: Zba/Zbb/Zbs/Zbc)

`add.uw`, `sh1add`/`sh2add`/`sh3add` and `.uw` forms, `andn`/`orn`/`xnor`,
`rol`/`ror`/`rolw`/`rorw`/`rori`/`roriw`, `min`/`minu`/`max`/`maxu`,
`clz`/`ctz`/`cpop`/`clzw`/`ctzw`/`cpopw`, `sext.b`/`sext.h`/`zext.h`,
`orc.b`/`rev8`, `bset`/`bclr`/`binv`/`bext` and immediate forms,
`slli.uw`, `clmul`/`clmulh`/`clmulr`.

### Bit 1 additions (profiles 2 and 3: RVC)

The RV64 C whitelist from the pre-activation decoder: `c.addi4spn`,
`c.lw`/`c.ld`/`c.sw`/`c.sd`, `c.addi`/`c.nop`/`c.addiw`/`c.li`,
`c.addi16sp`/`c.lui`, `c.srli`/`c.srai`/`c.andi`, the `c.sub`/`c.xor`/
`c.or`/`c.and`/`c.subw`/`c.addw` group, `c.j`, `c.beqz`/`c.bnez`,
`c.slli`, `c.lwsp`/`c.ldsp`/`c.jr`/`c.mv`/`c.jalr`/`c.add`, `c.swsp`/
`c.sdsp`. `c.ebreak` decodes to `Inst::Trap`; the all-zero `c.unimp`
encoding (reserved `C.ADDI4SPN` imm=0) is rejected at decode. Quadrant 0
funct3 values outside that list are rejected. Jump and link addresses use
the encoded length (2 or 4), not a fixed 4.

## 3. Memory map

`CODE_BASE = 0x1000`. Addresses `[0, CODE_BASE)` are **unmapped**: any
load, store, jump or ecall buffer that touches them traps.

`mem_kib` sizes the usable region **above** `CODE_BASE`:

```
usable_len = mem_kib * 1024
mem_len    = CODE_BASE + usable_len     // exclusive end of guest space
```

Resource and setup charges use `mem_kib` / `usable_len`, not the unmapped
low page. The interpreter allocates exactly `mem_len` bytes.

`decode` accepts any `mem_kib` from 4 through `CALC_MEM_KIB_ENGINE_MAX`
(2_097_147 KiB, about 2 GiB) whose regions do not overlap. The engine
ceiling is the AOT `i32` address compare: `CODE_BASE + mem_kib * 1024`
must be `<= i32::MAX`. The admission cap is not a wire constant.
`verify_container_with_cap` rejects `mem_kib > cap.calc_mem_kib_max`.
`SpaceCap::calc_mem_kib_max` defaults to 64 (`CALC_MEM_KIB_MAX`).
A sidechain raises `SpaceCap::calc_mem_kib_max` and passes the same
`SpaceCap` to the packer. That does not change the header. Raising the L1
default is a consensus upgrade. Layout always follows the container's own
`mem_kib`.

Layout (absolute addresses, 16-byte aligned where noted):

```
[0, CODE_BASE)                              unmapped
[CODE_BASE, CODE_BASE+code_len)             code   RX
[rodata_base, rodata_base+rodata_len)       rodata R  (rodata_base = align16(CODE_BASE+code_len))
[rodata_end, data_base)                     hole   R  (empty; nothing is linked here)
[data_base, data_base+init_len)             init   RW (data_base = align16(rodata_end))
[data_end, data_end+CALC_STACK_GUARD)       guard  store-trap (256 bytes, guard_base = data_end)
[stack_bottom, stack_top)                   stack  RW, grows down; sp = stack_top
[output_base, output_base+output_max)       output RW
[input_base, input_base+input_max)          input  RW (input_base = align16_down(mem_len-input_max))
[input_base+input_max, mem_len)             tail   RW (alignment padding, unused)
stack_top = output_base
```

`output_base = align16_down(input_base - output_max)`. The stack is every
byte between the guard and the output region. `CALC_STACK_SIZE` (2 KiB) is
the minimum that must remain after the guard; it is not a fixed stack
window. Both `CALC_STACK_SIZE` and `CALC_STACK_GUARD` are consensus
constants that may only be **lowered** after activation: raising them
would reject previously legal containers and turn previously legal stores
into traps. The fit rule is single: `data_end + CALC_MIN_FREE_RW_GAP <=
stack_top`, where `CALC_MIN_FREE_RW_GAP = CALC_STACK_GUARD +
CALC_STACK_SIZE`.

`init` carries both `.data` and the explicit zero span of `.bss` (the
packer folds the ELF `.bss`/`.sbss` VMA into `init`). There is no separate
bss field, and the guard therefore always starts at the end of the
container's zero-initialized image. `pack_elf` rejects a `.bss` that does
not fit the layout; `min_mem_kib_for` returns the smallest `mem_kib` that
does.

Stores with `addr < data_base` trap (code, the optional alignment pad
before rodata, rodata, and the `[rodata_end, data_base)` hole). The hole
exists so `.data`/`.bss` VMAs match the 16-byte `data_base`; linker
scripts `ALIGN(16)` before `.data` and link nothing into the hole.
Stores intersecting the 256-byte stack guard trap. Loads may read the
guard. Linker script `.text` stays at `0x1000`.

## 4. Misaligned access (Zicclsm)

Loads and stores at **any alignment** execute correctly: little-endian,
byte-wise copy of the accessed width. Bounds and permission checks apply
to the **full** `[addr, addr+size)` range. There is **no extra cycle
charge** for misalignment: a per-access surcharge cannot be prepaid in the
basic-block model without peeking at the address. Cost equals the aligned
weight. AOT matches (x86-64 unaligned loads/stores).

Instruction fetch and jump targets must be instruction boundaries: 4-byte
aligned on profiles 0/1 and 2-byte aligned on profiles 2/3. `jalr` still
clears bit 0 of the target, and a target that is not an exact instruction
start traps.

## 5. Register state at entry

All 32 registers are 0 except:

| Reg | Value |
|-----|--------|
| `sp` (x2) | `stack_top` |
| `a0` (x10) | `input_base` |
| `a1` (x11) | input length in bytes |
| `a2` (x12) | `output_base` |
| `a3` (x13) | `output_max` |

`ra` is 0. `jalr` to `ra` (or any non-code address) is an invalid target
and traps. There is no return sentinel.

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
| 16..=31 | HINT / private input | | **trap** (L1 nodes cannot agree on hint content) |
| ≥ 32 | unknown | | trap |

**v1 executes HALT only.** Host ecalls (`a7` = 1..=6) are compiled but
disabled by `CALC_HASH_ECALLS_ENABLED = false`: every non-HALT ecall traps.
The digest implementations (`run_hash`), the pricing (`HashKind::cycles`)
and both engine paths stay in the tree behind that constant, so a later
coordinated consensus upgrade can enable the table without re-deriving it.
A container that sets the `CALC_PROFILE_NO_HOST_ECALL` bit keeps trapping
non-HALT ecalls even after that flip.

The table above is the contract for the enabled state. Hash ecalls charge
`64 + ceil(len/64) * per_block` **before** bounds checks (so a too-small
budget yields OutOfGas rather than Trap). Per-block: SHA-256=96,
SHA3/RIPEMD/Keccak=128, BLAKE2=64.

HALT with `a0 == 0` is success: the host copies `a1` bytes from the output
region (must be `<= output_max`). HALT with `a0 != 0` is a guest-reported
failure: classified as **Trap** (`CalcAbort2::Trap`), receipt
`exit_code = a0`, empty output / empty output hash. Cycles consumed so far
are still charged.

Write the output **before** HALT. The hash builtins execute host-native
code outside the RISC-V instruction trace. They are deterministic on L1,
but a zk proof of a kernel that calls one requires a matching
precompile/circuit; that is why v1 keeps them off and why the profile byte
carries the HALT-only bit.

## 7. Traps

Deterministic Trap-class aborts (cycles consumed up to the failing
instruction's block charge, excluding a charge that does not fit):

- unmapped address (`< CODE_BASE`)
- load/store/ecall out of `mem_len` or address overflow
- store / digest into code or rodata
- store / digest into the stack guard
- `jal`/`jalr`/`branch` target not an instruction boundary inside code
- `ebreak` / canonical `unimp` (`Inst::Trap`)
- unknown / disabled host ecall / HINT ecall
- output length > `output_max` on HALT
- guest HALT with nonzero exit code
- input longer than `input_max` (pre-flight, 0 cycles)

OutOfGas: remaining budget cannot cover the next prepaid block, the hash
ecall cost, or the setup charge. The setup shortfall reports
`cycles_used = setup` (only the attempted charge; the rest of the budget is
not burned) and is a terminal failure — the abort class and the tx outcome
are identical either way because a transaction that reaches it cannot be
included. Deadline is host-only (non-consensus).

**OutOfResource is pre-flight only (0 cycles).** The resource charge of a
call is fully determined before it runs: `ceil(input_len/16)` +
`ceil(output_max/16)` + `8 per declared KiB`. `output_max` — the
container's declared maximum, not the bytes the kernel happens to return —
is reserved by the pre-flight, so a successful call can never exceed the
resource budget after the run. There is no post-run resource check. The
returned `resource` field still charges the *actual* output length.

`output_max` needs no run-time check: deploy and load admit it against
`SpaceCap::value_size`, the caps only rise, and the caller validates the
returned value against the same cap.

## 8. Gas

`CYCLES_PER_COMPUTE = 56` (policy composition `32 + 16 + 8`: old base
rate + per-KiB setup cycles + per-KiB resource gas; these are different
dimensions, so the sum is a chosen policy value, not a unit derivation).
On the 2026-11-04 warm same-machine calibration (HVM ~33 ns/compute, AOT
~0.38 ns/cycle) AOT parity is N ≈ 87, so N=56 is roughly +55% over AOT
hardware cost; the older cold-state calibration (50.7/6.0/0.62) gives
~+46%, i.e. "about +50%". Against the interpreter/lockstep it remains a
large discount. The 200,863-cycle HIP5 kernel costs 3,587 compute at
N=56 (6,277 at N=32); with resource 413 the total is 4,000. N is
consensus-frozen: any later change is a hard fork. Setup (always kept,
even on trap):

```
setup = CALC_SETUP_BASE_CYCLES + mem_kib * CALC_SETUP_PER_KIB_CYCLES
      = 64 + mem_kib * 16
```

Resource: `ceil(input_len/16) + ceil(output_len/16) + mem_kib * 8`.
Aborted runs charge 0 for output length.

The resource pre-flight reserves the declared `output_max` on the same
formula (see §7); this is what makes the charge known before execution.

### Static weights

One table (`decode::WEIGHTS`) serves every profile: repricing a profile
after activation would change the gas of already-deployed containers, so a
future price change may only attach a new table to a NEW profile value,
never reprice 0..=7. (The mechanism is not built yet; adding it is a
widening.)

| Class | Cycles |
|-------|--------|
| ALU, branches, jumps, FENCE, unary/bitmanip except clmul | 1 |
| load / store | 2 |
| MUL family | 3 |
| DIV/REM family | 12 |
| CLMUL family | 24 |
| ECALL static (`ecall_base`) | 1 (hash cost added at runtime when enabled) |

### Basic-block prepaid charging

The verifier splits code at every JAL/JALR/branch/ECALL/Trap (terminator)
and at the instruction after a terminator. Entering a block charges
`suffix_cycles[idx]`: the sum of static weights from the **entry
instruction** to the block end. A `jalr` into the middle of a block
therefore does **not** pay for skipped prefix instructions.

On trap or OOG: if the remaining budget cannot cover the next suffix
charge, abort OutOfGas **without** taking that charge. A trap after a
successful charge keeps the charge. Hash ecall extra cost is charged
inside the ecall, after the static suffix has already been paid.

## 9. Image ID and receipt

```
image_id = SHA-256( b"hacash-calc-image-v1" || encode(container) )
```

`CalcContainer::image_id()` and `CalcFnObj::image_id()` return this. The
object hashes it on first use (`OnceLock`), never per call.

The call receipt is a **derived artifact**, not part of consensus
execution. `CalcFnObj::run` returns `CalcExec { result, cycles_used,
resource, exit_code }` only — it does **not** SHA-256 the input or
output (that would be uncharged host work on the hot path). Callers
that need a proof statement invoke `CalcFnObj::receipt(&self, input,
exec)`:

```
image_id, input_hash = SHA-256(input),
output_hash = SHA-256(output) on success else SHA-256([]),
exit_code, abort (None on success), cycles_used
```

Identical between interpreter and AOT when derived from equal `CalcExec`.

## v1 semantics (unconditional)

Official x86-64 nodes build with `calc-aot` (the vm crate default). Each
call runs the interpreter and then AOT in order on the calling thread. The
call is kept only when `cycles_used`, the output bytes, the abort class and
the halt code match. A mismatch is `CalcEngineMismatch` (fault 156): the
transaction is rejected and does not enter a block, and no agreed gas is
settled for that call; a panic inside either engine is caught and reported
the same way. Builds without the feature, or with the image unmounted, run
the interpreter only. This lockstep pair is the early launch check. After
it has stayed quiet, the interpreter is retired and AOT is the only engine.

The lockstep compares the **consensus-observable** surface (output bytes,
abort class, exit code, cycles). Registers and memory are compared by the
differential test suites, not on the hot path.

A host wall-clock deadline is observed by the interpreter only. The AOT
image **still runs** (a deadline never skips the lockstep, so preflight and
consensus exercise the same engines); a deadline stop is host policy, not
guest semantics, so the AOT verdict is not compared and the call reports
`Deadline`.

Every execution capability is unconditional — the only chain parameter is
`SpaceCap::calc_mem_kib_max`, and no legal future value can turn a
capability off, because that would reject or reinterpret a previously legal
container:

- All eight profile cells of header byte 5 are admitted; the deployer picks
  the cell. zk images leave bit 1 clear and set bit 2; the chain does not
  forbid either choice.
- HALT (`a7` = 0) always works. Host hash ecalls are disabled in v1 (§6).
- Code, the pad below `rodata_base`, rodata and the `[rodata_end,
  data_base)` hole are read-only; stores and digest writes there trap.
- Stores and digest writes that intersect the 256-byte stack guard band
  trap.
- Any instruction-boundary entry inside the code region is accepted.

There is no consensus config struct and no height-dependent execution
config. Admission grows by raising the current `SpaceCap`; a deployed
kernel loads under every later cap because caps only rise. The layout
constants (`CODE_BASE`, the 16-byte alignment, `CALC_STACK_GUARD`,
`CALC_STACK_SIZE`, `CALC_MEM_KIB_MIN`) are frozen: after activation they may
only be relaxed (guard/stack/min-memory lowered), never tightened. `abi` and
`bss` are not wire fields in v1; `.bss` is part of `init`.

## 10. Building a kernel

There is no tier-2 `riscv64im` rustc target. `riscv64imac-unknown-none-elf`
plus `-C target-feature=-c` still links **prebuilt** `compiler_builtins`
that contain C, so the image is not 4-byte-only. Fixtures therefore use a
custom target JSON (`riscv64im_zicclsm.json`: arch `riscv64`, features
`+m,-a,-c,-d,-f`, `llvm-abiname: lp64`, `os: none`, `panic-strategy:
abort`) and rebuild libcore:

```
RUSTC_BOOTSTRAP=1 cargo build -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem --release \
  --target riscv64im_zicclsm.json
RUSTFLAGS="-C relocation-model=static -C link-arg=-Tcontainer.ld"
```

(`zicclsm` is a memory-model convention — misaligned loads/stores — not an
LLVM feature flag.) Linker script starts at `CODE_BASE` (`. = 0x1000`).
Entry is `calc_entry`; `llvm-nm` of that symbol is the absolute `entry`
written into the header. The kernel **must HALT via ecall** (`a7=0`); a
compiled `ret` jumps to `ra=0` and traps.

Pack tools use `pack_elf_min_mem`: the smallest `mem_kib` that fits
`code + rodata + init (.data + .bss) + guard + CALC_STACK_SIZE + the
declared IO regions`. Declaring `CALC_MEM_KIB_MAX` instead would charge
every call for memory the kernel never touches.

Profile 1: set JSON `features` to `+m,+zba,+zbb,+zbs,+zbc,-a,-c,-d,-f`.

Stable Rust path (profiles 2/3): the tier-2
`riscv64imac-unknown-none-elf` target with
`RUSTFLAGS="-C relocation-model=static -C link-arg=-Tcontainer.ld -C target-feature=-a"`
builds RV64IMC code without `-Zbuild-std` or `RUSTC_BOOTSTRAP`; the
`calckernel` fixture builds and contains no A-extension instructions. This
path is only usable when the container selects profile 2/3 (C is part of
the image). Two caveats: prebuilt `core`/`compiler_builtins` are compiled
for a +A target, so a kernel that pulls in an atomic helper can still fail
verification (keep atomics out; LTO/`--gc-sections` drops unused helpers),
and profiles 2/3 accept `c.ebreak` but still reject `c.unimp` (§2). A kernel
path that LLVM lowers to the all-zero `c.unimp` therefore needs the profile
0/1 custom-target build or a workaround (a 32-bit trap encoding; LTO/
`--gc-sections` for dead paths). Profiles 0/1 (the zkVM-compatible,
4-byte-only subset) still require the custom target JSON and the nightly
`-Zbuild-std` recipe above.

## 11. RISC-V architecture tests

See `vm/tests/archtest/README.md`. The harness is `vm/tests/calc_archtest.rs`
(`#[ignore]` until `CALC_ARCHTEST_DIR` is set). Do not vendor the suite.
