

pub trait PeerManage: Send + Sync  { // + dyn_clone::DynClone
    fn switch_peer(&self, _: Arc<Peer>) -> Arc<Peer>;
    fn broadcast_message(&self, delay: u64, key: KnowKey, ty: u16, body: Vec<u8>);
}


// dyn_clone::clone_trait_object!(PeerManage);


