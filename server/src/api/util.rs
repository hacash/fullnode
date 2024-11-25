

fn right_00_to_ff(hx: &mut [u8]) {
    let m = hx.len();
    for i in 0..hx.len() {
        let n = m - i - 1;
        if hx[n] == 0 { // 00
            hx[n] = 255; // ff
        }else{
            break // finish
        }
    }
}

