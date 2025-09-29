


#[macro_export]
macro_rules! maybe {
    ($c:expr, $v1:expr, $v2:expr) => { 
        match $c { 
            true => $v1,
            false => $v2,
        }
    };
}



#[macro_export]
macro_rules! mayerr {
    ($c:expr, $e:expr) => { 
        match $c { 
            true => Ok(()),
            false => $e,
        }
    };
}


