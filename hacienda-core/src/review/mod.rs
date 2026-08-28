//! Human review queue for low-confidence PII detections.
//!
//! Satisfies the human-oversight requirement of AI Act Article 14: detections the
//! pipeline is not confident about are routed to a reviewer instead of being applied
//! silently.

/// Module error
pub mod error;
/// Module queue
pub mod queue;
/// Module store
pub mod store;
/// Module store_file
pub mod store_file;
/// Module types
pub mod types;

pub use error::ReviewError;
pub use queue::ReviewQueue;
pub use store::{InMemoryReviewStore, ReviewStore};
pub use store_file::FileReviewStore;
pub use types::{
    Priority, QueueStats, ReviewConfig, ReviewDecision, ReviewQueueItem, ReviewRequest,
    ReviewStatus,
};
