use x16rs::*;


#[test]
fn t_shas() {

    // sha2
    let cres = hex::encode(sha2("123456"));
    assert_eq!(cres, "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92");
    
    // sha3
    let cres = hex::encode(sha3("123456"));
    assert_eq!(cres, "d7190eb194ff9494625514b6d178c87f99c5973e28c398969d2233f2960a573e");

    /*
    genesis block head meta: 
    01
    0000000000
    005c57b08c
    0000000000000000000000000000000000000000000000000000000000000000
    ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b01
    00000001
    098b3445
    00000000
    0000
    */
    // block_hash
    let hdts = hex::decode("010000000000005c57b08c0000000000000000000000000000000000000000000000000000000000000000ad557702fc70afaf70a855e7b8a4400159643cb5a7fc8a89ba2bce6f818a9b0100000001098b3445000000000000").unwrap();
    let cres = hex::encode(block_hash(1, hdts));
    assert_eq!(cres, "000000077790ba2fcdeaef4a4299d9b667135bac577ce204dee8388f1b97f7e6");

}

