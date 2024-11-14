
macro_rules! not_find_action_kind_error {
    ($t:expr) => {
        Err(format!("action kind '{}' not find", $t).to_owned())
    };
}



