# Hacash Fullnode Configuration Reference

This document is the user manual for `hacash.config.ini`, the configuration file
consumed by the Hacash fullnode (`fullnodenext`).

It describes **every** option the fullnode recognizes, grouped by INI section.
For each option it lists the key name, the value type, the built-in default
(used when the key is omitted or set to empty), and a short explanation of what
the option controls.

---

## How the config file is loaded

* The file is plain INI text. Lines starting with `;` or `#` are comments
  (except inside quoted values). Sections are written as `[section]` and keys
  as `key = value`.
* Empty values are treated as "not set": the field falls back to its default.
  To set an empty string explicitly, write `key = ""`.
* Comma-separated values (e.g. `boot_nodes`, `txpool.maxs`) are parsed as lists.
* Booleans accept `true` / `1` and `false` / `0`.
* Unknown sections are ignored, but **unknown keys inside a known section are
  rejected** at startup. Remove or comment out any typo'd key.
* Each known section uses `#[serde(default)]`, so **every** key below is
  optional — omitting a whole section is fine and yields all defaults.
* The config path defaults to `hacash.config.ini` **next to the executable**.
  A relative path is resolved against the executable directory, not the current
  working directory. You can override the path by passing it as the first
  command-line argument.

### Environment variables that override the config

| Variable | Effect |
| --- | --- |
| `HACASH_DATA_DIR` | Overrides `[engine].data_dir` (and the P2P `node.id` location, which mirrors it). |
| `HACASH_DB_SYNC` | Enables synchronous database writes (slower, more durable). |
| `HACASH_DB_SMALL_MACHINE` | Switches to small-machine friendly database tuning. |

---

## `[engine]`

Chain engine, validation, pipeline and storage knobs.
Struct: `base::EngineConfig`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `data_dir` | string | `""` | Root directory for all chain state, indexes and the P2P `node.id` file. A relative path is resolved against the executable directory. Overridden by the `HACASH_DATA_DIR` environment variable. **Required in practice** — the node cannot operate without a writable data directory. |
| `fast_sync` | bool | `false` | If `true`, the node performs a fast initial sync that skips full validation of historical blocks. Set to `false` to fully validate every block since genesis. |
| `unstable_block` | u64 | `4` | Retention / reorg window: how many recent blocks are kept in memory for replay and reorganization. Blocks older than `tip − unstable_block` are pruned from the in-memory view. |
| `recent_blocks` | bool | `true` | Whether to retain and serve the recent-blocks view. When `false`, the recent block list is empty and is not served. |
| `average_fee_purity` | bool | `true` | Whether the engine tracks a running average fee purity on each new head. When false, the average is reported as the local mempool minimum fee purity. |
| `show_miner_name` | bool | `false` | If `true`, block-submission logs include the miner name / detail. |

---

## `[p2p]`

Peer-to-peer networking configuration.
Struct: `base::P2PConfig`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `listen_ip` | IP address | `0.0.0.0` | IP address the P2P listener binds to. `0.0.0.0` binds on all interfaces. |
| `listen_port` | u16 | `3337` | TCP port for inbound P2P connections. Set to `0` to disable inbound acceptance (outbound-only node). |
| `node_name` | string | `""` | Human-readable peer name advertised in handshakes. If left empty, the loader assigns `"hn"` + the first 8 hex characters of the auto-generated `node_key`. Padded to 16 bytes on the wire, so keep it short. |
| `boot_nodes` | list of strings | `[]` | Comma-separated seed peer addresses used for initial discovery, e.g. `1.2.3.4:3337,5.6.7.8:3337`. Each entry is `host:port`. |
| `find_nodes` | bool | `true` | If `true`, the node runs DHT-style `find_nodes` queries to discover new peers. |
| `accept_nodes` | bool | `true` | If `false`, the node refuses all inbound P2P connections. |
| `use_stable_nodes` | bool | `true` | If `true`, public backbone addresses from `stable.nodes` are loaded before boot nodes. If `false` and no `boot_nodes` are configured, the node cannot bootstrap. |
| `backbone_peers` | usize | `4` | Target number of stable "backbone" peer connections to maintain. |
| `offshoot_peers` | usize | `200` | Target number of non-backbone ("offshoot") peer connections. Total peer capacity is `backbone_peers + offshoot_peers`. |
| `block_queue_cap` | usize | `8` | Capacity of the inbound block queue used by the sync pipeline. |

