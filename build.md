# Hacash fullnodenext build and run guide

This document replaces the `fullnodedev/build.md` instructions. The old project
was a root package; `fullnodenext` has a virtual Cargo workspace and the node
executable is the `fullnode` binary of package `app`. Build commands therefore
need `-p app --bin fullnode`.

## 1. Toolchain and native dependencies

The workspace uses Rust edition 2024. Use a current stable Rust toolchain:

```sh
rustup update stable
rustup default stable
rustc --version
```

Ubuntu dependencies for the default sled backend:

```sh
sudo apt update
sudo apt install build-essential pkg-config
```

`db-leveldb-sys` additionally needs a C/C++ toolchain and CMake.
`db-rocksdb` additionally needs CMake, Clang and libclang:

```sh
sudo apt install build-essential cmake clang libclang-dev pkg-config
```

## 2. Build the full node

The default backend is sled:

```sh
cargo build --release -p app --bin fullnode
```

The resulting executable is:

```text
target/release/fullnode
```

For a development build:

```sh
cargo build -p app --bin fullnode
```

The other executables remain independent Cargo binary targets:

```sh
cargo build --release -p app --bin poworker
cargo build --release -p app --bin diaworker
cargo build --release -p app --bin fitshc
```

Enable `ocl` when building a worker that should use OpenCL:

```sh
cargo build --release -p app --bin poworker --features ocl
```

## 3. Build the WASM/JavaScript SDK

Install the WASM target and the matching wasm-bindgen CLI once:

```sh
rustup target add wasm32-unknown-unknown
cargo install -f wasm-bindgen-cli --version 0.2.100
```

Build all Node.js, web ESM and inline-page artifacts:

```sh
./sdk/pack.sh
```

Outputs are written to `sdk/dist/{nodejs,web,page,js}`. To build only one raw
wasm-bindgen target, run `./sdk/build.sh nodejs`, `web`, or `no-modules`.
`wasm-opt` is used automatically when installed.

## 4. Select a database backend

`app` forwards one database feature to `db`. Select exactly one backend. When
selecting a backend other than the default, `--no-default-features` is required:

```sh
# sled (default, pure Rust)
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-sled

# rusty-leveldb (pure Rust LevelDB implementation)
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-rusty-leveldb

# leveldb-sys (native C++ LevelDB)
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-leveldb-sys

# RocksDB (native C++)
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-rocksdb
```

If several DB features are enabled accidentally, runtime priority is
`rocksdb > leveldb-sys > rusty-leveldb > sled`; the unused backends are still
compiled. A build with no DB feature compiles, but opening a disk-backed node
fails with `no db backend feature enabled`.

The backend is a compile-time choice, not an INI setting. Do not open an
existing data directory with a different backend. In particular, do not point
fnext at fdev's production data directory; use a new directory and let fnext
synchronize or import data through an explicitly verified migration process.

Database durability can be adjusted at process startup:

```sh
HACASH_DB_SYNC=1 ./target/release/fullnode /absolute/path/hacash.config.ini
```

`HACASH_DB_SYNC=1` requests synchronous writes. With sled,
`HACASH_DB_SMALL_MACHINE=1` also selects its low-space mode and a 64 MiB cache.
Accepted true values are `1`, `true`, `yes`, and `on` (case-insensitive).

## 5. Linux static build

The pure-Rust DB backends are the simplest choices for a musl build:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release -p app --bin fullnode \
  --target x86_64-unknown-linux-musl \
  --no-default-features --features db-sled
```

Output:

```text
target/x86_64-unknown-linux-musl/release/fullnode
```

Native C++ backends need a matching cross C/C++ toolchain and are not covered
by the command above.

## 6. Windows and macOS

Build on Windows with MSVC (PowerShell):

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --release -p app --bin fullnode --target x86_64-pc-windows-msvc
```

Output: `target/x86_64-pc-windows-msvc/release/fullnode.exe`.

Build natively on an Intel macOS host:

```sh
rustup target add x86_64-apple-darwin
cargo build --release -p app --bin fullnode --target x86_64-apple-darwin
```

Cross-compiling native C++ DB backends still requires the target platform's
compiler, linker and SDK. The old fdev instructions that manually changed the
`leveldb-sys` version are obsolete: fnext already pins `leveldb-sys = 2.0.9`.

## 7. Configuration file

The repository contains a conservative mainnet example at
`hacash.config.ini`. The first positional argument is the config path:

```sh
./target/release/fullnode "$(pwd)/hacash.config.ini"
```

With no argument, the node loads `hacash.config.ini` beside the executable, not
from the shell's working directory. A relative argument is also resolved from
the executable directory. Pass an absolute path as above, or place the config
next to the executable.

INI rules:

- `#` and `;` start comments outside quoted values.
- Boolean values are `true`/`false` or `1`/`0`.
- Lists are comma-separated, for example `a:3337,b:3337`.
- `key =` means "not configured" and lets the Rust default apply. Use `key = ""`
  only when an explicit empty string is required.
- Unknown keys inside known sections are rejected. Unknown top-level sections
  are allowed so an external indexer can own its own section.

### `[engine]`

| Key | Default | Meaning |
| --- | ---: | --- |
| `data_dir` | empty | Empty uses in-memory storage; set this for a persistent node. Relative paths use the process working directory. |
| `fast_sync` | `false` | Enables the reduced-check synchronization path. Keep false unless the operational trust model explicitly permits it. |
| `unstable_block` | `4` | Fork/reorganization window. |
| `recent_blocks` | `true` | Maintain recent-block indexes. |
| `average_fee_purity` | `true` | Maintain rolling fee-purity samples. |
| `show_miner_name` | `false` | Print miner details in block logs. |
| `vm_log_enable` | `false` | Persist VM logs. |
| `vm_log_open_height` | `0` | First height at which VM logs are persisted. |

