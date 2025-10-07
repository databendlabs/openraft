// Measure the size of error types to guide optimization
use std::mem::size_of;

use openraft::BasicNode;
use openraft::ErrorSubject;
use openraft::ErrorVerb;
use openraft::StorageError;

// Define a simple test config
openraft::declare_raft_types!(
    pub TestConfig:
        NodeId = u64,
        Node = BasicNode,
        Entry = openraft::Entry<TestConfig>,
        SnapshotData = std::io::Cursor<Vec<u8>>,
        AsyncRuntime = openraft::impls::TokioRuntime
);

#[test]
fn measure_storage_error_size() {
    println!("\n=== StorageError Size Analysis ===\n");

    let storage_error_size = size_of::<StorageError<TestConfig>>();
    println!("StorageError<TestConfig>:         {storage_error_size} bytes");

    println!("\n--- Components ---");
    println!(
        "ErrorSubject<TestConfig>:         {} bytes",
        size_of::<ErrorSubject<TestConfig>>()
    );
    println!("ErrorVerb:                        {} bytes", size_of::<ErrorVerb>());
    println!(
        "Box<anyerror::AnyError>:          {} bytes",
        size_of::<Box<anyerror::AnyError>>()
    );
    println!(
        "Option<String>:                   {} bytes",
        size_of::<Option<String>>()
    );

    println!("\n--- ErrorSubject Variants ---");
    use openraft::LogId;
    use openraft::SnapshotId;
    use openraft::storage::SnapshotSignature;

    println!(
        "LogId<TestConfig>:                {} bytes",
        size_of::<LogId<TestConfig>>()
    );
    println!(
        "Option<LogId<TestConfig>>:        {} bytes",
        size_of::<Option<LogId<TestConfig>>>()
    );
    println!(
        "SnapshotSignature<TestConfig>:    {} bytes",
        size_of::<SnapshotSignature<TestConfig>>()
    );
    println!(
        "Option<SnapshotSignature<TestConfig>>: {} bytes",
        size_of::<Option<SnapshotSignature<TestConfig>>>()
    );

    println!("\n--- SnapshotSignature Components ---");
    println!(
        "Option<Box<LogId<TestConfig>>>:   {} bytes",
        size_of::<Option<Box<LogId<TestConfig>>>>()
    );
    println!("SnapshotId (String):              {} bytes", size_of::<SnapshotId>());

    println!("\n--- Related Types ---");
    use openraft::error::Fatal;
    use openraft::error::RaftError;

    println!(
        "RaftError<TestConfig, ()>:        {} bytes",
        size_of::<RaftError<TestConfig, ()>>()
    );
    println!(
        "Fatal<TestConfig>:                {} bytes",
        size_of::<Fatal<TestConfig>>()
    );

    println!("\n=== Target ===");
    println!("Current:                          {storage_error_size} bytes");
    println!("Target:                           < 32 bytes");
    println!(
        "Reduction needed:                 {} bytes\n",
        storage_error_size.saturating_sub(32)
    );

    // Fail the test if size is too large (for tracking)
    if storage_error_size > 136 {
        panic!("StorageError size increased from 136 to {storage_error_size} bytes! This is a regression.");
    }
}