> The node identity is **not** read from the INI. `node_key` (a 16-byte key) is
> loaded from — or generated into — `<data_dir>/node.id`. The `data_dir` used by
> the P2P layer mirrors `[engine].data_dir`.

---

## `[server]`

HTTP API server configuration.
Struct: `base::ServerConfig`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `enable` | bool | `false` | Enables the HTTP API listener. When `false`, no HTTP socket is bound and no API routes are served. |
| `listen_ip` | IP address | `127.0.0.1` | IP the HTTP API server binds to. Defaults to localhost for safety; set to `0.0.0.0` to expose the API to the network. |
| `listen_port` | u16 | `8082` | TCP port for the HTTP API server. |
| `debug_routes` | bool | `false` | If `true`, HTTP routes marked `debug` are registered and served. When `false` (the default), debug routes are filtered out and inaccessible. |

---

## `[mint]`

Network identity and genesis-form parameters.
Struct: `mint::MintConf`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `chain_id` | u32 | `0` (mainnet) | Network / chain identifier. This is the authoritative chain id and must fit in a u32. |
| `diamond_form` | bool | `true` | "Diamond form" consensus variant (dev-compatible genesis configuration). **Persisted into the genesis state at first init and cannot be changed afterwards** — editing it on an existing chain state fails startup with a mismatch error. |

Block, transaction, and difficulty limits are fixed `mint::MintParams` rules;
they cannot be overridden by this file.

---

## `[txpool]`

Transaction pool configuration.
Struct: `TxPoolConfig`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `maxs` | list of usize | `[]` | Comma-separated per-group transaction-pool capacity caps, e.g. `maxs = 2000, 100`. Applied positionally: the i-th value overrides the i-th tx-pool group's default capacity. An empty list means each group keeps its built-in default. **When `[miner].enable = false`, every group is first clamped to `10` before these overrides are applied**, so a non-mining node cannot grow an unbounded mempool. Capacity, ordering, and replacement decisions affect local retention only; a valid transaction that passes relay admission is still forwarded when it is not retained. |
| `min_fee_purity` | u64 | `6024` | Local fee-per-byte floor for mempool admission and relay. It does not change block validity. |

---

## `[miner]`

Standard (PoW) block miner configuration.
Struct: `MinerFileConfig`, converted to `mint::MinerConf`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `enable` | bool | `false` | Enables the standard PoW block miner. Also gates transaction-pool capacity: when `false`, all tx-pool groups are clamped to `10` (see `[txpool].maxs`). |
| `reward` | string | `""` | Readable Hacash address that receives the block coinbase reward. Must be a valid readable address; an invalid value aborts startup. Only used when `enable = true`. |
| `message` | string | `""` | Free-text miner comment embedded in the coinbase transaction. Truncated or space-padded to a fixed 16 bytes. |

---

## `[diamond_miner]`

Diamond-mint bidding miner configuration.
Struct: `DiamondMinerFileConfig`, converted to the diamond fields of `mint::MinerConf`.
All keys are only consumed when `enable = true`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `enable` | bool | `false` | Enables the diamond-mint bidding miner. |
| `reward` | string | `""` | Readable address receiving diamond-mint rewards. **Must be a PRIVKEY-type address** (an address carrying a private key); otherwise startup fails. |
| `bid_password` | string | `"123456"` (when derived) | Password used to derive the bidding account. Parsed via `Account::create_by`. An invalid password aborts startup. |
| `bid_min` | string (amount) | `1 HAC` (compressed) | Minimum diamond bid amount. Parsed as a Hacash `Amount` string, then compressed. |
| `bid_max` | string (amount) | `31 HAC` (compressed) | Maximum diamond bid amount. Parsed the same way as `bid_min`. |
| `bid_step` | string (amount) | `5.247 HAC` (compressed) | Bid step granularity. Parsed the same way as `bid_min`. |

