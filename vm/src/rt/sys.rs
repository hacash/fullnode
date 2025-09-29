
macro_rules! std_mem_transmute  {
    ($v: expr) => { 
        unsafe { std::mem::transmute($v) }
    }
}

