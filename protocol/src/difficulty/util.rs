const HXS: usize = 32;
const BITS: usize = HXS * 8;

/******************/


pub fn biguint_to_u32(bn: &BigUint) -> u32 {
    let hx = biguint_to_hash(bn);
    hash_to_u32(&hx)
}

pub fn u32_to_biguint(num: u32) -> BigUint {
    let hx = u32_to_hash(num);
    hash_to_biguint(&hx)
}

pub fn biguint_to_hash(bn: &BigUint) -> [u8; HXS] {
    let res = bn.to_bytes_be();
    if res.len() > HXS {
        return [255; HXS] // max
    }
    vec![
        vec![0u8; HXS-res.len()],
        res,
    ].concat().try_into().unwrap()
}

pub fn hash_to_biguint(hx: &[u8; HXS]) -> BigUint {
    BigUint::from_bytes_be(&hx[..])
}

pub fn u32_to_hash(num: u32) -> [u8; HXS] {
    let numbts = num.to_be_bytes();
    let mut bits = Vec::with_capacity(BITS);
    bits.append( &mut vec![0u8; 255 - numbts[0] as usize] );
    let mut bits2 = vec![
        byte_to_bits(numbts[1]).to_vec(),
        byte_to_bits(numbts[2]).to_vec(),
        byte_to_bits(numbts[3]).to_vec(),
    ].concat();
    bits.append( &mut bits2 );
    bits.append( &mut vec![0u8; 256-bits.len()] );
    // ok
    bits_to_bytes(bits.as_slice().try_into().unwrap())
}

pub fn hash_to_u32(hx: &[u8; HXS]) -> u32 {
    let mut bits = bytes_to_bits(hx).to_vec();
    let lzero = left_zero(&bits);
    bits.append(&mut vec![1u8; lzero]);
    let reshx = bits_to_bytes(&bits[lzero..].try_into().unwrap());
    let mut u32bts = [0u8; 4];
    u32bts[0] = 255 - lzero as u8;
    u32bts[1] = reshx[0];
    u32bts[2] = reshx[1];
    u32bts[3] = reshx[2];
    // ok
    u32::from_be_bytes(u32bts)
}

/******************/

pub fn hash_big_than(src: &[u8], tar: &[u8]) -> bool {
    let mut sz = src.len();
    if sz > tar.len() {
        sz = tar.len();
    }
    for i in 0..sz {
        if src[i] > tar[i] {
            return true
        }else if src[i] < tar[i] {
            return false
        }
    }
    // equal
    false

}

fn left_zero(buf: &[u8]) -> usize {
    let mut lzo = 0usize;
    for a in buf {
        if *a > 0 {
            return lzo
        }
        lzo += 1;
    }
    lzo
}

// 256 to 32
fn bits_to_bytes(bits: &[u8; BITS]) -> [u8; HXS] {
    let mut res = Vec::with_capacity(HXS);
    for i in 0..HXS {
        let x = i * 8;
        res.push( bits_to_byte(bits[x..x+8].try_into().unwrap()) );
    }
    res.as_slice().try_into().unwrap()
}

// 32 to 256
fn bytes_to_bits(bytes: &[u8; HXS]) -> [u8; BITS] {
    let mut res = Vec::with_capacity(BITS);
    for b in bytes {
        res.append( &mut byte_to_bits(*b).to_vec() );
    }
    res.as_slice().try_into().unwrap()
}

// 8 to 1
fn bits_to_byte(bits: [u8; 8]) -> u8 {
	let mut b = 0 as u8;
	b += 1 * bits[7];
	b += 2 * bits[6];
	b += 4 * bits[5];
	b += 8 * bits[4];
	b += 16 * bits[3];
	b += 32 * bits[2];
	b += 64 * bits[1];
	b += 128 * bits[0];
	return b
}

// 1 to 8
fn byte_to_bits(b: u8) -> [u8; 8] {
	return [
		(b >> 7) & 0x1,
		(b >> 6) & 0x1,
		(b >> 5) & 0x1,
		(b >> 4) & 0x1,
		(b >> 3) & 0x1,
		(b >> 2) & 0x1,
		(b >> 1) & 0x1,
		(b >> 0) & 0x1,
    ]
}






