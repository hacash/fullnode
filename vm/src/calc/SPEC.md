# Hacash calc VM specification

Normative semantics for the HVM calc-func RISC-V micro-VM. Interpreter and
x86-64 AOT (`calc-aot`) **must be bit-identical** on outputs, register file,
memory, cycle totals, abort class, image ID and receipt. This channel is not
yet activated on mainnet; the wire format here is v1.

Target compatibility: **profile 0** matches Ethereum zkVM
`riscv64im_zicclsm-unknown-none-elf` (eth-act/zkvm-standards): RV64I + M +
Zicclsm, little-endian, flat memory, **no C, no F/D, no A**.

## 1. Container wire format

36-byte big-endian header, then `code || rodata || init`. Magic `HCF1`.
There is no checksum; consensus commits the bytes.

| Offset | Size | Field |
|--------|------|--------|
| 0 | 4 | magic `HCF1` |
| 4 | 1 | version = 1 |
| 5 | 1 | `profile` bitfield. Bit 0 = Zba/Zbb/Zbs/Zbc. Bit 1 = RVC. Other bits rejected. Values 0..=3 |
| 6 | 1 | `abi` = 1 (bytes → bytes) |
| 7 | 1 | reserved, must be 0 |
| 8 | 4 | `mem_kib` (u32, `decode` requires 4..=2097147; verify cap is config) |
| 12 | 4 | `entry` — **absolute guest PC** |
| 16 | 4 | `code_len` (> 0; multiple of 4 when bit 1 is clear, multiple of 2 when RVC is set) |
| 20 | 4 | `init_len` |
| 24 | 2 | `input_max` (u16). Chain admission is `SpaceCap::call_data_size` (L1: 5120). The call input is flattened from a one-layer list under the same cap |
| 26 | 2 | `output_max` (u16, at least 1). Chain admission is `SpaceCap::value_size` (L1: 1280) |
| 28 | 4 | `rodata_len` |
| 32 | 4 | `bss_len` (u32). Zero means no `.bss`. No policy cap: the fit check must still leave the 256-byte guard and the 2048-byte minimum stack |

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
`init_len` inside the container are already `u32`.

## 2. ISA profiles

Header byte 5 is an orthogonal matrix. Bit 0 and bit 1 are chosen by the
deployer. zk proofs use a container with bit 1 clear (profiles 0 and 1).
Profiles 2/3 (RVC) are a toolchain-density convenience, not the industry
RISC-V zkVM baseline; a deployment that promises third-party proofs should
stay on profiles 0/1 (see also the hash-ecall caveat in §6).

| Value | Constant | ISA |
|-------|----------|-----|
| 0 | `CALC_PROFILE_RV64IM` | RV64I + M + Zicclsm. No C. No bitmanip. zkVM baseline |
| 1 | `CALC_PROFILE_RV64IM_B` | profile 0 plus Zba/Zbb/Zbs/Zbc. No C |
| 2 | `CALC_PROFILE_RV64IMC` | profile 0 plus RVC |
| 3 | `CALC_PROFILE_RV64IMC_B` | profile 2 plus Zba/Zbb/Zbs/Zbc |

Profiles 0 and 1: `code_len % 4 == 0`; every instruction is 4 bytes.
Compressed encodings (`inst[1:0] != 0b11`) are rejected. Profiles 2 and 3:
`code_len % 2 == 0`; instructions are 2 or 4 bytes; jump targets must be
instruction boundaries (2-byte aligned is not enough). `ebreak`
(`0x00100073`) and canonical `unimp`
(`0xC0001073`, `csrrw x0, cycle, x0`) decode to `Inst::Trap`: static cost
1 ALU cycle, then the interpreter traps and AOT jumps to the trap stub.
Other CSR, A, F/D, and unknown opcodes trap-reject at verify.

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
`c.sdsp`. `c.ebreak` is rejected at decode. Quadrant 0 funct3 values
outside that list are rejected. Jump and link addresses use the encoded
length (2 or 4), not a fixed 4.

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
`verify_container` rejects `mem_kib > cfg.mem_kib_max`. Consensus config
v1 and `SpaceCap::calc_mem_kib_max` both default to 64 (`CALC_MEM_KIB_MAX`).
A sidechain raises `SpaceCap::calc_mem_kib_max` (and passes the same
number to `CalcConfig::with_mem_kib_max` when packing). That does not
change the header. Raising the L1 default is a consensus upgrade.
`CalcConfig::testkit` may set a cap up to 16384 KiB (16 MiB) and is
absent from production builds. `CalcConfig::with_mem_kib_max` clamps at
the engine ceiling and is what a sidechain packer calls. Layout always
follows the container's own `mem_kib`.

