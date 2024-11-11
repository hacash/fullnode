pub type Ret<T> = Result<T, Error>;
pub type Rerr   = Result<(), Error>;


pub const _BUF_E1: &str = "buffer too short";


#[macro_export]
macro_rules! er {
    ($v:expr) => { Some(($v).to_string()) };
}

#[macro_export]
macro_rules! erf {
    ( $($v:expr),+ ) => { er!(format!( $($v),+ )) };
}

#[macro_export]
macro_rules! err {
    ($v:expr) => { Err(($v).to_string()) };
}

#[macro_export]
macro_rules! errf {
    ( $($v:expr),+ ) => { err!(format!( $($v),+ )) };
}

#[macro_export]
macro_rules! errunbox {
    ($errbox:expr) => {
        match $errbox {
            Ok(v) => Ok(v),
            Err(e) => Err(e.to_string()),
        }
    };
}

#[macro_export]
macro_rules! ifer {
    ( $value:expr ) => { 
// Some => Err
if let Some(e) = $value {
    return Err(e)
}    
    };
}

#[macro_export]
macro_rules! err_buf_short {
    () => { err!(_BUF_E1) };
}
