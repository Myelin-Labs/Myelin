fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/session_network.proto");
    tonic_build::configure().build_server(true).build_client(true).compile_protos(&["proto/session_network.proto"], &["proto"])?;
    Ok(())
}
