use axum::Router;

use super::ctx::*;



/*
    extend hook
*/
pub type FnExtendApiRoutesFunc = fn() -> Router<ApiCtx>;

pub static mut EXTEND_API_ROUTES_FUNC: FnExtendApiRoutesFunc = ||Router::new();

pub fn setup_extend_api_routes(f: FnExtendApiRoutesFunc) {
    unsafe {
        EXTEND_API_ROUTES_FUNC = f;
    }
}



/*
    routes
*/
pub fn routes() -> Router<ApiCtx> {
    unsafe {
        EXTEND_API_ROUTES_FUNC()
    }
}


