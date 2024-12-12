


#[macro_export]
macro_rules! maybe {
    ($c:expr, $v1:expr, $v2:expr) => { 
        match $c { 
            true => $v1,
            false => $v2,
        }
    };
}


