

impl HttpServer {

    pub fn start(&self) {
        if !self.cnf.enable {
            return // disable
        }
        let rt = new_tokio_rt(self.cnf.multi_thread);
        // server listen loop
        rt.block_on(async move {
            server_listen(self).await
        });
    }


}


async fn server_listen(ser: &HttpServer) {
    let port = ser.cnf.listen;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await;
    if let Err(ref e) = listener {
        println!("\n[Error] RPC Server bind port {} error: {}\n", port, e);
        return
    }
    let listener = listener.unwrap();
    println!("[RPC Server] Listening on http://{addr}");
    // 
    let app = rpc::routes(ApiCtx::new(
        ser.engine.clone(),
        ser.hcshnd.clone(),
    ));
    if let Err(e) = axum::serve(listener, app).await {
        println!("{e}");
    }
}
