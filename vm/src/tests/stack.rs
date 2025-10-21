
#[allow(unused)]
pub fn stack_test_1() {
    let _codes1 = build_codes!{
        P0 POP JMPL 0 1 END
    };
    let _codes2 = build_codes!{
        P0 P1 SWAP END
    };
    let _codes3 = build_codes!{
        P0 P1 PU8 2 REV 3 END
    };
    let _codes4 = build_codes!{
        P1 PU8 2 P0 CHOISE END
    };
    let _codes5 = build_codes!{
        P0 DUP DUP DUP POP POP POP POP END
    };
    let _codes6 = build_codes!{
        P0 DUP DUP DUP POPN 4 END
    };
    let _codes7 = build_codes!{
        P0 P1 PU8 2 CAT CAT END
    };
    let _codes8 = build_codes!{
        P0 P1 PU8 2 PU8 3 JOIN 4 END
    };
    let _codes9 = build_codes!{
        PBUF 4 0 1 2 3 P1 PU8 3 CUT END
    };
    let _codes10 = build_codes!{
        PBUF 4 0 1 2 3 PU8 3 BYTE END
    };
    let _codes11 = build_codes!{
        NEWLIST P0 APPEND P1 APPEND P2 APPEND NEWLIST P3 APPEND P1 PU8 1 INSERT PU8 4 APPEND MERGE DUP 
        CLONE P1 APPEND PU8 5 REMOVE END
    };
    let _codes12 = build_codes!{
        ALLOC 2 P0 PUT 0 P1 PUT 1 NEWMAP GET1 GET0 INSERT GET0 P1 ADD PUT 0 JMPS 246  END
    };
    let _codes13 = build_codes!{
        P0 P1 P2 P3 PU8 4 PACKMAP VALUES END
    };
    let _codes14 = lang_to_bytecode(r##"
        var num = 0
        while num < 12 {
            heap_grow(1)
            num += 1
        }
        return num
    "##).unwrap();
    let _codes15 = build_codes!{
        HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 HGROW 1 END
    };
    let _codes16 = build_codes!{
        HGROW 10 END
    };
    let _codes17 = build_codes!{
        HGROW 2 PBUF 4 6 7 8 9 PU16 0 254 HWRITE 
        PU8 255 PU16 0 2 HREAD CU16
        END
    };
    let _codes18 = build_codes!{
        HGROW 1 PBUF 8 0 1 2 3 4 5 6 7 HWRITEX 0
        HREADUL 0b00100000 0b00000001 END
    };
    let _codes = build_codes!{
        P2 PNIL NOT END
    };


    let res = execute_test_maincall(65535, _codes);
    debug_println!("res: {:?}", res)
}

/*


*/