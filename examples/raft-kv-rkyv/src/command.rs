/// The application schema. Deliberately does not implement rkyv::Deserialize:
/// the state machine reads ArchivedCommand directly instead.
#[derive(rkyv::Archive, rkyv::Serialize)]
pub(crate) struct Command {
    pub(crate) key: String,
    pub(crate) value: String,
}
