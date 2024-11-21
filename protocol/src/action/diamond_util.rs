
/**
 * Diamond Constant
 */
// start with the 20001st diamond and enable the 32-bit MSG byte
pub const DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE: u32 = 2_0000;

// Starting from the 30001st diamond, destroy 90% of the bidding cost
pub const DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES: u32 = 3_0000;

// The average bidding cost of 30001 ~ 40000 diamonds is adopted, and the previous setting is 10 HAC
pub const DIAMOND_ABOVE_NUMBER_OF_STATISTICS_AVERAGE_BIDDING_BURNING: u32 = 4_0000;

// 40001 diamond, start with Sha3_ Hash (diamondreshash + blockhash) determines diamond shape and color matching
pub const DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BLOCK_HASH: u32 = 4_0000;

// 41001 diamond, start with Sha3_ Hash (diamondreshash + blockhash + bidfee) includes the bidding fee to participate in the decision of diamond shape color matching
pub const DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BIDDING_FEE: u32 = 4_1000;







/*************** util ***************/




const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";


/**
 * calculate diamond visual gene
*/
pub fn calculate_diamond_visual_gene(name: &[u8;6], life_gene: &[u8;32]) -> DiamondVisualGene {
    
    let mut genehexstr = [b'0'; 20];
    // step 1
    let searchgx = |x| {
        for (i, a) in x16rs::DIAMOND_HASH_BASE_CHARS.iter().enumerate() {
            if *a == x {
                return HEX_CHARS[i-1]
            }
        }
        panic!("not supply diamond char!!!")
    };

    for i in 0..6 {
        genehexstr[i+2] = searchgx( name[i] );
    }

    // step 2
    let mut idx = 8;
    for i in 20..31 {
        let k = (life_gene[i] as usize) % 16;
        genehexstr[idx] = HEX_CHARS[k];
        idx += 1;
    }
    // last bit of hash as shape selection
    let mut genehex = hex::decode(genehexstr).unwrap();
    genehex[0] = life_gene[31];
    
    // ok
    DiamondVisualGene::from(genehex.try_into().unwrap())
}

/**
 * calculate diamond visual gene
*/
pub fn calculate_diamond_gene(dianum: u32, diamhash: &[u8;32], diamondstr: &[u8;16], pedding_block_hash: &Hash, diabidfee: &Amount) -> (DiamondLifeGene, DiamondVisualGene) {
    
    
    // cacl vgenehash
    let mut vgenehash = diamhash.clone();
    if dianum > DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BLOCK_HASH {
        let mut vgenestuff = diamhash.to_vec();
        vgenestuff.append( &mut pedding_block_hash.to_vec() ); // add block hash
        if dianum > DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BIDDING_FEE {
            vgenestuff.append( &mut diabidfee.serialize() ); // add bidfee
        }
        vgenehash = x16rs::calculate_hash(vgenestuff);
    }

    let dianame = diamondstr[10..16].try_into().unwrap();
    // ok ret
    (
        DiamondLifeGene::from(vgenehash.try_into().unwrap()),
        calculate_diamond_visual_gene(&dianame, &vgenehash), 
    )
}


/**
 * calculate diamond average bid burn
 */
pub fn calculate_diamond_average_bid_burn(diamond_number: u32, hacd_burn_zhu: u64) -> Uint2 {

    // old
    if diamond_number <= DIAMOND_ABOVE_NUMBER_OF_STATISTICS_AVERAGE_BIDDING_BURNING {
        return Uint2::from(10)
    }

    // average
    let bsnum = diamond_number - DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES;
    let bidfee = hacd_burn_zhu / 1_0000_0000 / (bsnum as u64) + 1;
    // ok
    Uint2::from(bidfee as u16)
}

