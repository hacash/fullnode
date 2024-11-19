

impl HttpServer {

    #[allow(dead_code)]
    fn route_rpc(&self, _app: &mut Router, _pathkind: &str) {

        // app.get(pathkind, query);
    }

    /*
    fn app_router(rpc: Arc<HttpServer>) -> Router {
        let app = Router::new();
        
        // stable rpc
        // app.get("/", console);

        let ctx = rpc.clone();
        app.clone().route("/query", Route::new().get(|req| async move { 
            rpc::balance(ctx.as_ref(), req).await
        }));

        // self.route_rpc(&mut app, "/query",);


        // unstable api

        // ok
        app.clone()
    }
    */


}