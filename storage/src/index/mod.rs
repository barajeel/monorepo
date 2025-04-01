//! A memory-efficient index for values stored in a journal.

mod storage;
pub use storage::{Index, LocationIterator};
pub mod translator;

use std::hash::Hash;

/// Translate keys into an internal representation used in `Archive`'s
/// in-memory index.
///
/// If invoking `transform` on keys results in many conflicts, the performance
/// of `Archive` will degrade substantially.
pub trait Translator: Clone {
    type Key: Eq + Hash + Send + Sync + Clone;

    /// Transform a key into its internal representation.
    fn transform(&self, key: &[u8]) -> Self::Key;
}
