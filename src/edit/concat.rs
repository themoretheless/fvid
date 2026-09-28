/// Marker for concatenation in the editing model.
///
/// Actual source compatibility checks and packet/frame execution belong to the
/// future format/runtime integration, not to the request model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Concat;
