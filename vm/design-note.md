Comparison Reference:

    1. Move VM
    2. Ethereum VM
    3. Solana VM
    4. Ton VM
    5. CKB VM
    6. EOS VM
    7. NEO VM


Call Entry:

    1. Main
    2. Abst

   
Contract Func Type:

    1. Abst
    2. User


Runtime Space:

    1. Oprand stack  (Func)
    2. Local stack   (Func)
    3. Heap          (Func)
    4. Memory        (Contract temp)
    5. Global        (Public temp)
    6. Storage       (Contract state)


Contract Deploy & Store:

    1. contract max size is 65535 byte = 64kb
    2. function max size is 65535 byte = 64kb
    3. deploy or update contract burn 90 fee


Contract Verify:

    1. irnode compile
    3. bytecode finish with END|RET inst
    3. bytecode inst valid
    3. bytecode param check
    4. bytecode jump dest


Contract Code Store Rent:

    1. 50x tx fee


Contract KV State Rent:

    1. 


Storage Ban:

    - Main
    - Static Call
    - Library Call (can read)



Gas Calculation:

    - 1 gas = 1 byte
    - gas price = fee purity = txfeegot / txsize
    - 1 gcu = 32 gas / 32 byte
    - gas limit is 65535 for one tx
    - a machine execution charges at least 1 gcu of gas (32 gas) = gas / GSCU + 1
    - load a contract for call cost 2 * gcu = 64gas
    - call main cost gas at least 1 * gcu =  32gas
    - call abst cost gas at least 4 * gcu = 128gas


Call Kind:

    - DynCall                    (addr, fnsig, argv)
    - Call        <libidx, fnsig>(argv)
    - InrCall             <fnsig>(argv)
    - LibCall     <libidx, fnsig>(argv)
    - StaticCall  <libidx, fnsig>(argv)
    - CodeCopyCall<libidx, fnsig>(argv)


Call Privileges:

    - Main           => Outer, OuterDyn, Static, Code
    - Abst           => Inner, Lib, Static, Code
    - Library        => Lib, Static, Code
    - Static         => Static, Code
    - Code           => ()
    - Outer | Inner  => Outer, OuterDyn, Inner, Lib, Static, Code (support all types)


Call Context Change:

    - move context => Outer, OuterDyn
    - move current => Outer, OuterDyn, Lib, Static


Abst Call Param:

    - Change( bytes[0] )
    - Append( bytes[0] )
    - PermitHAC(      to_addr[21], amount[3~] )
    - PermitSAT(      to_addr[21], satoshi[8] )
    - PermitHACD(     to_addr[21], dianum[1], diamonds[6~] )
    - PermitAsset(    to_addr[21], serial[8], amount[8] )
    - PayableHAC(   from_addr[21], amount[3~] )
    - PayableSAT(   from_addr[21], satoshi[8] )
    - PayableHACD(  from_addr[21], dianum[1], diamonds[6~] )
    - PayableAsset( from_addr[21], serial[8], amount[8] )


Add Opcode must Modified:

    1. Bytecode define enum     `./rt/bytecode.rs`
    2. Bytecode metadata table  `./rt/bytecode.rs`
    3. Gas table                `./rt/gas.rs`
    4. lang func define         `./rt/lang.rs`
    5. interpreter              `./interpreter/execute.rs`



