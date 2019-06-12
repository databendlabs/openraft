use prost_build;

fn main() {
    // Compile protobuf code.
    let _ = prost_build::Config::new()
        .out_dir("src/proto")
        .compile_protos(&["protobuf/raft.proto"], &["protobuf"])
        .map_err(|err| panic!("Failed to compile protobuf code. {}", err));
}
