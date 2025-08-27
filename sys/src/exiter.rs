use std::{sync::*, thread::sleep};

use async_broadcast::{broadcast, Sender, Receiver, Recv, TryRecvError};

#[derive(Clone)]
pub struct Worker {
    jobs: Arc<Mutex<isize>>,
    receiver: Receiver<()>,
}

impl Worker {

    pub fn fork(&self) -> Self {
        let mut jobs = self.jobs.lock().unwrap();
        *jobs += 1;
        Self {
            jobs: self.jobs.clone(),
            receiver: self.receiver.clone(),
        }
    }

    pub fn end(&self) {
        let mut jobs = self.jobs.lock().unwrap();
        *jobs -= 1;
    }

    pub fn wait(&mut self) -> Recv<'_, ()> {
        self.receiver.recv_direct()
    }

    pub fn quit(&mut self) -> bool {
        match self.receiver.try_recv() {
            Err(TryRecvError::Empty) => false,
            _ => {
                self.end();
                true
            }
        }
    }

}



#[allow(dead_code)]
#[derive(Clone)]
pub struct Exiter {
    jobs: Arc<Mutex<isize>>,
    sender: Sender<()>,
    receiver: Receiver<()>,
}


impl Exiter {

    pub fn new() -> Self {
        let (s, r) = broadcast::<()>(5);
        Self {
            jobs: Arc::default(),
            sender: s,
            receiver: r,
        }
    }

    pub fn exit(&self) {
        let _ = self.sender.broadcast_blocking(());
    }

    pub fn work(&self) -> Worker {
        let mut jobs = self.jobs.lock().unwrap();
        *jobs += 1;
        Worker {
            jobs: self.jobs.clone(),
            receiver: self.receiver.clone()
        }
    }
    
    
    pub fn wait(self) {
        loop {
            sleep(Duration::from_millis(333));
            let j = self.jobs.lock().unwrap();
            // println!("Exiter::wait, jobs={}", *j);
            if *j <= 0 {
                break; // exit all
            }
        }
    }


}

