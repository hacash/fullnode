



fn mrkl_merge(list: &Vec<Hash>) -> Vec<Hash> {
    let num = list.len();
    let mut res = vec![];
    let mut x = 0usize;
    loop {
        let lh = list[x].to_vec();
        let rh = match x+1 < num {
            true => list[x+1].to_vec(),
            false => lh.clone(),
        };
        let hx = sys::calculate_hash(vec![lh, rh].concat());
        res.push(Hash::must(&hx));
        x += 2;
        if x >= num {
            break
        }
    }
    res
}


/*
* 
*/
pub fn calculate_mrklroot(list: &Vec<Hash>) -> Hash {
    let mut reslist = list;
    let mut tmp: Vec<Hash>;
    loop {
        // println!("mrklroot len={}", list.len());
        if reslist.len() <= 1 {
            return reslist[0].clone()
        }
        tmp = mrkl_merge(&reslist);
        reslist = &tmp;
    }
}





/*
* 
*/
pub fn calculate_mrkl_coinbase_modify(list: &Vec<Hash>) -> Vec<Hash> {
    let mut res = vec![];
    let hxl = list.len();
    if hxl == 0 {
        never!()
    }
    if hxl == 1 {
        return res
    }
    if hxl == 2 {
        res.push(list[1]);
        return res
    }

    let mut reslist = list;
    let mut tmp: Vec<Hash>;
    loop {
        // println!("mrklroot len={}", list.len());
        if reslist.len() == 1 {
            break
        }
        if reslist.len() >= 2 {
            res.push(reslist[1])
        }
        tmp = mrkl_merge(&reslist);
        reslist = &tmp;
    }
    res
}


/*
* return: newmrkl_
*/
pub fn calculate_mrkl_coinbase_update(cbhx: Hash, list: &Vec<Hash>) -> Hash {
    let mut reshx = cbhx;
    for h in list {
        reshx = Hash::from(sys::calculate_hash(vec![reshx.to_vec(), h.to_vec()].concat()));
    }
    reshx
}