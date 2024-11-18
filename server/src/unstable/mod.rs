
use axum::{
    extract::{Query, State}, 
    response::IntoResponse,
    routing::get,
    Router,
};
use serde_json::json;


use super::ctx::*;


include!("test.rs");


pub fn routes() -> Router<ApiCtx> {

    Router::new().route(&query("testapi1234563847653475"), get(testapi1234563847653475))

}






