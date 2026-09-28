#![deny(clippy::all, clippy::pedantic, clippy::nursery)]

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::io::Read;

fn main() -> Result<(), String> {
    let exe_path = std::env::args().nth(1).unwrap_or_else(|| {
        if cfg!(windows) {
            "target/release/senseaim.exe".to_string()
        } else {
            "target/release/senseaim".to_string()
        }
    });

    let mut file = std::fs::File::open(&exe_path)
        .map_err(|_| format!("FAILED TO OPEN TARGET BINARY: {exe_path}"))?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buffer[..n]),
            Err(_) => return Err("FAILED TO READ TARGET BINARY".to_string()),
        }
    }
    let hash_result = hasher.finalize();

    let secret_key_bytes: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    let signing_key = SigningKey::from_bytes(&secret_key_bytes);
    let signature = signing_key.sign(&hash_result);

    std::fs::write("senseaim.sig", signature.to_bytes())
        .map_err(|_| "FAILED TO WRITE SIGNATURE FILE".to_string())?;

    Ok(())
}
