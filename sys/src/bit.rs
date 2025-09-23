

#[macro_export]
macro_rules! bit4l {
    ($n: expr) => {
        { $n >> 4 }
    }
}


#[macro_export]
macro_rules! bit4r {
    ($n: expr) => {
        { $n & 0b00001111 }
    }
}