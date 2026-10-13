//! Offline fixture helper: one JSON sequence-payload byte array per line.
use fvid::codec::av1_sequence::Sequence;
use std::io::{self, BufRead, Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let result = (|| -> Result<[u8; 2], Box<dyn std::error::Error>> {
            let payload: Vec<u8> = serde_json::from_str(&line?)?;
            Ok(Sequence::parse(&payload)?.codec_configuration_fields()?)
        })();
        let response = match result {
            Ok(fields) => serde_json::json!({"fields": fields}),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}
