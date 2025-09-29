
// Hacash node
pub trait Server: Send + Sync {

    fn start(&self, _: Worker) {}
    
}

