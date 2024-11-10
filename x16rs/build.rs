extern crate cc;


fn main() {
    cc::Build::new()
        .file("src/x16rs.c")
        .compile("x16rs");
    println!("cargo:rerun-if-changed=src/x16rs.c");
}





/*


## linux 

# - static
gcc -c src/x16rs/x16rs.c && ar rcs libx16rs.a x16rs.o && mv *.a ./src/x16rs && rm -f *.o

# - dyn
gcc -fPIC -shared -o src/x16rs/libx16rs.so src/x16rs/x16rs.c

## test
rustc src/x16rs/test.rs -l x16rs -L ./src/x16rs
env LD_LIBRARY_PATH=./src/x16rs ./test


# cargo build --release --target=x86_64-unknown-linux-musl
# ldd target/x86_64-unknown-linux-musl/release/hacash

# Windows
[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+crt-static"]

# /usr/local/musl/bin


*/





/*


build release software

Ubuntu: 
RUSTFLAGS='-C target-feature=+crt-static' RUST_BACKTRACE=1 cargo build --release --target x86_64-unknown-linux-gnu
cp target/x86_64-unknown-linux-gnu/release/hacash ../../download_rust/hacash_ubuntu


Windows:
set RUSTFLAGS='-C target-feature=+crt-static' ; set RUST_BACKTRACE=1 ; cargo build --release --target x86_64-pc-windows-msvc
cp target/x86_64-pc-windows-msvc/release/hacash.exe ./hacash_windows.exe

MacOS:
RUSTFLAGS='-C target-feature=+crt-static' RUST_BACKTRACE=1 cargo build --release --target x86_64-apple-darwin
cp target/x86_64-apple-darwin/release/hacash ./hacash_macos


*/






/*

# Step 1: create libx16rs.a
# Step 2: build and run

gcc -c src/x16rs/x16rs.c && ar rcs libx16rs.a x16rs.o && mv *.a ./src/x16rs && rm -f *.o

RUSTFLAGS="$RUSTFLAGS -Awarnings -L ./src/x16rs/" cargo run

# Build static release software
cargo build --release --target=x86_64-unknown-linux-musl
ldd target/x86_64-unknown-linux-musl/release/hacash


RUSTFLAGS="$RUSTFLAGS -Awarnings -L ./src/x16rs/" cargo build && 
cp ./target/debug/hacash ./test/hacash_dev1 &&
./test/hacash_dev1 hacash.dev1.config.ini


RUSTFLAGS="$RUSTFLAGS -Awarnings" RUST_BACKTRACE=1 cargo run -- --reptblk


*/ 
