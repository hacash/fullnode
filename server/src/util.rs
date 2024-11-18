
pub fn new_tokio_rt(multi_thread: bool) -> tokio::runtime::Runtime {
    let mut bdr;
    if multi_thread {
        bdr = tokio::runtime::Builder::new_multi_thread();
    }else{
        bdr = tokio::runtime::Builder::new_current_thread();
    }
    bdr.enable_time().enable_io()
    .build().unwrap()
}



