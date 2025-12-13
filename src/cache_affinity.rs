use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

#[derive(Debug, Clone)]
pub struct CacheAffinity {
    pub provider_id: String,
    pub expire_at: f64,
    pub request_count: u32,
}

#[derive(Clone)]
pub struct CacheAffinityManager {
    store: Arc<RwLock<HashMap<String, CacheAffinity>>>,
    default_ttl: u64,
}

impl CacheAffinityManager {
    pub fn new(default_ttl: u64) -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
        }
    }

    /// Generate cache affinity key
    /// Format: {user_id}:{kind}:{model}
    pub fn generate_key(user_id: &str, kind: &str, model: &str) -> String {
        format!("{}:{}:{}", user_id, kind, model)
    }

    /// Get cached provider ID if affinity exists and is valid
    pub async fn get(&self, key: &str) -> Option<String> {
        // First, try with read lock (fast path for concurrent reads)
        {
            let store = self.store.read().await;

            if let Some(affinity) = store.get(key) {
                let now = current_time();

                // Check if expired
                if now >= affinity.expire_at {
                    // Need to remove - drop read lock and acquire write lock
                    drop(store);
                    let mut store = self.store.write().await;
                    store.remove(key);
                    tracing::debug!("Cache affinity expired: {}", key);
                    return None;
                }

                // Valid affinity found
                let provider_id = affinity.provider_id.clone();

                // Drop read lock before acquiring write lock for counter update
                drop(store);

                // Update request count with write lock
                let mut store = self.store.write().await;
                if let Some(affinity) = store.get_mut(key) {
                    affinity.request_count += 1;
                    tracing::debug!(
                        "Cache affinity hit: {} → {} (count: {})",
                        key,
                        provider_id,
                        affinity.request_count
                    );
                }

                return Some(provider_id);
            }
        }

        None
    }

    /// Set cache affinity for a key
    pub async fn set(&self, key: &str, provider_id: &str) {
        let now = current_time();
        let affinity = CacheAffinity {
            provider_id: provider_id.to_string(),
            expire_at: now + self.default_ttl as f64,
            request_count: 1,
        };

        let mut store = self.store.write().await;
        store.insert(key.to_string(), affinity);

        tracing::debug!(
            "Cache affinity set: {} → {} (expires in {}s)",
            key,
            provider_id,
            self.default_ttl
        );
    }

    /// Invalidate cache affinity (called when cached provider fails)
    pub async fn invalidate(&self, key: &str) {
        let mut store = self.store.write().await;
        if store.remove(key).is_some() {
            tracing::warn!("Cache affinity invalidated: {}", key);
        }
    }

    /// Start background cleanup task
    pub fn start_cleanup_task(manager: Arc<Self>) {
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(60));

            loop {
                ticker.tick().await;

                let mut store = manager.store.write().await;
                let now = current_time();
                let before = store.len();

                store.retain(|_, affinity| affinity.expire_at > now);

                let removed = before - store.len();
                if removed > 0 {
                    tracing::debug!(
                        "Cleaned up {} expired cache affinities ({} remaining)",
                        removed,
                        store.len()
                    );
                }
            }
        });
    }
}

/// Get current Unix timestamp in seconds
fn current_time() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs_f64()
}

/// Hash a string (for user_id generation from API key)
pub fn hash_string(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8]) // Use first 8 bytes for brevity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_cache_affinity_basic() {
        let manager = CacheAffinityManager::new(300);
        let key = "user123:claude:claude-sonnet-4";

        // Initially no affinity
        assert!(manager.get(key).await.is_none());

        // Set affinity
        manager.set(key, "provider1").await;

        // Should return cached provider
        assert_eq!(manager.get(key).await, Some("provider1".to_string()));

        // Request count should increment
        assert_eq!(manager.get(key).await, Some("provider1".to_string()));

        let store = manager.store.read().await;
        assert_eq!(store.get(key).unwrap().request_count, 3);
    }

    #[tokio::test]
    async fn test_cache_affinity_expiry() {
        let manager = CacheAffinityManager::new(1); // 1 second TTL
        let key = "user123:claude:claude-sonnet-4";

        manager.set(key, "provider1").await;
        assert!(manager.get(key).await.is_some());

        // Wait for expiry
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Should be expired
        assert!(manager.get(key).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_affinity_invalidate() {
        let manager = CacheAffinityManager::new(300);
        let key = "user123:claude:claude-sonnet-4";

        manager.set(key, "provider1").await;
        assert!(manager.get(key).await.is_some());

        manager.invalidate(key).await;
        assert!(manager.get(key).await.is_none());
    }

    #[test]
    fn test_hash_string() {
        let hash1 = hash_string("sk-ant-api-key-123");
        let hash2 = hash_string("sk-ant-api-key-456");

        assert_ne!(hash1, hash2);
        assert_eq!(hash1.len(), 16); // 8 bytes in hex = 16 chars
    }
}
