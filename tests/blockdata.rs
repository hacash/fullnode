

mod devtest {


    #[test]
    fn t1() {
        use protocol::*;
        use server::ctx::*;

        /*
        let blkhex = "010000099c590067b70dcb00000000000a73b89196ec94154f7a0b8cbb860d091a1b9acb0f4a9bc4fe8f28c9fa20ed39a0c31e91c5fafd7d7bd1507ce2e4ef3abee8c5208c3b9e1c1fb5ec000000253bc65a08d495b58c000000001cbc259fce41b6ddf5d53b015287fbd34b20e6bef80108202020202020202020202020202020200165f4c2739228e22f4ea2eae4ced9406a972640c71742515391f0a2e5cd3211a400020067b7024c007a076f8c04740d5990dde38b9067f12ba4c6bb6df4010100010001008c084e7d6d1b7d59b94a7b1e3148d79cc39964aaf4035d894f0001029b077fd5e94592343fffc62c4d36e71938a8107c41c2b2ff074768a84cc49693d4c382573e2bafe705d5129c1487ef49c984c56df1206a4f9d3435a75c48bb6d5c65ddca6c64e75c2036df4a95ce65353c05fd570c051742f14a84cc968395f30000020067b7024e00fe157d1720bd6ab284c1b70faae9d3d09c1ce93bf4010100010001008c084e7d6d1b7d59b94a7b1e3148d79cc39964aaf4032cb54f000102c644c5a45d598cf520c365e006a23a803b84d5aa5875a01ecb9d9aa33dfcb367646edd4aa3381cd3765e00549167abcaeea86cbfc939818dc493f27992a7cf7c564cb0abef2c1f6764f4ec645ae358ca45a3af0b9e0eaef07a676a662db4b3c80000020067b702a600c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e5af9fcf15cf30685a8e5e43fb54f538801ad246e20926e1e58cf84314856417ef4670b45842d4ff75d3028ac5a303acc41682a991d183cab2b29b7ce76469a8240000020067b7035c00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e53d376ec9055c437326b8889047b0b58a5d1903fb7aac26b46e1d1c216270336c205ebac3f32a3c15b1fae63d5e15b6cb0cb1ac1be4dc3df6033e0e73c93292290000020067b7041d00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e549734b6c14c06f67e21b2385b89863a0144fb0d6494aa9d4bcc75f8907f57637200e43f933eb7dd44b78e2e7ae17f88282ddda0366f56fceacbcfd9f7fe9ef530000020067b704d300c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e523b6fd27963612c3ba00ee454d15f932515ea68ce0d79325c5c9669ac2730c790e015b06ba43ebdc4f0cdfdeb75a992f95e76e1dc70c463a0f4f5ac63a9875db0000020067b7063e00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e56033198422b81c9724b93eee3c3ef578e0f7d267fffafc548ed5e7cf3ddde9cd4a1ea6f373dcbb54e8436c6bf9e9c1e5ec0129d0777f2605fd4b9af239a1f2e70000020067b706f300c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e55bc7a237a2068b747a66a31a3a15c2b9a61f6fc447c04f53862a865eb71bfe4a6024761b5e9b5b5baf47a38dc534b83a6ab7736e7c0526010107960fea7d6f480000020067b707a900c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e59efb5a36237cb570fe440b8e18a1d7cdffc33205321d2f12760575fec6f908be1c4224ea0a0f6b412f3dbd183cc07f51869a4b28536aa49846d7023c326c66430000020067b7085e00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e52d2122849e795436213f8e4344a5757a7e98d2b6354a30b245f1e23249bbc0b63e22f3d766feeb5ddf91b18eac7386a2002b2e493301ee100bfed7def74765860000020067b7091400c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e5c5c63e7dc6922d2426a88da96db2848bb3e1db0fe63836ce644165857ec8463171603e2b20d2356789cdf12bf813760fa2f8cfd1a752c2429a45a39a6158fb670000020067b709c900c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e598bfbd9ef24274fa7529284ed49d4a9a5d1700ea4fc9e522511c39119ad6708446529d57ad7dd94826332285fb184b78db66816f466a9778ae90ff90b0fd37fa0000020067b70a7f00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e5a5d97cb6edcb5543c26f732904e8b4b89562a6eb6687058ed0797ad753bd59455b998733a435756ca154b81fc77153ec5257fdffcc655f162d4a6ff668a65c850000020067b70b3500c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e565458adeea1a30131a808ecf1fadf3d018bf5eb3625d6ce9ec03ed5b683a777a368ca3f11441dc619e72324e581faf46837b11f0b79fd55155804640bb4ffd3a0000020067b70beb00c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e51b8979cb4a0ae50ccf8285c751a80716f5409c61d2d81a2f6334115d082c6751512f107f799eacb3b4c77aaaad8592bf38a462417ab586c419cb37fb3888bbd70000020067b70ca700c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e549928c102898c2f31e4ecbf4f7a377b7f504a6eadf5d14546f27e1b74fad49461222092bd95399be7fd84c5761cf5b4fdf1ca9e0e7cc37eefdad6a2da1c8acd50000020067b70d6700c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e59b3a02ff37fde8c749870ff75d7a535ce017e1ccb705b2203b0778ba55af42275d8d8f2fa907a9be4befbe9694d8b80ec6e6106d608e2484fc245ef0c75098880000020067b703540031ce0cc1fb0473a0352657a5daf14b48377b2f4ff401010001000549424b54414900ca06530abfb920b4c96af2f96a27c5a44d5c100d00010367a48ade0a6f4fe359060bd56612e3691c6d413478dbff13b1c43c24a396403f29445fa8b5386360a63f4fcaacc7649ca3c3a7ec40f10590f7646497c01b29336b9bf4504710bd1317ec2161da0eacf3a6ddeb238ebac84e33aaf09d8ac674ad0000020067b702a600dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0b48044306de3d804ce104443a39cedefe727e0809d4997c82518701beaced48244655a02f5870434b8807483f9d708bc797a25023b6b463169e3f44db3ea30270000020067b7035c00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0115493508d794b1c080803f6919307dd16fbebed91be2f50e26faf281579924673527d95e40b3741887bb8b772662d0567a00c941ebde878442e67bf9e065e9c0000020067b7041d00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd04cb4c0ded5f2be58a93d736e7c30841f709d2fe35b5d9ec796b4886319dd4e675a3c8ea821bf227e67c0570ed4f31726fdfa110f43e515828cb541546464c2f90000020067b704d300dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0f31f55f51c4b296801d4c174bf6421c1de43c48ea15c56be233031540a0f787d31e7580e00d409f86ab1992e9c29bc16836cfb334cd2b0f4d1ad28bed50e1b6d0000020067b7058800dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd08a1c65cd4072f38cb39aa59f0d8c499b4fb918a792a1c513ad07547c8d6343904f533d916edd197a94df139963c0a17056196877e5ba20a70bfc6f1351fc7b7d0000020067b7058800c1d48aa5dcc2eb17d474c7ac52bcf166b54742bcf401010001000100278307bd40099473222bd80c111fdae1353e4f43f303130192000102c9390612a91af8916565b331043cf56455f1cf7f3c96ef32ceea2b5f986ad2e5c54c133e2d9ef9fbf6c5fc2a33ed7a399d61dc42d760afca73a7200ed5064a9d7becfad0a4f39764d9c24e5bdd2d5f55ba056eb8f97180a6b2fb6cbc102101320000020067b7063e00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0273b7837a110726dcbd539c3ccac2421d0e3dcd54c6b92ef82953a886639645332e02476a5b2ed78cbd1cf0f8998e72701ec6c4ce549b3f65d85751b28a8cc020000020067b706f300dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd03d69621b0ae2b45125ba58f699308382de7cf6efacbe24077e54f3db6790783960af749ddfc3bfa13ad833bc1423e8d56a5410b3e6bf88d417f34801fe4d135d0000020067b707490031ce0cc1fb0473a0352657a5daf14b48377b2f4ff401010001000100cb4c66ca2e27e8a0aa6920511ccb48a7df39b017f2039bf16100010367a48ade0a6f4fe359060bd56612e3691c6d413478dbff13b1c43c24a396403f0698b4e3d203cb1a3c2693c77e1ec0469106bf8a082350c9d8dea2debb98ae9827d7e5e4005fb85436a3e16ffebbcb65be01897472e61004a2d72f0a294d526f0000020067b707a900dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd014a84ea541f620d69624ce57ce9d2c6a07f4daf3888d5065709b81a0bc6bf9787f2cd619ac312d1e00a6bc20090f8da66d8c714ba1e2cac4d2cde5efde0b213c0000020067b7085e00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0c1b317a370a36f3c575da0814828f550c89d5f70ea3cb681095b541d978a01de5f63d3000057affec894adaee3cca5264e02de2deb66263f13ab375a5853d1f90000020067b7091400dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0587e07031ee623812db21af4cd8ea2b1faa90298eb8e30919214d71d080815636aee976c6b5809de66c7824071b91b84a7aae5b006c9fea2c91544f68651a0350000020067b709c900dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd04a56e82bc512ffd046ba7a01dc8087a64cf6e7ae0b52ffa78e9275411a534dae32f8b2ea6777bb7c72e518ba17f498b58a9d810e003cb4e7b636c8d031b9e4cb0000020067b70a7f00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0b616be7d8b96ffc572120129ecbd0a88e1400d898cbec57c672e0b11f07c09e07bb7d8567e295c6e7074ac21633a26253e249e640ddb61f74f01a9adc2f6461b0000020067b70b3500dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd056dcd31a77d1fbb6381b9cca619adf934da3e123fe7ff1f73aba2804e5a1c8ea2d8ffe560e9feb9134e3431f9b2f933dcab5d255a27c3bec80ac0caae775d4c60000020067b70beb00dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0593619af85facc13b3d3e2062046d2226f736d040e79e92b516c553cf5d56ee400d1626ff647d913b40c072bcd522ea95f7d308285e2b8c332a997198836f15c0000020067b70ca700dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd012bcbefa9634cede108572ea93ca9eeced61d153c6268a1cd5e633b512fdf20c08ddddfc9aaa525a7e870b432e32ea2d7c48e906269efacd5080589de809d4b10000020067b70d6700dc3d5bbf9f461051c551737c0fcbcffab2a49896f401010001000100278307bd40099473222bd80c111fdae1353e4f43f0050bcfa6fa9a0001039c4b527c22af8b57ec6f899a230545c89fe77e2636677272cc546c4df3b70fd0d9f559d3ceb7c799a0f52a19409f4a1fce2cee73b6256b13928395e69d16bd53658b5f48103055c5d81fe76f05b565662e44648f38366edd62338fe90591d4fa0000";
        let blkdts = hex::decode(blkhex).unwrap();
        let blk = BlockPkg::build(blkdts).unwrap();

        let txs = blk.objc.transactions();
        for i in 1..txs.len() {
            let tx = &txs[i];
            println!("---- tx {}: ", tx.hash());
            let acts = tx.actions();
            for a in 0..acts.len() {
                let act = &acts[a];
                println!( "act: {:?}", action_to_json_desc(tx.as_read(), act.as_ref(), "mei", true, false) )
            }
        }


       */


    }

}