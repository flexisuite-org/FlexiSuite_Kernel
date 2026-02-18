// Re-export event types from kernel-data to maintain API compatibility
// and resolved circular dependency.
pub use kernel_data::event::{
    Delivery, EventEnvelope, EventError, OrderMode, PublishAck, ReliableConsumer, ReliableProducer,
    RetryPolicy, SHARD_COUNT, validate_stream_key,
};
