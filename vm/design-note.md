

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


Call Privileges:

    - Main                 => Ext, ExtDyn, Static, Code
    - Abst                 => Lib, Static, Code
    - Extenal | Location   => Ext, ExtDyn, Loc, Lib, Static, Code (support all types)
    - Library              => Lib, Static, Code
    - Static               => Static, Code
    - Code                 => ()


Call Context Change:

    - move context => Ext, ExtDyn
    - move current => Ext, ExtDyn, Lib, Static


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


