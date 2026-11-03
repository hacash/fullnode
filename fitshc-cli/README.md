# fitshc — FitSH compiler CLI with rv64im calc kernels

`fitshc` compiles `.fitsh` contracts and, for `function rv64im` declarations,
builds their rust kernels through the real rust cross-toolchain, packs them
into `HCF1` calc containers, verifies them with the exact chain checks, and
injects them into the contract.

```bash
cargo build --release -p fitshc-cli        # binary: target/release/fitshc
```

Toolchain prerequisites (one time):

```bash
rustup target add riscv64imac-unknown-none-elf   # rv64imc / rv64imcb recipes
rustup component add rust-src                    # rv64im / rv64imb (+ --strict) recipes
```

The `rv64im`/`rv64imb` recipes rebuild `core`/`compiler_builtins`
(`-Zbuild-std`), which needs `RUSTC_BOOTSTRAP=1`; the CLI sets it for the
cargo invocation, so plain **stable** works (a nightly toolchain also works).

## One-line contract kernels

`contract.fitsh`:

```
pragma fitsh 1.0.0
contract Demo {
    function rv64im hash(data: bytes) -> bytes
    function external fingerprint(x: bytes) -> bytes { return self::hash(x) }
}
```

`hash.rs` (same directory):

```rust
// calc: profile = rv64imc
// calc: input_max = 1024
// calc: output_max = 64

pub fn hash(input: &[u8], out: &mut [u8]) -> usize {
    // pure computation; write into `out`, return the output length
}
```

```bash
fitshc build contract.fitsh
# Compile success!
# Calc kernel 'hash': ./hash.hcf1 (profile 0x06, mem 4 KiB, in 1024, out 64, ...)
# Generated: contract.contractmap.json
# Generated: contract.deploy.json
```

The rust file must define a function named after the fitsh declaration.
With call data (`hash(data: bytes)`):

```rust
pub fn hash(input: &[u8], out: &mut [u8]) -> usize
```

Zero-arg form (`tick()`):

```rust
pub fn tick(out: &mut [u8]) -> usize
```

The file is a plain **module**, not a crate root: no `#![no_std]`, no
`#[panic_handler]`, no linker scripts — the generated wrapper owns the whole
ABI (`a0..a3` slices, HALT, traps, panic handler). Wrong signatures are
ordinary rustc errors pointing at your file.

## `// calc:` metadata

Only the leading comment block is parsed (blank lines may interleave);
unknown / duplicate / malformed keys are errors with file and line.

| key             | default   | values                            | notes                          |
|-----------------|-----------|-----------------------------------|--------------------------------|
| `profile`       | `rv64imc` | `rv64im` `rv64imb` `rv64imc` `rv64imcb` | container profile bits    |
| `input_max`     | `1024`    | `0..=65535`, ≤ chain `call_data_size` | input region size         |
| `output_max`    | `1024`    | `1..=65535`, ≤ chain `value_size` | output region size            |
| `mem_kib`       | `auto`    | `auto` or `4..=calc_mem_kib_max`  | `auto` = minimum fitting fit |
| `no_host_ecall` | `true`    | `true` / `false`                  | HALT-only declaration bit     |

Metadata steers the build only; the container header is the runtime truth.
The fitsh keyword stays `rv64im` (the engine family) — the profile is a
property of the compiled kernel, and the same contract source can be
recompiled to a different profile without touching the fitsh file.

## Kernel resolution order

For `function rv64im <name>` without `= 0x...` (relative to the `.fitsh`
directory):

1. `--calc <name>=<path>` — explicit `.rs` / `.hcf1` / `.elf` override
2. `<name>.rs` — rust kernel, auto-compiled (source of truth)
3. `calc/<name>.rs` — directory-grouped variant
4. `<name>.hcf1` — pre-packed container (decoded + verified)
5. `<name>.elf` — pre-compiled ELF (packed with `--in/--out/--profile/--mem`)

`--no-build` skips steps 2–3 (CI / rustc-less environments).

## Subcommands

```bash
# compile contract (+ auto kernels) and emit contractmap/deploy json
fitshc build contract.fitsh [fee] [nonce] [--periods N]

# legacy positional form — identical to `build`
fitshc contract.fitsh 1:248 1

# pack an already-linked ELF into a verified container
fitshc pack kernel.elf --profile rv64imc --in 1024 --out 1024 --mem auto -o kernel.hcf1

# build (if .rs) and execute a kernel locally — the fastest feedback loop
fitshc run hash.rs --input 0x686163617368
```

Flags: `--calc name=path` (repeatable), `--no-build`, `--strict`
(rebuild core for C profiles — avoids the prebuilt `+A` core pulling in
atomics), `--in/--out/--profile/--mem` (bare `.elf` packing), `--cap l1`,
`--verbose`, `-o`.

## Caching & reproducibility

Builds are content-addressed under `$FITSHC_CACHE` (default
`~/.cache/fitshc`), keyed by kernel source + metadata + recipe + wrapper
templates + rustc/cargo versions + the calc `SpaceCap` fields. A hit skips
cargo entirely and the stored container is re-verified before reuse. Output
paths are remapped (`--remap-path-prefix …=/calc/src`), `debuginfo=0`,
`opt-level=z`, `lto`, `codegen-units=1`, `panic=abort`; the same source,
toolchain and flags yield identical image IDs.

Each build records `<name>.calc.manifest.json` next to the `.fitsh`:
source/ELF hashes, recipe, rustc version, effective `mem_kib`, region sizes,
image ID and the `SpaceCap` snapshot.

## Layering

- `vm::fitshc` (library) — tokenizer/parser, `collect_calc_requests`,
  `compile_with_rv64im`: pure, no file system, no processes, usable from the
  wasm/SDK builds.
- this crate — everything that touches the file system or spawns cargo/rustc.
- nodes/consensus — never compile kernels; they only decode/verify the
  already-deployed container bytes.

Design notes and the resolution-order rationale: `vm/doc/fitshc-calc.loc.md`.
