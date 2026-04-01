fn main() {
    let mut cmd = capnpc::CompilerCommand::new();
    if let Ok(capnp) = std::env::var("CAPNP") {
        cmd.capnp_executable(capnp);
    }
    cmd.file("src/proto/gluebox.capnp")
        .run()
        .expect("capnp schema compilation failed");
}
