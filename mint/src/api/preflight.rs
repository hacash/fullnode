use std::sync::Arc;
use std::time::Instant;

use base::{
    ApiExecCtx, ApiRequest, ApiResponse, Env, ExecutionServices, StateChunkRef, TxInfo, TxRef,
};
use sys::ToHex;

use crate::api::util::{api_error, api_state_read_error, body_data_may_hex, json_string};

/// Execute a complete signed transaction against one optimistic canonical head.
/// The transaction receives a private child chunk that is dropped on both paths,
/// so this endpoint cannot commit state, submit to the pool, or broadcast.
pub(crate) fn transaction_preflight_handler(ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let permit = match ctx
        .sandbox_limiter
        .acquire(req.peer_ip.map(|ip| ip.to_string()))
    {
        Ok(permit) => permit,
        Err(reason) => return retry_response(reason),
    };
    let max_size = ctx.engine.consensus().mint_params().max_tx_size;
    let raw_max = if crate::api::util::q_bool(&req, "hexbody", false) {
        max_size.saturating_mul(2)
    } else {
        max_size
    };
    if req.body.len() > raw_max {
        return api_error(&format!(
            "transaction body exceeds max encoded size {raw_max}"
        ));
    }
    let data = match body_data_may_hex(&req) {
        Ok(data) if !data.is_empty() => data,
        _ => return api_error("transaction body invalid"),
    };
    let pkg = match base::TxPkg::from_bytes(
        ctx.engine.services().as_ref(),
        data,
        base::PkgSource::new(base::PkgOrigin::Api),
    ) {
        Ok(pkg) => pkg,
        Err(error) => return api_error(&format!("transaction decode failed: {error}")),
    };
    let tx = pkg.tx_ref();
    if tx.is_block_prelude() {
        return api_error("a prelude transaction cannot be preflighted as a user transaction");
    }
    if base::tx_exceeds_max_size(pkg.size(), max_size) {
        return api_error(&format!("tx size {} exceeds max {}", pkg.size(), max_size));
    }
    if tx.action_count() != tx.actions().len() {
        return api_error("tx action count does not match its decoded actions");
    }
    if tx.timestamp().value() > sys::curtimes() {
        return api_error("transaction timestamp is in the future");
    }
    let snapshot = match ctx.engine.optimistic_canonical() {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return retry_response("node is busy; retry transaction preflight"),
        Err(error) => return api_state_read_error(&error),
    };
    let head_hash = snapshot.head_hash;
    let head_height = snapshot.head_height;
    let epoch = snapshot.epoch;
    let execution_height = head_height.saturating_add(1);
    let mut env = Env::default();
    env.chain.id = ctx.engine.consensus().chain_id();
    env.chain.consensus_flags = ctx.engine.consensus().chain_flags(execution_height);
    env.block.height = execution_height;
    env.block.author = ctx.engine.block_producer().external_exec_author();
    env.tx = TxInfo {
        ty: tx.ty(),
        main: tx.main(),
        addrs: tx.addrs(),
        fee: tx.fee().clone(),
    };
    let result = execute_signed_transaction(
        ctx.engine.services().clone(),
        snapshot.begin_tx(tx.hash()),
        env,
        tx.clone(),
        permit.deadline(),
    );

    // A concurrent head change invalidates both success and failure. In
    // particular, do not report a transaction failure against a stale state.
    finish_preflight_result(
        result,
        ctx.engine.validate_optimistic(epoch),
        head_hash,
        head_height,
        execution_height,
    )
}

fn finish_preflight_result(
    result: sys::Ret<()>,
    state_valid: bool,
    head_hash: field::Hash,
    head_height: u64,
    execution_height: u64,
) -> ApiResponse {
    if !state_valid {
        return retry_response("canonical state changed; retry transaction preflight");
    }
    let head_hash = head_hash.as_ref().to_hex();
    match result {
        Ok(()) => ApiResponse::json(format!(
            "{{\"ret\":0,\"ok\":true,\"head_hash\":{},\"head_height\":{},\"execution_height\":{}}}",
            json_string(&head_hash),
            head_height,
            execution_height
        )),
        Err(error) if error.is_abort() => api_state_read_error(&error),
        Err(error) => ApiResponse::json(format!(
            "{{\"ret\":1,\"ok\":false,\"head_hash\":{},\"head_height\":{},\"execution_height\":{},\"err\":{}}}",
            json_string(&head_hash),
            head_height,
            execution_height,
            json_string(&error.to_string())
        )),
    }
}

/// Execute a signed full transaction in a disposable state chunk using the
/// node's real execution services. This is exposed for the fullnode testkit so
/// the API path can be exercised with production services without a live node.
#[doc(hidden)]
pub fn execute_signed_transaction(
    services: Arc<dyn ExecutionServices>,
    mut chunk: StateChunkRef,
    env: Env,
    tx: TxRef,
    deadline: Instant,
) -> sys::Rerr {
    tx.verify_signature()?;
    check_deadline(deadline)?;
    let vm_params = services.vm_params()?;
    base::init_block_contract_storage_budget(&mut chunk, vm_params, env.block.height)?;
    let mut execution = services.create_context(env, chunk, tx.clone())?;
    if let Some(vm) = execution.vm_peek() {
        vm.set_deadline(Some(deadline));
    }
    check_deadline(deadline)?;
    tx.execute(execution.as_mut())?;
    check_deadline(deadline)?;
    // Dropping the context discards its chunk and every state write.
    drop(execution);
    Ok(())
}

fn check_deadline(deadline: Instant) -> sys::Rerr {
    if Instant::now() > deadline {
        Err(sys::Error::fault("sandbox_wall_clock_timeout"))
    } else {
        Ok(())
    }
}

fn retry_response(message: &str) -> ApiResponse {
    ApiResponse::json(format!(
        "{{\"ret\":2,\"retry\":true,\"err\":{}}}",
        json_string(message)
    ))
}

#[cfg(test)]
mod tests {
    use super::finish_preflight_result;
    use field::Hash;

    #[test]
    fn stale_state_masks_success_and_failure_with_retry_response() {
        for result in [Ok(()), Err(sys::Error::revert("transaction rejected"))] {
            let response = finish_preflight_result(result, false, Hash::default(), 7, 8);
            let body = String::from_utf8(response.body).unwrap();
            assert!(body.contains("\"ret\":2"), "{body}");
            assert!(body.contains("\"retry\":true"), "{body}");
            assert!(!body.contains("\"ok\":"), "{body}");
        }
    }
}
