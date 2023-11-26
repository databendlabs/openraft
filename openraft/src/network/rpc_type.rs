use std::fmt;

#[derive(Debug, Clone, Copy)]
#[derive(PartialEq, Eq)]
#[derive(Hash)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum RPCTypes {
    Vote,
    AppendEntries,
    InstallSnapshot,
}

impl fmt::Display for RPCTypes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
