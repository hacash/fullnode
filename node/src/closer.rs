use std::sync::Arc;
use tokio::sync::broadcast::{self, Receiver, Sender};


#[allow(dead_code)]
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
        let _ = self.closechtx.send(true);
    }

}