

# nodejs
./build.sh nodejs
mkdir -p dist/nodejs
mv dist/hacashsdk.js ./dist/nodejs
mv dist/hacashsdk_bg.wasm ./dist/nodejs

# web
./build.sh web
mkdir -p dist/web
mv dist/hacashsdk.js ./dist/web
mv dist/hacashsdk_bg.wasm ./dist/web

# page
./build.sh no-modules
node pack.js
mkdir -p dist/page
mv dist/hacashsdk_bg.js ./dist/page
cp ./tests/test.html ./dist/page/test.html

# 
rm -f dist/*.js dist/*.ts dist/*.wasm 




