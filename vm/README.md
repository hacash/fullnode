HVM Testnet
===

This document provides you with the most critical code guidelines for the core design of HVM, as well as the manual to run a local testnet and implement simple contracts to deploy and invoke.


### key Code Guidelines

1. Opcode define: [vm/src/rt/bytecode.rs](vm/src/rt/bytecode.rs)
2. Opcode execute: [vm/src/interpreter/execute.rs](vm/src/interpreter/execute.rs)
3. Opcode gas table: [vm/src/rt/gas.rs](vm/src/rt/gas.rs)
4. Machine resource: [vm/src/machine/resource.rs](vm/src/machine/resource.rs)
5. Contract address: [vm/src/field/address.rs](vm/src/field/address.rs)
6. Contract struct: [vm/src/field/contract.rs](vm/src/field/contract.rs)
7. Contract call: [vm/src/frame/call.rs](vm/src/frame/call.rs)
8. Transfer hook: [src/hook/action.rs](src/hook/action.rs)
9. IR language defing: [vm/src/rt/lang.rs](vm/src/rt/lang.rs)
10. IR language compile: [vm/src/lang/syntax.rs](vm/src/lang/syntax.rs)
11. IR language example: [vm/tests/lang_syntax.rs](vm/tests/lang_syntax.rs)


### Run Local Testnet

Check the crate codes with cargo:

```sh
cargo check
cd ../ && cargo check
cd vm
```

Build local testnet fullnode:

```sh
mkdir ./testnet && cd ../
cargo build --bin fullnode --features hvm
cp ./target/debug/fullnode ./vm/testnet/fullnode
cd vm
```

Build test poworker:

```sh
cd ../
cargo build --release --bin poworker
cp ./target/release/poworker ./vm/testnet/poworker
cd vm
```

Run local testnet:

```sh
cd testnet
cp fullnode_config_ini.txt fullnode.config.ini
./fullnode fullnode.config.ini
```

Run poworker to mint blocks:

```sh
cd testnet
cp poworker_config_ini.txt poworker.config.ini
./poworker poworker.config.ini
```

At this time, we can see that the local full node is minting blocks.


### Create & Deploy && Call Contract

At present, there is no perfect IDE tool to write the contract, for the time being, you can use Rust code to directly generate the contract, the example code is in: [vm/tests/lang_syntax.rs](vm/tests/lang_syntax.rs). 

Here's a simple and usable contract to test: [vm/src/hook/test.rs](vm/src/hook/test.rs). 

```sh
cargo test deploy::recursion -- --nocapture
```

This will result in a POST submission command line containing the contract deployment transaction:

```sh
curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006842549600e63c33a796b3032ce6b856f68fccf06608d9ed18f501020001007a000000000000000000000000000000020500000c0601434e0308f0d180437cec0f000008070143480c437bec0001e3b674a0800004415080eb0000000000000000f5010600010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263ce7b05f535f7fde34a23437fb0292bf8ff6c4c2889769d5d7b2b5ccac4e9f4cfe73520e164e55b6c602f1552d45c79bfbb818d5659d12c5fe2a9f8c8dfc2c5b160400"
```

Run it in the terminal, you can deploy the test contract to the local full node.

To call this contract, run the following test code to generate another transaction:

```sh
cd hvm
cargo test hook::maincall::test3 -- --nocapture
cd ..
```

Submit the following transaction to be able to call the deployed contract through HAC transfer. Note that the contract's test code logic is that the transfer will only succeed after the block height is 15, which means that the transfer cannot be made to the contract until the block height is 15. You can modify the test contract and write other custom logic to achieve more asset management or account abstraction.

```sh
curl "http://127.0.0.1:8088/submit/transaction?hexbody=true" -X POST -d "03006842579e00e63c33a796b3032ce6b856f68fccf06608d9ed18f50102000100010135d4470300daabea474d082733333c1b694d8065f8010200010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c93d8d4049f09211cde012bf006a31071b8634d1e8c050e4c414d3f85ad57ded44f6a1b8d917806acffe0c26dce435e0f980951c1f24975506127d9d63d0cc7b10400"
```




### Other Tests


run test:

```
cargo test


# lang syntax 
cargo test --test lang_syntax -- --nocapture

```


```
cd vm 
cargo run   # run all vm test
cargo test lang::token_t::t1
```