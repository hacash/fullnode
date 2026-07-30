//! Account create / transfer-build API service.

use base::{ApiExecCtx, ApiRequest, ApiResponse, ApiRoute, ApiService};

use super::util::{api_json_error, hex_bytes, json_string};

fn account_create_handler(_ctx: &ApiExecCtx, req: ApiRequest) -> ApiResponse {
    let quantity = req.query_u64("quantity").unwrap_or(1);
    if quantity == 0 {
        return api_json_error("invalid quantity");
    }
    if quantity > 200 {
        return api_json_error("quantity must not exceed 200");
    }
    let mut list = Vec::with_capacity(quantity as usize);
    for _ in 0..quantity {
        let acc = match sys::Account::create_randomly(&|data| {
            getrandom::fill(data).map_err(|e| sys::Error::fault(e.to_string()))
        }) {
            Ok(v) => v,
            Err(e) => return api_json_error(&e.to_string()),
        };
        list.push(format!(
            "{{\"address\":{},\"prikey\":{},\"pubkey\":{}}}",
            json_string(acc.readable()),
            json_string(&hex_bytes(&acc.secret_key().serialize())),
            json_string(&hex_bytes(&acc.public_key().serialize_compressed())),
        ));
    }
    ApiResponse::json(format!("{{\"ret\":0,\"list\":[{}]}}", list.join(",")))
}

pub struct AccountApi;
impl ApiService for AccountApi {
    fn name(&self) -> &str {
        "account"
    }
    fn routes(&self) -> Vec<ApiRoute> {
        vec![ApiRoute::get("/create/account", account_create_handler)]
    }
}