Layout (absolute addresses, 16-byte aligned where noted):

```
[0, CODE_BASE)                              unmapped
[CODE_BASE, CODE_BASE+code_len)             code   RX
[rodata_base, rodata_base+rodata_len)       rodata R  (rodata_base = align16(CODE_BASE+code_len))
[rodata_end, data_base)                     hole   R  (empty; nothing is linked here)
[data_base, data_base+init_len)             init   RW (data_base = align16(rodata_end))
[data_end, data_end+bss_len)                bss    RW (bss_len = 0 on older containers)
[guard_base, stack_bottom)                  guard  store-trap (256 bytes, guard_base = bss end)
[stack_bottom, stack_top)                   stack  RW, grows down; sp = stack_top
[output_base, output_base+output_max)       output RW
[input_base, input_base+input_max)          input  RW (input_base = align16_down(mem_len-input_max))
[input_base+input_max, mem_len)             tail   RW (alignment padding, unused)
stack_top = output_base
```

`output_base = align16_down(input_base - output_max)`. The stack is every
byte between the guard and the output region. `CALC_STACK_SIZE` (2 KiB) is
the minimum that must remain after `.bss` and the guard; it is not a fixed
stack window. `bss_len` is a `u32` with no separate policy cap. `pack_elf`
rejects a `.bss` that does not leave the guard and the minimum stack, or
that does not fit in `u32`. The alignment padding after the defined input/output
regions is ordinary writable memory; it is deliberately not part of the
ABI (no pointer into it is exposed at entry).

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

HALT with `a0 == 0` is success: the host copies `a1` bytes from the output
region (must be `<= output_max`). HALT with `a0 != 0` is a guest-reported
failure: classified as **Trap** (`CalcAbort2::Trap`), receipt
`exit_code = a0`, empty output / empty output hash. Cycles consumed so far
are still charged.

Write the output **before** HALT. Hash ecalls charge
`64 + ceil(len/64) * per_block` **before** bounds checks (so a too-small
budget yields OutOfGas rather than Trap). Per-block: SHA-256=96,
SHA3/RIPEMD/Keccak=128, BLAKE2=64.

The hash builtins execute host-native code outside the RISC-V instruction
trace. They are deterministic on L1, but a zk proof of a kernel that calls
one requires a matching precompile/circuit (or a provable profile that
forbids the ecall). "zk-ready ISA" does not by itself make the ecall set
provable.

## 7. Traps

