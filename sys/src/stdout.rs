


#[macro_export]
macro_rules! flush {(
        $($param: expr),+
    )=>(
    {
        use std::io::Write;
        print!( $( $param ),+ );
        std::io::stdout().flush();
    }
)}