---

## `[vm]`

VM execution logging and log-management configuration.
Struct: `base::VmConfig`.

| Key | Type | Default | Description |
| --- | --- | --- | --- |
| `log_enable` | bool | `false` | Master switch for VM execution logging. Logging only actually turns on when `log_enable = true` **and** the current block height is `>= log_open_height`. |
| `log_open_height` | u64 | `0` | Block height at or after which VM logging becomes active (only meaningful when `log_enable = true`). |
| `log_delete_auth_hash` | string | `""` | Authorization hash required to call the VM API's `vm_logs_delete` endpoint. Passed into the VM API service registry and stored on the `VmApi` instance. An empty string disables authorization for that endpoint — set a non-empty hash to require callers to present it. |

---

## Example configuration

A minimal mainnet example (matches the shipped `hacash.config.ini`):

```ini
; Hacash fullnodenext mainnet example.
; The fullnode resolves a relative config path from the executable directory.

[engine]
data_dir = ./hacash_mainnet_data
fast_sync = false
show_miner_name = true

[p2p]
listen_ip = 0.0.0.0
listen_port = 3337
node_name =
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
maxs =

[miner]
enable = false

[diamond_miner]
enable = false

[vm]
log_enable = false
log_open_height = 0
log_delete_auth_hash =
```

A more complete example showing the optional knobs:

```ini
[engine]
data_dir = ./hacash_mainnet_data
fast_sync = false
unstable_block = 4
recent_blocks = true
average_fee_purity = true
show_miner_name = true

[p2p]
listen_ip = 0.0.0.0
listen_port = 3337
node_name =
boot_nodes = 182.92.163.225:3337,54.193.49.59:3337,54.219.80.127:3337
find_nodes = true
accept_nodes = true
use_stable_nodes = true
backbone_peers = 4
offshoot_peers = 200
block_queue_cap = 8

[server]
enable = false
listen_ip = 127.0.0.1
listen_port = 8082
debug_routes = false

[mint]
chain_id = 0
diamond_form = true

[txpool]
maxs =
min_fee_purity = 6024

[miner]
enable = false
reward =
message =

[diamond_miner]
enable = false
reward =
bid_password =
bid_min =
bid_max =
bid_step =

[vm]
log_enable = false
log_open_height = 0
log_delete_auth_hash =
```

---

## Notes & caveats

1. **`diamond_form` is genesis-persistent.** Once the chain state is
   initialized, changing `[mint].diamond_form` causes startup to fail with a
   mismatch error; it cannot be toggled on an existing chain.
2. **`node_key` and `p2p.data_dir` are not read from the INI.** `node_key` is
   loaded from (or generated into) `<data_dir>/node.id`; `p2p.data_dir` mirrors
   `[engine].data_dir`.
3. **`[diamond_miner].reward` must be a PRIVKEY-type address**, otherwise
   startup fails.
4. **Unknown keys inside a known section are rejected.** A typo such as
   `[engne]` (wrong section name) is silently ignored, but a typo like
   `lisen_ip = ...` inside `[p2p]` aborts startup. Comment out or remove unused
   keys rather than leaving them misspelled.
5. **Worker binaries use separate config files**, not `hacash.config.ini`:
   * `poworker.config.ini` — PoW worker (sections `[default]` and `[gpu]`).
   * `diaworker.config.ini` — diamond worker (sections `[default]` and `[gpu]`).
