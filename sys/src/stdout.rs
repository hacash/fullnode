



/// Prints to the standard ouput only in debug build.  
/// In release build this macro is not compiled thanks to `#[cfg(debug_assertions)]`.  
/// see [https://doc.rust-lang.org/std/macro.print.html](https://doc.rust-lang.org/std/macro.print.html) for more info.
#[macro_export]
macro_rules! debug_print {
    ($($arg:tt)*) => (#[cfg(debug_assertions)] print!($($arg)*));
}

/// Prints to the standard ouput only in debug build.  
/// In release build this macro is not compiled thanks to `#[cfg(debug_assertions)]`.  
/// see [https://doc.rust-lang.org/std/macro.println.html](https://doc.rust-lang.org/std/macro.println.html) for more info.
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => (#[cfg(debug_assertions)] println!($($arg)*));
}

/// Prints to the standard error only in debug build.  
/// In release build this macro is not compiled thanks to `#[cfg(debug_assertions)]`.  
/// see [https://doc.rust-lang.org/std/macro.eprint.html](https://doc.rust-lang.org/std/macro.eprint.html) for more info.
#[macro_export]
macro_rules! debug_eprint {
    ($($arg:tt)*) => (#[cfg(debug_assertions)] eprint!($($arg)*));
}

/// Prints to the standard error only in debug build.  
/// In release build this macro is not compiled thanks to `#[cfg(debug_assertions)]`.  
/// see [https://doc.rust-lang.org/std/macro.eprintln.html](https://doc.rust-lang.org/std/macro.eprintln.html) for more info.
#[macro_export]
macro_rules! debug_eprintln {
    ($($arg:tt)*) => (#[cfg(debug_assertions)] eprintln!($($arg)*));
}



#[macro_export]
macro_rules! flush {(
        $($param: expr),+
    )=>(
    {
        use std::io::Write;
        print!( $( $param ),+ );
        let _ = std::io::stdout().flush();
    }
)}

