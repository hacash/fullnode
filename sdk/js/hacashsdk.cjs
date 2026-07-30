const path = require("path");
const { pathToFileURL } = require("url");

const moduleFile = pathToFileURL(path.join(__dirname, "hacashsdk.mjs")).href;

async function create_hacash_sdk(options) {
    const moduleApi = await import(moduleFile);
    return moduleApi.create_hacash_sdk(options);
}

module.exports = create_hacash_sdk;
module.exports.create_hacash_sdk = create_hacash_sdk;
module.exports.default = create_hacash_sdk;
