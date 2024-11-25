

impl HttpServer {

    #[allow(dead_code)]
    fn route_api(&self, _app: &mut Router, _pathkind: &str) {

        // app.get(pathkind, query);
    }

    /*
    fn app_router(api: Arc<HttpServer>) -> Router {
        let app = Router::new();
        
        // stable api
        // app.get("/", console);

        let ctx = api.clone();
        app.clone().route("/query", Route::new().get(|req| async move { 
            api::balance(ctx.as_ref(), req).await
        }));

        // self.route_api(&mut app, "/query",);


        // unstable api

        // ok
        app.clone()
    }
    */


}