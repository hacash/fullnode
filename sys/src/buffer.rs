
#[macro_export]
macro_rules! bufcut {
    ( $buf:expr, $l:expr, $r:expr ) => { 
        $buf[$l..$r].try_into().unwrap()
    };
}

