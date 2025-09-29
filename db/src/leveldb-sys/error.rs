fn new_string_from_char_ptr(message: *const c_char) -> String {
    use std::str::from_utf8;
    use std::ffi::CStr;
    let err_string;
    unsafe {
        err_string = from_utf8(CStr::from_ptr(message).to_bytes()).unwrap().to_string();
        leveldb_free(message as *mut c_void);
    }
    err_string
}

