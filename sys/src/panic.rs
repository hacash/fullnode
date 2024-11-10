
#[macro_export]
macro_rules! panic_never_call_this {
    ()=>( panic!("never call this") )
}

#[macro_export]
macro_rules! must_have {
    ( $tip:expr, $value:expr ) => (
        match $value {
            None => return errf!("{} not find", $tip),
            Some(a) => a,
        }
    )
}


pub const HNERRSDEF: [&str; 8] = [
    "Hacash",
    "Config",
    "",
    "",
    "",
    "",
    "",
    "",
];

#[macro_export]
macro_rules! exiterr {
    ($ety: expr, $tip: expr, $( $ps: expr ),+)=>(
        &format!(
            "{}{}{}{}{}", "\n\n┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸┸\n[", HNERRSDEF[$ety], " Error] ", 
            format!($tip, $( $ps ),+),
            ".\n┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰┰\n\n\n", 
        )
    )
}
