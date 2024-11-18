
/*
#[async_trait]
impl Handler<Request> for RPCServer {
    type Output = Result<Response>;
    async fn call(&self, req: Request) -> Self::Output {
        let path = req.path();
        let method = req.method().clone();
        // let code = self.code.fetch_add(1, Ordering::SeqCst);
        Ok(format!("method = {}, path = {}", method, path).into_response())
    }
}

*/