use std::sync::Arc;
use tokio::sync::broadcast::{self, Receiver, Sender};


#[allow(dead_code)]
#[derive(Clone)]
pub struct Exiter {
    closech: Arc<Receiver<bool>>,
    closechtx: Sender<bool>,
}


impl Exiter {

    pub fn new() -> Self {
        let (closetx, closerx) = broadcast::channel(4);
        Self {
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

    pub fn exit(&self) {
        let _ = self.closechtx.send(true);
    }

}