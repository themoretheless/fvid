//! Offline fixture-generation helper: expose owned parser region boundaries.
//! Independent Python polynomial division supplies the CRC; tests never run this.
use std::io::{self, Read};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let rows: serde_json::Value = serde_json::from_str(&input).unwrap();
    let result: Vec<_> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            fvid_media::owned_aac::adts_crc::regions(
                &hex(r["payload"].as_str().unwrap()),
                &hex(r["asc"].as_str().unwrap()),
            )
            .unwrap()
            .iter()
            .map(|s| serde_json::json!({"start":s.start,"end":s.end,"width":s.width}))
            .collect::<Vec<_>>()
        })
        .collect();
    println!("{}", serde_json::to_string(&result).unwrap());
}