Persistent storage is split below `data_dir` into `block/`, `state_v4/`, and
`vmlog/`. When `DB_VERSION` changes, fnext renames an old state/log directory
to a timestamped backup and rebuilds state from local block history.

`HACASH_DATA_DIR` overrides `[engine].data_dir` for the runtime storage path.

### `[p2p]`

| Key | Default | Meaning |
| --- | ---: | --- |
| `listen_ip` | `0.0.0.0` | IP address used for inbound P2P binding. |
| `listen_port` | `3337` | Inbound and advertised P2P port; `0` disables inbound listening. |
| `boot_nodes` | empty | Comma-separated bootstrap `host:port` addresses. |
| `node_name` | generated | Human-readable peer name; an empty value generates `hn` plus 8 key hex digits. |
| `block_queue_cap` | `8` | Inbound block queue capacity. |
| `dial_interval_secs` | `60` | Bootstrap/addrbook redial interval. |
| `max_peers` | `204` | Total peer limit. |
| `find_nodes` | `true` | Enable peer discovery. |
| `accept_nodes` | `true` | Accept inbound peers. |
| `use_stable_nodes` | `true` | Read and reuse `stable.nodes`. |
| `backbone_peers` | `4` | Desired public/backbone peers. |
| `offshoot_peers` | `200` | Non-backbone peer limit. |
| `addrbook_max` | `200` | In-memory address-book limit. |
| `stable_max_write` | `200` | Maximum persisted stable addresses. |
| `addrbook_dial_max` | `16` | Maximum addresses dialed per discovery pass. |

The node creates `node.id` and `stable.nodes` under the data directory.

### `[server]`, `[txpool]`, and `[vm_api]`

| Section/key | Default | Meaning |
| --- | ---: | --- |
| `server.enable` | `false` | Enable the HTTP API listener. When `false`, no HTTP socket is bound. |
| `server.listen_ip` | `127.0.0.1` | HTTP bind IP. Set `0.0.0.0` only when the API should be reachable remotely. |
| `server.listen_port` | `8082` | HTTP port; `0` disables the HTTP server. |
| `server.debug_routes` | `false` | Register routes marked debug. Do not expose them on a public listener. |
| `txpool.maxs` | empty | Comma-separated capacities for consensus-defined transaction groups. Missing entries keep group defaults. |
| `txpool.min_fee_purity` | `6024` | Local mempool minimum fee purity (`1000000 / 166`, integer division). |
| `vm_api.log_delete_auth_hash` | empty | Authorization hash used by the VM log deletion API. |

`poworker` and `diaworker` do not open local listeners. Their `connect` value is
a remote full-node endpoint and therefore remains a single `host:port` value;
its default is `127.0.0.1:8082`, matching `[server]` above. `boot_nodes` follows
the same remote-endpoint rule and remains a comma-separated `host:port` list.

### `[mint]`, `[miner]`, and `[diamond_miner]`

| Section/key | Default | Meaning |
| --- | ---: | --- |
| `mint.chain_id` | `0` | `0` is mainnet. |
| `mint.diamond_form` | `true` | Genesis diamond ownership form; persisted and immutable after initialization. |
| `miner.enable` | `false` | Expose block mining work. |
| `miner.reward` | empty | Required Hacash reward address when mining is enabled. |
| `miner.message` | empty | Coinbase message, truncated/padded to 16 bytes. |
| `diamond_miner.enable` | `false` | Enable automatic diamond mining/bidding integration. |
| `diamond_miner.reward` | empty | Required private-key-type reward address when enabled. |
| `diamond_miner.bid_password` | empty | Password used to derive the bidding account. Treat as a secret. |
| `diamond_miner.bid_min` | empty | Minimum bid amount, in normal amount text format. |
| `diamond_miner.bid_max` | empty | Maximum bid amount. |
| `diamond_miner.bid_step` | empty | Bid increment. |

The old fdev keys are not aliases. In particular, migrate `data_dir` to
`[engine].data_dir`, `[node]` to `[p2p]`, split each `listen` value into
`listen_ip` and `listen_port`, migrate `boots` to `boot_nodes`, `[server].diamond_form` to
`[mint].diamond_form`. Block, transaction, and difficulty limits are fixed
Hacash chain rules and are not INI settings.

## 8. Startup and shutdown flow

The fnext composition root is `app::fullnode`:

1. Load and type-check INI configuration; load/create the P2P node identity.
2. Build the action/transaction registry and Hacash consensus runtime.
3. Open `block`, versioned `state`, and `vmlog` stores through the selected DB.
4. Open the chain engine and recover/replay persisted chain state.
5. Create the grouped memory transaction pool and P2P node.
6. Register chain listeners and assemble status, chain, pool, account, mint and
   VM HTTP services.
7. Start consensus/node hooks, optional indexer and diamond bidder, then P2P and
   HTTP listeners.
8. On Ctrl-C, stop admission/workers and flush/shut down the chain engine.

The major dependency direction is `sys -> field -> base`, with `protocol`,
`vm`, `mint`, `chain`, `node`, `server`, `api`, `db`, and `x16rs` implementing
domain capabilities above `base`; `app` is the only standard full-node
composition root that wires them together.
