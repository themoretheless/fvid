use super::Operation;

/// The execution strategy selected by the planner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionMode {
    PacketCopy,
    DecodeProcessEncode,
}

/// Select the simplest valid execution strategy for an editing request.
pub fn execution_mode(operations: &[Operation]) -> ExecutionMode {
    if operations.iter().any(|operation| matches!(operation, Operation::Transform(_))) {
        ExecutionMode::DecodeProcessEncode
    } else {
        ExecutionMode::PacketCopy
    }
}
