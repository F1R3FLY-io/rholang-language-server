fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/repl.proto")?;
    tonic_build::compile_protos("proto/lsp.proto")?;
    Ok(())
}
