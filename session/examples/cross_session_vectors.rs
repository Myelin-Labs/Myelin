//! Check the public ADR-008 commitment corpus without a database or service.
#[path = "support/protocol_reference.rs"]
mod reference;

fn main() -> Result<(), String> {
    let mut corpus = reference::corpus()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            for vector in &corpus.vectors {
                reference::verify(vector)?;
            }
            println!("verified {} protocol commitment vectors", corpus.vectors.len());
        }
        [arg] if arg == "--emit" => {
            for vector in &mut corpus.vectors {
                let bytes = reference::preimage(vector)?;
                vector.preimage_hex = reference::hex(&bytes);
                vector.blake3 = blake3::hash(&bytes).to_hex().to_string();
            }
            println!("{}", serde_json::to_string_pretty(&corpus).map_err(|error| error.to_string())?);
        }
        _ => return Err("usage: cross_session_vectors [--emit]".to_owned()),
    }
    Ok(())
}
