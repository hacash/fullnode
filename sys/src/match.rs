#[macro_export]
macro_rules! maybe {
    ($condition:expr, $when_true:expr, $when_false:expr) => {
        match $condition {
            true => $when_true,
            false => $when_false,
        }
    };
}
