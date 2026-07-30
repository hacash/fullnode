# Hacash Fullnode

A Rust implementation of the Hacash full node. This Cargo workspace contains protocol codecs, consensus, chain state, P2P networking, HTTP APIs, the contract VM, database backends, mining workers, and a JavaScript/WASM SDK. The `app` crate assembles the standard node and produces the `fullnode` executable.

## Build and Run

### Prerequisites

- A current stable Rust toolchain. The workspace uses Rust edition 2024.
- Basic build tools for the default `sled` database backend on Linux:

```bash
rustup update stable
sudo apt update
sudo apt install build-essential pkg-config
```

Native LevelDB and RocksDB builds also require CMake, Clang, and libclang:

```bash
sudo apt install build-essential cmake clang libclang-dev pkg-config
```

### Build the full node

The default build uses `sled`:

```bash
cargo build --release -p app --bin fullnode
```

The executable is written to `target/release/fullnode`. Omit `--release` for a development build. The other executables can be built independently:

```bash
cargo build --release -p app --bin poworker
cargo build --release -p app --bin diaworker
cargo build --release -p app --bin fitshc
```

Build an OpenCL-enabled mining worker with:

```bash
cargo build --release -p app --bin poworker --features ocl
```

The database backend is selected at **compile time**, not in the INI file. Disable default features when selecting another backend, and never open the same data directory with different backends:

```bash
# Pure-Rust LevelDB
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-rusty-leveldb

# Native LevelDB
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-leveldb-sys

# RocksDB
cargo build --release -p app --bin fullnode \
  --no-default-features --features db-rocksdb
```

See [build.md](build.md) for platform-specific builds, static linking, and SDK packaging.

### Configure the node

Start with the mainnet example included in the repository:

```bash
cp docs.loc/hacash.config.ini ./hacash.config.ini
./target/release/fullnode "$(pwd)/hacash.config.ini"
```

Passing an absolute configuration path is recommended. With no argument, the node reads `hacash.config.ini` from the **executable's directory**. A relative argument is also resolved from the executable's directory, not the shell's current directory.

A minimal persistent-node configuration is shown below:

```ini
[engine]
data_dir = ./hacash_mainnet_data
fast_sync = false

[p2p]
listen_ip = 0.0.0.0
listen_port = 3337
boot_nodes = 182.92.163.225:3337,54.193.49.59:3337,54.219.80.127:3337

[server]
enable = false
listen_ip = 127.0.0.1
listen_port = 8082
debug_routes = false

[mint]
chain_id = 0
diamond_form = true

[txpool]
min_fee_purity = 6024

[miner]
enable = false

[diamond_miner]
enable = false

[vm]
log_enable = false
```

Important operational details:

- An empty `data_dir` uses in-memory storage. A production node must use a persistent directory.
- The data directory contains `block/`, `state_vN/`, `vmlog/`, `node.id`, and `stable.nodes`.
- HTTP is disabled by default. Set `server.enable = true` to listen on `127.0.0.1:8082`; before exposing it publicly, use a firewall or reverse proxy and keep `debug_routes = false`.
- Setting the relevant `listen_port` to `0` also disables P2P or HTTP listening.
- `HACASH_DATA_DIR` overrides `engine.data_dir`; `HACASH_DB_SYNC=1` requests synchronous writes; `HACASH_DB_SMALL_MACHINE=1` enables small-machine database tuning.
- Unknown keys in a recognized INI section cause startup to fail. Unknown top-level sections are allowed for independently owned components such as external indexers.
- `mint.diamond_form` is persisted in genesis state and cannot be changed after a data directory has been initialized.

See [config.md](config.md) for all options, defaults, and INI syntax rules.

## Architecture

The project keeps stable interfaces in lower-level crates and performs concrete assembly in the application layer. `base` defines cross-module traits, `chain` does not depend on the concrete Hacash protocol, and `server` handles HTTP transport only. The `app` crate selects and connects the standard protocol, consensus, storage, networking, and APIs.

```text
                         app (composition root)
                   /       |       |       \
             protocol     mint     vm      api
                  \         |       /       |
                   +--------base------------+
                    /        |       \      \
                 chain      node    server   db
                    \         |       /      /
                     +------ field ----------+
                              |
                             sys
```

The main crates are:

| Crate | Responsibility |
| --- | --- |
| `sys` | Errors, bytes, hashes, accounts, INI parsing, and shutdown coordination |
| `field` | On-chain field types and binary/JSON codecs |
| `base` | Shared Block, Transaction, Action, State, Engine, Node, API, Registry, and Scaner traits |
| `protocol` | Standard Hacash block, transaction, and Action codecs and execution context |
| `mint` | Hacash consensus, genesis, difficulty, mining, diamond/channel/asset rules, and related APIs |
| `vm` | Contract VM, runtime, host calls, logs, and VM APIs |
| `chain` | Fork tree, block validation and execution, synchronization, stable-block persistence, and state snapshots |
| `node` | P2P, discovery, synchronization pipeline, mempool, broadcast, and admission |
| `db` | `sled`, LevelDB, and RocksDB backends plus the block/state/vmlog storage layout |
| `server` | Axum-based HTTP routing and service dispatch |
| `api` | Consensus-independent status, chain, pool, and account APIs |
| `app` | Registry, configuration, component assembly, lifecycle, mining workers, and binary entry points |
| `sdk` | Account, signing, and transaction SDK for JavaScript/WASM |

Standard startup and data flow:

