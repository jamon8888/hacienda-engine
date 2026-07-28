//! Human review queue for low-confidence PII detections.
//!
//! Satisfies the human-oversight requirement of AI Act Article 14: detections the
//! pipeline is not confident about are routed to a reviewer instead of being applied
//! silently.

pub mod error;
pub mod queue;
pub mod types;

pub use error::ReviewError;
pub use queue::ReviewQueue;
pub use types::{
    Priority, QueueStats, ReviewConfig, ReviewDecision, ReviewQueueItem, ReviewRequest,
    ReviewStatus,
};
