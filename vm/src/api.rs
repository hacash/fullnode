//! VM API service.

use std::sync::Arc;

use base::{
    ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService, BlockInfo, Env, TransactionCreator,
    TxCreateRequest, TxInfo, api_state_read_error,
};
use field::{AddrOrList, Address, Amount, Hash};
use sys::Ret;

use crate::action::{P2shLeafSpec, P2shTool};
use crate::machine::{self, SandboxSpec};
use crate::rt::{CodeConf, GasExtra, SpaceCap};
use crate::state::{VMStateRead, VmLog};
use crate::value::{ContractAddress, Value};

pub struct VmApi {
    tx_creator: Arc<dyn TransactionCreator>,
    log_delete_auth_hash: String,
}

impl VmApi {
    pub fn new(tx_creator: Arc<dyn TransactionCreator>, log_delete_auth_hash: String) -> Self {
        Self {
            tx_creator,
            log_delete_auth_hash,
        }
    }
}

fn json_string(s: &str) -> String {
    // Shared escaper from field (no serde): matches serde_json's default
    // string output.
    field::json_escape(s)
}

fn api_error(errmsg: &str) -> ApiResponse {
    ApiResponse::json(format!(r#"{{"ret":1,"err":{}}}"#, json_string(errmsg)))
}

fn api_data_raw(s: String) -> ApiResponse {
    ApiResponse::json(format!(r#"{{"ret":0,{}}}"#, s))
}

fn req_hex(s: &str) -> Ret<Vec<u8>> {
    hex::decode(s.trim_start_matches("0x")).map_err(|_| sys::Error::fault("hex format invalid"))
}

fn req_addr(s: &str) -> Ret<Address> {
    Address::from_readable(s)
        .map_err(|e| sys::Error::fault(format!("address {} format invalid: {}", s, e)))
}

fn vm_status_handler(ctx: &ApiExecCtx, _req: ApiRequest) -> ApiResponse {
    ApiResponse::json(format!(
        "{{\"ret\":0,\"vm\":\"enabled\",\"height\":{}}}",
        ctx.engine.latest_height()
    ))
}

fn vm_logs_read(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let height = req.query_u64("height").unwrap_or(0);
    let index = req.query_usize("index").unwrap_or(0);
    let stable_hei = ctx
        .engine
        .latest_height()
        .saturating_sub(ctx.engine.config().unstable_block);
    if height > stable_hei {
        return api_data_raw(r#""unstable":true"#.to_owned());
    }
    if !ctx.engine.config().is_open_vmlog(height) {
        return api_data_raw(r#""end":true"#.to_owned());
    }
    let logs = match ctx.engine.store().log_backend().load_block_logs(height) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let Some(entry) = logs.get(index) else {
        return api_data_raw(r#""end":true"#.to_owned());
    };
    if entry.topic != "vm" {
        return api_data_raw(r#""ignore":true"#.to_owned());
    }
    let item = match VmLog::from_bytes(&entry.data) {
        Ok(v) => v,
        Err(_) => return api_error("log format invalid"),
    };
    let ignore = api_data_raw(r#""ignore":true"#.to_owned());
    if let Some(qadr) = req.query("address") {
        let Ok(addr) = req_addr(qadr) else {
            return api_error("address format invalid");
        };
        if addr != item.addr {
            return ignore;
        }
    }
    macro_rules! filter_topic {
        ($key:expr, $topic:expr) => {
            if let Some(tp) = req.query($key) {
                let Ok(raw) = req_hex(tp) else {
                    return api_error("hex format invalid");
                };
                if raw.as_slice() != $topic.raw() {
                    return ignore;
                }
            }
        };
    }
    filter_topic!("topic0", &item.topic0);
    filter_topic!("topic1", &item.topic1);
    filter_topic!("topic2", &item.topic2);
    filter_topic!("topic3", &item.topic3);
    api_data_raw(format!(
        r#""height":{},"index":{},{}"#,
        height,
        index,
        item.render()
    ))
}

fn vm_logs_delete(ctx: &ApiExecCtx, req: ApiRequest, auth_hash: &str) -> ApiResponse {
    let height = req.query_u64("height").unwrap_or(0);
    let auth_hash = auth_hash.trim();
    if !auth_hash.is_empty() {
        let auth = req.query("auth").unwrap_or("").trim();
        if auth != auth_hash {
            return api_error("auth failed");
        }
    }
    match ctx.engine.store().log_backend().remove_block_logs(height) {
        Ok(()) => api_data_raw(format!(r#""height":{},"deleted":true"#, height)),
        Err(e) => api_error(&e.to_string()),
    }
}

fn peer_ip_from_req(req: &ApiRequest) -> Option<String> {
    // No trusted-proxy config, so forwarding headers are client-controlled; rate limit by TCP peer only.
    req.peer_ip.map(|ip| ip.to_string())
}

fn contract_sandbox_call(
    ctx: &ApiExecCtx,
    req: ApiRequest,
    tx_creator: &dyn TransactionCreator,
) -> ApiResponse {
    // §13.2: acquire a global + per-IP concurrency permit (held for the call, gives a
    // wall-clock deadline that supersedes any gas-burning loop) BEFORE touching the snapshot.
    let permit = match ctx.sandbox_limiter.acquire(peer_ip_from_req(&req)) {
        Ok(p) => p,
        Err(reason) => return api_error(reason),
    };
    let services = ctx.engine.services().clone();
    // §13.2: optimistic snapshot, no StateGate held; busy (`Ok(None)`) reports "state changed",
    // fatal/stopping engine (`Err`) surfaces the unavailable error (§5 error-system design).
    let Some(snapshot) = (match ctx.engine.optimistic_canonical() {
        Ok(snapshot) => snapshot,
        // EngineUnavailable (fatal/stopping) maps to HTTP 503 through the
        // single API mapping entry (§8.2 of the error-system design).
        Err(e) => return api_state_read_error(&e),
    }) else {
        return api_error("state changed during sandbox call");
    };
    let start_epoch = snapshot.epoch;
    let height = snapshot.head_height.saturating_add(1);
    let contract = req.query("contract").unwrap_or("");
    let function = req.query("function").unwrap_or("").trim().to_owned();
    let params = req.query("params").unwrap_or("");
    let Ok(addr) = Address::from_readable(contract) else {
        return api_error("contract address format invalid");
    };
    let Ok(ctrladdr) = ContractAddress::from_addr(addr) else {
        return api_error("contract address version error");
    };
    if function.is_empty() {
        return api_error("function cannot be empty");
    }
    let caller = match req.query("caller") {
        Some(a) => match req_addr(a) {
            Ok(v) => v,
            Err(_) => return api_error("caller address format invalid"),
        },
        None => ctx.engine.block_producer().external_exec_author(),
    };

    let args = match machine::parse_sandbox_params(params) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let mut spec = SandboxSpec::new(ctrladdr, function)
        .args(args)
        .caller(caller);
    if let Some(gmx) = req.query("gas_max").and_then(|s| s.parse::<u8>().ok()) {
        spec = spec.gas_max_byte(gmx);
    }

    let (tx_gas_max, _) = match machine::resolve_sandbox_gas(&spec) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };

    // vm only describes the common transaction envelope. The injected creator
    // selects and constructs the protocol's concrete transaction type.
    let addrlist = match AddrOrList::from_list(vec![caller, ctrladdr.into_addr()]) {
        Ok(list) => list,
        Err(e) => return api_error(&e.to_string()),
    };
    let tx = match tx_creator.create(
        TxCreateRequest::new(3, caller, Amount::unit238(machine::SANDBOX_TX_FEE), height)
            .with_addrlist(addrlist)
            .with_gas_max(tx_gas_max),
    ) {
        Ok(t) => t,
        Err(e) => return api_error(&e.to_string()),
    };

    let mut env = Env::default();
    env.chain.id = ctx.engine.consensus().chain_id();
    // Bind the sandbox caller as both tx main and block author so all caller-facing host reads see it.
    env.block = BlockInfo {
        height,
        hash: Hash::default(),
        author: caller,
    };
    env.tx = TxInfo {
        ty: tx.ty(),
        main: tx.main(),
        addrs: tx.addrs(),
        fee: tx.fee().clone(),
    };

    let chunk = snapshot.begin_tx(tx.hash());
    let mut ctxobj = match services.create_context(env, chunk, tx) {
        Ok(c) => c,
        Err(e) => return api_error(&e.to_string()),
    };
    // Install the permit deadline into the VM (checked at each instruction; unset in consensus execution).
    if let Some(vm) = ctxobj.vm_peek() {
        vm.set_deadline(Some(permit.deadline()));
    }
    if let Err(reason) = permit.check_deadline() {
        return api_error(reason);
    }
    let callres = match machine::sandbox_call(ctxobj.as_mut(), spec) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    if let Err(reason) = permit.check_deadline() {
        return api_error(reason);
    }
    // §13.2 / §6: discard sandbox result if state moved underneath us.
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed during sandbox call");
    }
    api_data_raw(format!(
        r#""use_gas":{},"gas_use":{{"compute":{},"resource":{},"storage":{}}},"ret_val":{}"#,
        callres.use_gas,
        callres.gas_use.compute,
        callres.gas_use.resource,
        callres.gas_use.storage,
        callres.ret_val.to_debug_json()
    ))
}

fn debug_contract_storage(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    // §13.2: same sandbox limiter envelope as contract_sandbox_call (debug-only, still rate-limited).
    let _permit = match ctx.sandbox_limiter.acquire(peer_ip_from_req(&req)) {
        Ok(p) => p,
        Err(reason) => return api_error(reason),
    };
    // §13.2: optimistic snapshot, no StateGate held; busy (`Ok(None)`) reports "state changed",
    // fatal/stopping engine (`Err`) surfaces the unavailable error (§5 error-system design).
    let Some(snapshot) = (match ctx.engine.optimistic_canonical() {
        Ok(snapshot) => snapshot,
        // EngineUnavailable (fatal/stopping) maps to HTTP 503 through the
        // single API mapping entry (§8.2 of the error-system design).
        Err(e) => return api_state_read_error(&e),
    }) else {
        return api_error("state changed during contract storage query");
    };
    let start_epoch = snapshot.epoch;
    let height = snapshot.head_height.saturating_add(1);
    let contract = req.query("contract").unwrap_or("");
    let key = req.query("key").unwrap_or("");
    let kind = req.query("kind").unwrap_or("storage");

    let Ok(addr) = Address::from_readable(contract) else {
        return api_error("contract address format invalid");
    };
    if ContractAddress::from_addr(addr).is_err() {
        return api_error("contract address version error");
    }
    if key.is_empty() {
        return api_error("key cannot be empty");
    }
    let args = match machine::parse_sandbox_params(key) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    if args.len() != 1 {
        return api_error("key must decode to exactly one value");
    }
    let key = args.into_iter().next().unwrap_or(Value::Nil);
    let gst = GasExtra::new(height);
    let cap = SpaceCap::new(height);
    let state = VMStateRead::wrap(snapshot.view());

    let response = match kind {
        "status" => match state.debug_status_get(&cap, &addr, &key) {
            Ok(v) => api_data_raw(format!(
                r#""height":{},"kind":"status","exists":{},"value":{}"#,
                height,
                !matches!(v, Value::Nil),
                v.to_debug_json()
            )),
            Err(e) => api_error(&e.to_string()),
        },
        "storage" | "" => match state.debug_storage_get(&gst, &cap, height, &addr, &key) {
            Ok(Some(info)) => api_data_raw(format!(
                r#""height":{},"kind":"storage","exists":true,"active":{},"recoverable":{},"live_rest":{},"recover_rest":{},"value":{}"#,
                height,
                info.active,
                info.recoverable,
                info.live_blocks,
                info.recover_blocks,
                info.value.to_debug_json()
            )),
            Ok(None) => api_data_raw(format!(
                r#""height":{},"kind":"storage","exists":false,"active":false,"recoverable":false,"live_rest":0,"recover_rest":0,"value":{}"#,
                height,
                Value::Nil.to_debug_json()
            )),
            Err(e) => api_error(&e.to_string()),
        },
        _ => api_error("kind must be storage or status"),
    };
    if !ctx.engine.validate_optimistic(start_epoch) {
        return api_error("state changed during contract storage query");
    }
    response
}

/// Debug: build canonical P2SH scriptmh address from one leaf.
/// Query: `libs` (comma-separated contract addrs, optional), `codeconf` (u8), `lockbox` (hex).
fn debug_p2sh_scriptmh(_ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let lockbox_hex = req.query("lockbox").unwrap_or("");
    if lockbox_hex.is_empty() {
        return api_error("lockbox hex required");
    }
    let lockbox_raw = match req_hex(lockbox_hex) {
        Ok(v) => v,
        Err(_) => return api_error("lockbox hex format invalid"),
    };
    let lockbox = match field::BytesW2::from(lockbox_raw) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let codeconf_raw = req.query_u64("codeconf").unwrap_or(0) as u8;
    let codeconf = match CodeConf::parse(codeconf_raw) {
        Ok(v) => v,
        Err(e) => return api_error(&e.to_string()),
    };
    let mut libs = crate::contract::ContractAddrListW1::default();
    if let Some(libs_q) = req.query("libs") {
        for part in libs_q.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let Ok(addr) = Address::from_readable(part) else {
                return api_error("libs address format invalid");
            };
            let Ok(caddr) = ContractAddress::from_addr(addr) else {
                return api_error("libs contract address version error");
            };
            if let Err(e) = libs.push(caddr) {
                return api_error(&e.to_string());
            }
        }
    }
    let spec = P2shLeafSpec {
        adrlibs: libs,
        codeconf,
        lockbox,
    };
    match P2shTool::build_canonical_tree(vec![spec]) {
        Ok(tree) => api_data_raw(format!(
            r#""address":"{}","root":"{}""#,
            tree.address().to_readable(),
            hex::encode(tree.root_sha3().as_bytes())
        )),
        Err(e) => api_error(&e.to_string()),
    }
}

impl ApiService for VmApi {
    fn name(&self) -> &str {
        "vm"
    }

    fn routes(&self) -> Vec<ApiRoute> {
        let tx_creator = self.tx_creator.clone();
        let log_delete_auth_hash = self.log_delete_auth_hash.clone();
        vec![
            ApiRoute::get("/vm/status", vm_status_handler),
            ApiRoute::get("/query/contract/sandboxcall", move |ctx, req| {
                contract_sandbox_call(ctx, req, tx_creator.as_ref())
            }),
            ApiRoute::get("/query/contract/logs", vm_logs_read),
            ApiRoute::get("/operate/contract/logs/delete", move |ctx, req| {
                vm_logs_delete(ctx, req, &log_delete_auth_hash)
            }),
            ApiRoute::debug_get("contract/storage", debug_contract_storage),
            ApiRoute::debug_get("p2sh/scriptmh", debug_p2sh_scriptmh),
        ]
    }
}

pub fn api_services(
    tx_creator: Arc<dyn TransactionCreator>,
    log_delete_auth_hash: String,
) -> Vec<Arc<dyn ApiService>> {
    vec![Arc::new(VmApi::new(tx_creator, log_delete_auth_hash))]
}