```text
Read INI
  -> build Registry + HacashConsensus + Store
  -> recover block/state into ChainEngine
  -> build MemTxPool + P2PNode
  -> merge generic, mint, VM, and indexer APIs
  -> start indexer catch-up, P2P, and HTTP

network block -> P2P sync/discovery -> ChainEngine validate/execute -> fork tree
                                                            -> persist stable blocks
                                                            -> notify ChainListener
```

A single mutex serializes block insertion. Readers use owned state snapshots and do not block the writer. Once the fork window exceeds `engine.unstable_block`, stable blocks and state are committed to the database in a batch. `Ctrl-C` uses a shared `Waiter` to coordinate shutdown across the indexer, P2P node, HTTP server, and chain engine.

The primary assembly entry points are:

- `app/src/registry.rs`: standard Registry registration order
- `app/src/fullnode/assemble.rs`: consensus, Engine, Node, API, and indexer assembly
- `app/src/fullnode/mod.rs`: startup and shutdown lifecycle
- `chain/src/lib.rs`: chain-engine design overview

## Customization

### 1. Customize a protocol or sidechain with Registry

`base::RegistryWriter` is the registration boundary for protocol components. It supports registering:

- block hashing, block construction, and block-size probing
- binary and JSON codecs for transactions and Actions
- execution-context creation and VM assignment
- VM host action/env/view definitions and execution parameters

The standard assembly is:

```rust
let mut registry = app::Registry::new(mint::block_hasher);
protocol::register_standard(&mut registry, &protocol::PROTOCOL_PARAMS)?;
mint::register(&mut registry)?;
vm::register(&mut registry)?;
```

A custom chain should normally add a separate application crate as its composition root. Replace the hasher, codecs, Actions, VM parameters, and `ConsensusRuntime` as needed, then follow `app/src/fullnode/assemble.rs` to assemble `ChainEngine`, `P2PNode`, and HTTP services. Registry rejects duplicate transaction types, Action kinds, and VM host IDs, so extensions must allocate non-conflicting identifiers.

`mint.chain_id` enters the execution environment and affects some consensus branches, but **changing `chain_id` alone does not isolate a sidechain**. The current P2P magic is not derived from `chain_id`. An independent network must also review or customize genesis state, consensus parameters, block hashing and difficulty, protocol upgrade heights, P2P magic, boot nodes, data directories, and SDK/signature domains. Nodes with incompatible consensus or codecs must not connect to mainnet.

### 2. Develop a third-party indexer or block explorer

An indexer is not embedded in `ChainEngine`. An external crate implements `base::Scaner` and injects it at the application layer:

```rust
use std::sync::Arc;
use base::{ApiService, BlockRef, Scaner, ScanerView};
use sys::{Rerr, Waiter};

struct ExplorerIndexer;

impl Scaner for ExplorerIndexer {
    fn name(&self) -> &str { "explorer" }

    fn sync(&self, view: Arc<dyn ScanerView>) -> Rerr {
        // Read history after the local checkpoint and enqueue it for indexing.
        let _history = view.block_history();
        Ok(())
    }

    fn on_block(&self, block: BlockRef, view: Arc<dyn ScanerView>) {
        // Enqueue quickly; perform database writes in an indexer-owned worker.
        let _ = (block, view);
    }

    fn api_services(&self) -> Vec<Arc<dyn ApiService>> {
        // These routes are merged into the full node's HTTP server.
        vec![]
    }

    fn start(&self, waiter: Waiter) -> Rerr {
        // Start background work and observe waiter for graceful shutdown.
        let _ = waiter;
        Ok(())
    }
}

fn main() -> Rerr {
    app::run_with_scaner(Arc::new(ExplorerIndexer))
}
```

Integration requirements:

- `sync` catches up historical blocks from the indexer's own checkpoint.
- `on_block` receives stable-block notifications only. It must return quickly and write asynchronously; an indexer failure must not block or roll back the chain.
- `ScanerView` exposes block history and balance queries at a specified snapshot. It intentionally does not expose raw database handles or arbitrary state KV access.
- Explorer routes are returned by `ApiService::routes()` using `ApiRoute::get/post`. Debug routes use `debug_get/debug_post` and are controlled by `server.debug_routes`.
- An external indexer may own a top-level INI section such as `[hascan]`. The full node ignores unknown sections, leaving the indexer to parse and validate its own configuration.
- A separately deployed explorer can consume the existing HTTP APIs. An in-process `Scaner` additionally receives stable-block events and consistent read-only snapshots without polling gaps.

### 3. Other extension points

- **Database:** select one of four backends with Cargo features, or implement `base::DiskDB`/`Store` and inject it from a custom composition root.
- **HTTP API:** implement `base::ApiService` and include its routes in the service list passed to `server::HttpServer::open`.
- **Chain events:** implement `base::ChainListener` to observe accepted and stable blocks. Listeners are observational and cannot reject or roll back blocks.
- **Mempool:** use `txpool.maxs` to override per-group capacities. `txpool.min_fee_purity` controls local admission and relay, not block validity.
- **Node role:** use `p2p.listen_port = 0` for an outbound-only node and `server.enable = false` to disable HTTP. Mining, automatic diamond bidding, and VM logs are independently configurable.
- **SDK:** `sdk/pack.sh` generates Node.js, Web ESM, and inline-page builds for wallets, explorers, and transaction-building tools.

Consensus rules, protocol encodings, and persistence formats are compatibility boundaries. Before deploying a customized chain, use a fresh data directory and an isolated P2P network, then test genesis, replay, forks, synchronization, upgrade heights, and cross-version codecs.
