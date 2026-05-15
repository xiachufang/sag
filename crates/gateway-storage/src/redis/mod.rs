pub mod counter;
pub mod kv;

pub use counter::RedisCounterStore;
pub use kv::RedisKvStore;

use crate::error::{Result, StorageError};

/// Connect to a Redis instance with the connection-manager (automatically
/// reconnects on transient drops).
pub async fn connect(url: &str) -> Result<redis::aio::ConnectionManager> {
    let client = redis::Client::open(url)
        .map_err(|e| StorageError::Unavailable(format!("redis open: {e}")))?;
    redis::aio::ConnectionManager::new(client)
        .await
        .map_err(|e| StorageError::Unavailable(format!("redis connect: {e}")))
}
