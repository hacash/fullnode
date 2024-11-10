

pub fn cover<'a, T: Copy>(dst: &'a mut Vec<T>, src: &'a [T]) -> &'a mut Vec<T> {
    let mut ln = dst.len();
    let l2 = src.len();
    if l2 < ln {
        ln = l2;
    }
    // copy
    for i in 0..ln {
        dst[i] = src[i];
    }
    dst
}

pub fn cover_clone<'a, T: Clone>(dst: &'a mut Vec<T>, src: &'a [T]) -> &'a mut Vec<T> {
    let mut ln = dst.len();
    let l2 = src.len();
    if l2 < ln {
        ln = l2;
    }
    // copy
    for i in 0..ln {
        dst[i] = src[i].clone();
    }
    dst
}