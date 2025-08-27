

impl HttpServer {

    fn do_start(&self, worker: Worker) {
        if !self.cnf.enable {
            worker.end();
            return // disable
        }
        let rt = new_tokio_rt(self.cnf.multi_thread);
        // server listen loop
        rt.block_on(async move {
            server_listen(self, worker).await
        });
    }


}


async fn server_listen(ser: &HttpServer, worker: Worker) {
    let port = ser.cnf.listen;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = TcpListener::bind(addr).await;
    if let Err(ref e) = listener {
        println!("\n[Error] Api Server bind port {} error: {}\n", port, e);
        worker.end();
        return
    }
    let listener = listener.unwrap();
    println!("[Http Api Server] Listening on http://{addr}");
    // 
    let app = api::routes(ApiCtx::new(
        ser.engine.clone(),
        ser.hcshnd.clone(),
    ));
    let mut wkr = worker.clone();
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = wkr.wait().await;
        })
        .await {
        println!("{e}");
    }
    println!("[Http Server] serve end.");
    worker.end();
}
