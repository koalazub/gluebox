fn main() {
    capnpc::CompilerCommand::new()
        .file("src/proto/gluebox.capnp")
        .run()
        .expect("capnp schema compilation failed");
}
