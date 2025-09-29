use tokio::time::*;

pub fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

pub async fn asleep(t: f32) {
    tokio::time::sleep(Duration::from_millis((t*1000.0) as u64)).await;
}

pub async fn new_ticker(dura: u64) -> Interval {
    new_ticker_at(dura, dura).await
}

pub async fn new_ticker_at(inst: u64, dura: u64) -> Interval {
    let mut intv = interval_at(
        Instant::now() + Duration::from_secs(inst),
        Duration::from_secs(dura),
    );
    intv.set_missed_tick_behavior(MissedTickBehavior::Delay);
    intv
}

pub fn new_tokio_rt(is_multi_thread: bool) -> tokio::runtime::Runtime {
    use tokio::runtime::Builder;
    match is_multi_thread {
        true => Builder::new_multi_thread(),
        false => Builder::new_current_thread(),
    }.enable_time().enable_io().build().unwrap()
}


pub fn new_current_thread_tokio_rt() -> tokio::runtime::Runtime {
    new_tokio_rt(false) // current_thread
}