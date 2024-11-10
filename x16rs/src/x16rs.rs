
#[link(name = "x16rs", kind = "static")]
extern "C" {
    fn c_x16rs_hash(a: i32, b: *const u8, c: *const u8) -> ();
}

/*
*
*/
pub fn x16rs_hash(loopnum: i32, indata: &[u8; H32S]) -> [u8; H32S] {

    let outdata = [0u8; H32S];
    
    unsafe {
        // input hash
        let input: *const u8 = indata.as_ptr();

        // output hash
        let output: *const u8 = outdata.as_ptr();

        // do call
        c_x16rs_hash(loopnum, input, output);
        // println!("{:?}", outdata);
    }

    return outdata;
}
