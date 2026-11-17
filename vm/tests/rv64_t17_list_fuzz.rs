//! T17 — a deterministic pseudo-random corpus over the general list-op
//! faces. Each generated body mixes creation, alias/CLONE binds, APPEND /
//! INSERT / REMOVE / MERGE / TAKEFIRST / TAKELAST / CLEAR / ITEMSET across
//! one element width (u8..u64 in pass 1, u128 in pass 2) and is compared
//! on every input against the interpreter in all three rv64 modes. A body
//! the kernel rejects (B-family) is skipped; an accepted body must agree
//! byte for byte (or fail with the identical code).

#[path = "c3lib/mod.rs"]
mod c3lib;

use c3lib::{Interp, Rv64};

fn interp_obs(it: &Interp, input: &[u8]) -> Result<Vec<u8>, u64> {
    it.run(input).map_err(|f| f.code as u64)
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 33) & 0xFFFF
    }
    fn pick(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn t17_fuzz_list_ops() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let mut built = 0usize;
    let mut checked = 0usize;
    for _case in 0..600 {
        let mut body = String::new();
        // two lists with small literal contents; the element width varies
        // (both lists share one width so MERGE stays inside the subset)
        let widths = ["u8", "u16", "u32", "u64"];
        let w = widths[rng.pick(4) as usize];
        let n = 1 + rng.pick(4) as usize;
        body.push_str("var a = [");
        for i in 0..n {
            if i > 0 {
                body.push_str(", ");
            }
            body.push_str(&format!("{}{}", rng.pick(100), w));
        }
        body.push_str("]\n");
        let m = rng.pick(4) as usize;
        body.push_str("var b = [");
        for i in 0..m {
            if i > 0 {
                body.push_str(", ");
            }
            body.push_str(&format!("{}{}", rng.pick(100), w));
        }
        body.push_str("]\n");
        let alias = rng.pick(3) == 0;
        if alias {
            body.push_str("var c = a\n");
        }
        // op sequence
        for _ in 0..(1 + rng.pick(4)) {
            match rng.pick(10) {
                0 => body.push_str(&format!("append(a, {}{})\n", rng.pick(100), w)),
                1 => body.push_str(&format!("insert(a, {}u64, {}{})\n", rng.pick(6), rng.pick(100), w)),
                2 => body.push_str(&format!("remove(a, {}u64)\n", rng.pick(6))),
                3 => body.push_str("merge(a, b)\n"),
                4 => body.push_str("var t = (take_first(a) as u64)\n"),
                5 => body.push_str("var t = (take_last(a) as u64)\n"),
                6 => body.push_str("clear(b)\n"),
                7 => body.push_str(&format!("a[{}u64] = {}{}\n", rng.pick(4), rng.pick(100), w)),
                8 => body.push_str("var t = length(a) as u64\n"),
                _ => body.push_str("var t = length(b) as u64\n"),
            }
        }
        // aggregate reads
        body.push_str("var i = (byte(data, 0) as u64) % 5u64\n");
        body.push_str("return ((((length(a) as u64) * 16u64 + length(b) as u64) * 16u64 + (i % 5u64))) as bytes");

        let interp = match Interp::try_build(&body) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let rv = match Rv64::try_build(&body) {
            Ok(x) => x,
            Err(_) => continue, // B-family compile reject: allowed
        };
        built += 1;
        for input in [vec![0u8], vec![3u8], vec![7u8, 1]] {
            let want = interp_obs(&interp, &input);
            for (mode, got) in rv.run(&input) {
                assert_eq!(
                    got, want,
                    "case {_case} mode {mode:?} input {input:?} diverged:\n{body}"
                );
            }
            checked += 1;
        }
    }
    // second pass: u128 elements (the (lo, hi) pair path)
    for _case in 0..250 {
        let mut body = String::new();
        let n = 1 + rng.pick(3) as usize;
        body.push_str("var a = [");
        for i in 0..n {
            if i > 0 {
                body.push_str(", ");
            }
            body.push_str(&format!(
                "{}u128",
                (rng.pick(0xFFFF) as u128) << 64 | rng.pick(0xFFFF) as u128
            ));
        }
        body.push_str("]\n");
        body.push_str("var b = [1u128, 2u128]\n");
        if rng.pick(2) == 0 {
            body.push_str("var c = a\n");
        }
        for _ in 0..(1 + rng.pick(3)) {
            match rng.pick(7) {
                0 => body.push_str("append(a, 340282366920938463463374607431768211455u128)\n"),
                1 => body.push_str(&format!("insert(a, {}u64, 7u128)\n", rng.pick(5))),
                2 => body.push_str(&format!("remove(a, {}u64)\n", rng.pick(5))),
                3 => body.push_str("merge(a, b)\n"),
                4 => body.push_str("var t = (take_first(a) as u128)\n"),
                5 => body.push_str("var t = (take_last(a) as u128)\n"),
                _ => body.push_str("var t = length(a) as u64\n"),
            }
        }
        body.push_str("return (length(a) as u64) as bytes");
        let interp = match Interp::try_build(&body) {
            Ok(x) => x,
            Err(_) => continue,
        };
        let rv = match Rv64::try_build(&body) {
            Ok(x) => x,
            Err(_) => continue,
        };
        built += 1;
        for input in [vec![0u8], vec![7u8]] {
            let want = interp_obs(&interp, &input);
            for (mode, got) in rv.run(&input) {
                assert_eq!(got, want, "u128 case {_case} mode {mode:?}:\n{body}");
            }
            checked += 1;
        }
    }
    println!("fuzz: {built} bodies built, {checked} comparisons");
    assert!(built > 100, "too few fuzz bodies built ({built})");
}
