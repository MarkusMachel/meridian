//! Every data error names the file, the item and the problem.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Io,
    Malformed,
    MissingField,
    WrongVersion,
    DuplicateId,
    NonContiguousId,
    UnknownReference,
    BadValue,
    BadBinary,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataError {
    pub file: String,
    /// What the error is about, e.g. `province[3] (id 7)`; `-` for whole-file problems.
    pub item: String,
    pub kind: ErrorKind,
    pub problem: String,
}

impl DataError {
    pub fn new(file: &str, item: impl Into<String>, kind: ErrorKind, problem: impl Into<String>) -> DataError {
        DataError { file: file.to_string(), item: item.into(), kind, problem: problem.into() }
    }
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {:?}: {}", self.file, self.item, self.kind, self.problem)
    }
}

impl std::error::Error for DataError {}
