
//
const express = require('express')

const app = express();

app.use(express['static']('../dist/web'));


app.listen(8888, function(){
    console.log(`app listening on port 8888!`)
});




/*

const sdk = require('../dist/nodejs/hacashsdk.js')
// import { CoinTransferParam } from '../dist/nodejs/hacashsdk.js'


console.log(sdk.hac_to_mei("12:244"))

let pms = new sdk.CoinTransferParam()
console.log(pms)
pms.timestamp = BigInt(9999)

console.log(sdk.create_coin_transfer(pms))


*/

