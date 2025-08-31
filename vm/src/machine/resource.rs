

#[derive(Default)]
pub struct Resoure {
    height: u64,
    pub gas_table: GasTable,
    pub gas_extra: GasExtra,
    pub space_cap: SpaceCap,
    pub global_vals: GKVMap,
    pub memory_vals: CtcKVMap,
    pub contracts: HashMap<ContractAddress, Arc<ContractObj>>,
    // stack heap
    pub stack_pool: Vec<Stack>,
    pub heap_pool: Vec<Heap>,
}


impl Resoure {

    pub fn create(height: u64) -> Self {
        let cap = SpaceCap::new(height);
        Self {
            height,
            global_vals: GKVMap::new(cap.max_global),
            space_cap: cap,
            gas_extra: GasExtra::new(height),
            gas_table: GasTable::new(height),
            ..Default::default()
        }
    }

    pub fn reset(&mut self, height: u64) {
        self.global_vals.clear();
        self.memory_vals.clear();
        self.contracts.clear();
        if self.height == height {
            return
        }
        let cap = SpaceCap::new(height);
        self.height = height;
        self.global_vals.reset(cap.max_global);
        self.space_cap = cap;
        self.gas_extra = GasExtra::new(height);
        self.gas_table = GasTable::new(height);
    }

    pub fn stack_allocat(&mut self) -> Stack {
        self.stack_pool.pop().unwrap_or(Stack::default())
    }
    
    pub fn stack_reclaim(&mut self, stk: Stack) {
        self.stack_pool.push(stk);
    }

    pub fn heap_allocat(&mut self) -> Heap {
        self.heap_pool.pop().unwrap_or(Heap::default())
    }
    
    pub fn heap_reclaim(&mut self, heap: Heap) {
        self.heap_pool.push(heap);
    }




}

