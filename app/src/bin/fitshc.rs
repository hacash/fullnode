//! fitshc — IR hex → executable bytecode (FitSH source frontend pending in vm/fitsh_port).
//! Usage: `fitshc <ir.hex|ir.bin>`; prints verified runtime bytecode as hex.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!(
            "Usage: fitshc <file.ir.hex|file.bin>\n\
             Note: FitSH (.fitsh) source compiler is not wired yet;\n\
             sources are staged under vm/fitsh_port/ — compile IR payloads here."
        );
        std::process::exit(2);
    };
    let path = Path::new(&path);
    let raw = match fs::read(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("read {}: {}", path.display(), e);
            std::process::exit(1);
        }
    };
    let ir_bytes = if path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("hex") || e.eq_ignore_ascii_case("txt"))
        .unwrap_or(false)
        || raw
            .iter()
            .all(|b| b.is_ascii_hexdigit() || b.is_ascii_whitespace())
    {
        let s = String::from_utf8_lossy(&raw);
        let hex_body: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        match hex::decode(&hex_body) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("hex decode: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        raw
    };

    match vm::fitshc::convert_ir_to_runtime_bytecode(&ir_bytes) {
        Ok(codes) => {
            println!("{}", hex::encode(codes));
        }
        Err(e) => {
            eprintln!("compile error: {}", e);
            std::process::exit(1);
        }
    }
}
