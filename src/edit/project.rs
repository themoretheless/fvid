use super::{Operation, StreamPolicy};

/// A source-independent editing request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditProject {
    operations: Vec<Operation>,
    stream_policy: StreamPolicy,
}

impl EditProject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trim(mut self, range: super::TimeRange) -> Self {
        self.operations.push(Operation::Trim(range));
        self
    }

    pub fn transform(mut self, transform: super::Transform) -> Self {
        self.operations.push(Operation::Transform(transform));
        self
    }

    pub fn concat(mut self) -> Self {
        self.operations.push(Operation::Concat);
        self
    }

    pub fn streams(mut self, policy: StreamPolicy) -> Self {
        self.stream_policy = policy;
        self
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn stream_policy(&self) -> StreamPolicy {
        self.stream_policy
    }
}
