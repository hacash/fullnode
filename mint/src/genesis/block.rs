

use std::sync::LazyLock;
static GENESIS_BLOCK: LazyLock<BlockV1> = LazyLock::new(|| create_genesis_block() );
static GENESIS_BLOCK_PKG: LazyLock<Arc<BlockPkg>> 
    = LazyLock::new(||Arc::new(BlockPkg::create(Box::new(create_genesis_block().clone()))));

pub fn genesis_block() -> &'static BlockV1 {
    &GENESIS_BLOCK
}

pub fn genesis_block_pkg() -> BlockPkg  {
    GENESIS_BLOCK_PKG.as_ref().clone()
}

pub fn genesis_block_ptr() -> Arc<BlockPkg>  {
    GENESIS_BLOCK_PKG.clone()
}

/**
 * create
 */ 
fn create_genesis_block() -> BlockV1 {
    let blktime = Timestamp::from(1549250700);
    let blknoncenum = Uint4::from(160117829);
    let reward_addr = Address::from_readable(&"1271438866CSDpJUqrnchoJAiGGBFSQhjd".to_string()).unwrap();
    let mut trsvec = DynVecTransaction::default(); 
    trsvec.push(Box::new(TransactionCoinbase{
        ty: Uint1::from(0),
        address: reward_addr,
        reward: Amount::small_mei(1),
        message: Fixed16::from_readable(b"hardertodobetter").unwrap(),
        extend: CoinbaseExtend::default()
    })).unwrap();
    let mut genesis_block = BlockV1 {
        intro: BlockHeadMeta { 
            head: BlockHead {
                version: Uint1::from(1), 
                height: BlockHeight::from(0), 
                timestamp: blktime, 
                prevhash: Hash::default(), // 000000...
                mrklroot: Hash::default(), // 000000...
                transaction_count: Uint4::from(1) // trs 1
            }, 
            meta: BlockMeta { 
                nonce: blknoncenum, 
                difficulty: Uint4::from(0), 
                witness_stage: Fixed2::default() 
            },
        },
        transactions: trsvec
    };
    // set mrklroot
    genesis_block.update_mrklroot();
    /*
    //ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b01
    //00000c1aaa4e6007cc58cfb932052ac0ec25ca356183f80101686172646572746f646f62657474657200
    //01 0000000000 005c57b08c 0000000000000000000000000000000000000000000000000000000000000000 ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b0100000001098b344500000000000000000 c1aaa4e6007cc58cfb932052ac0ec25ca356183f80101686172646572746f646f62657474657200
    println!("{}", genesis_block.mrklroot());
    println!("{}", hex::encode(genesis_block.transactions[0].serialize()));
    println!("{}", hex::encode(genesis_block.serialize()));
    */
    // check
    let blkhx = genesis_block.hash();
    let blkbd = genesis_block.serialize();
    let checkhx = Hash::from_hex(b"000000077790ba2fcdeaef4a4299d9b667135bac577ce204dee8388f1b97f7e6").unwrap();
    let checkbd = hex::decode(b"010000000000005c57b08c0000000000000000000000000000000000000000000000000000000000000000ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b0100000001098b344500000000000000000c1aaa4e6007cc58cfb932052ac0ec25ca356183f80101686172646572746f646f62657474657200").unwrap();
    if blkhx != checkhx {
        panic!("{}", format!("Genesis Block Hash Error: need {} but got {}", checkhx, blkhx))
    }    
    if blkbd != checkbd {
        panic!("{}", format!("Genesis Block Body Error: need {} but got {}", hex::encode(checkbd), hex::encode(blkbd)))
    }
    // println!("{}", hex::encode(genesis_block.serialize()));
    // check ok 
    genesis_block
}

