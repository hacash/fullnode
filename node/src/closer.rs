use std::thread;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast::{self, Receiver, Sender};


#[derive(Clone)]
pub struct Closer {
    closech: Arc<Receiver<bool>>,
    closechtx: Sender<bool>,
}


impl Closer {

    pub fn new() -> Closer {
        let (closetx, closerx) = broadcast::channel(4);
        Closer{
            closech: closerx.into(),
            closechtx: closetx,
        }
    }

    pub fn sender(&self) -> Sender<bool> {
        self.closechtx.clone()
    }

    pub fn signal(&self) -> Receiver<bool> {
        self.closechtx.subscribe()
    }

    pub fn close(&self) {
        self.closechtx.send(true);
    }

}