Deterministic Trap-class aborts (cycles consumed up to the failing
instruction's block charge, excluding a charge that does not fit):

- unmapped address (`< CODE_BASE`)
- load/store/ecall out of `mem_len` or address overflow
- store / digest into code or rodata
- store / digest into the stack guard
- `jal`/`jalr`/`branch` target not an instruction boundary inside code
- `ebreak` / canonical `unimp` (`Inst::Trap`)
- unknown / HINT ecall
- output length > `output_max` on HALT
- guest HALT with nonzero exit code
- input longer than `input_max` (pre-flight, 0 cycles)

OutOfGas: remaining budget cannot cover the next prepaid block, the hash
ecall cost, or the setup charge. The setup shortfall reports
`cycles_used = setup` (only the attempted charge; the rest of the budget is
not burned) and is a terminal failure — the abort class and the tx outcome
are identical either way because a transaction that reaches it cannot be
included. Deadline is host-only (non-consensus). OutOfResource /
ValueTooLarge are pre-flight host-limit errors (0 cycles).

## 8. Gas

`CYCLES_PER_COMPUTE = 32` (unchanged). Setup (always kept, even on trap):

```
setup = CALC_SETUP_BASE_CYCLES + mem_kib * CALC_SETUP_PER_KIB_CYCLES
      = 64 + mem_kib * 16
```

Resource: `ceil(input_len/16) + ceil(output_len/16) + mem_kib * 8`.
Aborted runs charge 0 for output length.

### Per-profile static weights

Tables are versioned per profile; values are currently identical.

| Class | Cycles |
|-------|--------|
| ALU, branches, jumps, FENCE, unary/bitmanip except clmul | 1 |
| load / store | 2 |
| MUL family | 3 |
| DIV/REM family | 12 |
| CLMUL family | 24 |
| ECALL static (`ecall_base`) | 1 (hash cost added at runtime) |

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

`CalcContainer::image_id()` and `CalcFnObj::image_id()` return this.
`image_id` is computed once at object build (deploy/load), not per call.

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

## v1 enabled set

Official x86-64 nodes build with `calc-aot` (the vm crate default). Each
call runs the interpreter and AOT on two threads. The call is kept only
when `cycles_used`, the output bytes, the abort class and the halt code
match. A mismatch is `CalcEngineMismatch` (fault 156): the transaction is
rejected and does not enter a block. No agreed gas is settled for that
call. Builds without the feature, and calls with a host deadline, run the
interpreter only. This lockstep pair is the early launch check. After it
has stayed quiet, the interpreter is retired and AOT is the only engine.

Consensus default is `CALC_CONFIG_V1`. Both profile axes are **on**: the
deployer selects the matrix cell in header byte 5. Hash
builtins, read-only rodata, the stack guard and a free entry point are
**on**. `compute_image_id_on_load` and `aot_mount` stay off in the
consensus constant; they are host knobs on `CalcHostConfig` (applied by
`CalcFnObj::build_with_host` / `build_with_aot`).
Deploy and load both use `CALC_CONFIG_V1` plus the current `SpaceCap`
(`calc_mem_kib_max` default 64, `call_data_size` 5120, `value_size` 1280,
`calc_image_max` 16383). Those caps only rise, so a kernel that passed
admission still loads. Execution flags do not vary by height. A future
relaxation that must keep old-height replay unchanged adds that fork
when it is actually required. AOT stays a host knob
on `CalcHostConfig`, applied at load via `Runtime::set_calc_host_config`
and never a consensus path.
`CALC_CONFIG_V1` is the execution snapshot for both deploy and load.
`CalcConfig::all_enabled()` is test-only (`mem_kib_max` stays 64; it is
not a large-memory preset and not a consensus upgrade). A future
consensus upgrade must add a named constant such as `CALC_CONFIG_V2`
rather than calling `all_enabled()`. Admission grows by raising the
current `SpaceCap`. Turning off an enabled v1 execution flag is not a
permitted upgrade. Layout and gas constants, including
`CYCLES_PER_COMPUTE`, do not change. `CalcConfig::v1()` is
`CALC_CONFIG_V1`.

| Flag | V1 |
|------|----|
| `profile1_bitmanip` | on — profile bit 0 is admitted; turning the flag off rejects any container with that bit |
| `compressed` | on — profile bit 1 (RVC) is admitted; turning the flag off rejects any container with that bit. zk images omit the bit |
| `hash_builtins` | on — ecall `a7` 1..=6 runs the hash builtins; turning the flag off traps "builtin not enabled" with no hash charge; HALT (`a7` 0) always works |
| `rodata_readonly` | on — stores and digest writes into rodata and `[rodata_end, data_base)` trap; code and the pad below `rodata_base` stay non-writable either way |
| `stack_guard` | on — stores that intersect the 256-byte guard band trap |
| `compute_image_id_on_load` | off — `image_id()` / `receipt()` hash the container on demand; `CalcHostConfig` can turn it on |
| `aot_mount` | off — `CalcFnObj::build` stays on the interpreter; `build_with_aot` / `CalcHostConfig::with_aot_mount(true)` mounts AOT when `calc-aot` is compiled |
| `free_entry` | on — any instruction-boundary entry inside the code region is accepted; turning the flag off requires `entry == CODE_BASE` |
| `mem_kib_max` | 64 — offline `verify_container` / `pack_container` reject `mem_kib` above this. Chain deploy and load use `SpaceCap::calc_mem_kib_max` (same default). Wire decode accepts up to 2097147 |

Sections above describe the semantics of each capability when that flag is on.

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
and profiles 2/3 currently reject the `c.unimp`/`c.ebreak` trap encodings
(§2), so a kernel path that LLVM lowers to `llvm.trap` needs the profile
0/1 custom-target build or a future policy decision. Profiles 0/1 (the
zkVM-compatible, 4-byte-only subset) still require the custom target JSON
and the nightly `-Zbuild-std` recipe above.

## 11. RISC-V architecture tests

See `vm/tests/archtest/README.md`. The harness is `vm/tests/calc_archtest.rs`
(`#[ignore]` until `CALC_ARCHTEST_DIR` is set). Do not vendor the suite.
