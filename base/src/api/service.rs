use sys::{Rerr, Waiter};

use crate::api::ApiRoute;

pub trait ApiService: Send + Sync {
    fn name(&self) -> &str {
        "api-service"
    }
    fn routes(&self) -> Vec<ApiRoute>;
}

pub trait Server: Send + Sync {
    fn start(&self, waiter: Waiter) -> Rerr;
    fn stop(&self) {}
}
