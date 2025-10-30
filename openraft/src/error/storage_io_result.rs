use crate::RaftTypeConfig;
use crate::StorageError;
use crate::storage::SnapshotSignature;
use crate::type_config::alias::LogIdOf;

/// Simplified error conversion from io::Error to StorageError
///
/// Provides methods that mirror StorageError creation methods for easier error handling.
pub(crate) trait StorageIOResult<C, T>
where C: RaftTypeConfig
{
    /// Convert io::Error to StorageError for writing a single log entry
    #[allow(dead_code)]
    fn sto_write_log_entry(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading a single log entry
    #[allow(dead_code)]
    fn sto_read_log_entry(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading a log entry at an index
    #[allow(dead_code)]
    fn sto_read_log_at_index(self, log_index: u64) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for writing multiple log entries
    fn sto_write_logs(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading multiple log entries
    fn sto_read_logs(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for writing vote state
    fn sto_write_vote(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading vote state
    fn sto_read_vote(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for applying a log entry
    fn sto_apply(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for writing to state machine
    #[allow(dead_code)]
    fn sto_write_sm(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading from state machine
    fn sto_read_sm(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for writing a snapshot
    fn sto_write_snapshot(self, signature: Option<SnapshotSignature<C>>) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for reading a snapshot
    fn sto_read_snapshot(self, signature: Option<SnapshotSignature<C>>) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for general read operations
    #[allow(dead_code)]
    fn sto_read(self) -> Result<T, StorageError<C>>;

    /// Convert io::Error to StorageError for general write operations
    fn sto_write(self) -> Result<T, StorageError<C>>;
}

impl<C, T> StorageIOResult<C, T> for Result<T, std::io::Error>
where C: RaftTypeConfig
{
    fn sto_write_log_entry(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write_log_entry(log_id, &e))
    }

    fn sto_read_log_entry(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_log_entry(log_id, &e))
    }

    fn sto_read_log_at_index(self, log_index: u64) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_log_at_index(log_index, &e))
    }

    fn sto_write_logs(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write_logs(&e))
    }

    fn sto_read_logs(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_logs(&e))
    }

    fn sto_write_vote(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write_vote(&e))
    }

    fn sto_read_vote(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_vote(&e))
    }

    fn sto_apply(self, log_id: LogIdOf<C>) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::apply(log_id, &e))
    }

    fn sto_write_sm(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write_state_machine(&e))
    }

    fn sto_read_sm(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_state_machine(&e))
    }

    fn sto_write_snapshot(self, signature: Option<SnapshotSignature<C>>) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write_snapshot(signature, &e))
    }

    fn sto_read_snapshot(self, signature: Option<SnapshotSignature<C>>) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read_snapshot(signature, &e))
    }

    fn sto_read(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::read(&e))
    }

    fn sto_write(self) -> Result<T, StorageError<C>> {
        self.map_err(|e| StorageError::write(&e))
    }
}
