//! Native VRTc storage engine with memory and embedded redb backends.
//!
//! VRTc owns namespacing, validation, limits, batch semantics, and backend
//! selection. The redb backend is an optional embedded persistence primitive.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use redb::{Database, ReadableTable, TableDefinition};
use vrtc_core::{ErrorCategory, VrtCError};

const KV_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("vrtc_kv");

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub max_key_bytes: usize,
    pub max_value_bytes: usize,
    pub max_batch_operations: usize,
    pub max_scan_items: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            max_key_bytes: 1_024,
            max_value_bytes: 4 * 1024 * 1024,
            max_batch_operations: 1_000,
            max_scan_items: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageOperation {
    Put {
        namespace: String,
        key: String,
        value: Vec<u8>,
    },
    Delete {
        namespace: String,
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageEntry {
    pub namespace: String,
    pub key: String,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageBatchResult {
    pub operations_applied: usize,
}

trait StorageBackend: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, VrtCError>;
    fn apply(&self, operations: &[(String, Option<Vec<u8>>)]) -> Result<(), VrtCError>;
    fn scan_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<(String, Vec<u8>)>, VrtCError>;
}

#[derive(Default)]
struct MemoryBackend {
    values: RwLock<BTreeMap<String, Vec<u8>>>,
}

impl StorageBackend for MemoryBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, VrtCError> {
        let values = self.values.read().map_err(|_| {
            storage_error(
                "STORAGE_LOCK_POISONED",
                "The in-memory storage read lock is unavailable",
                ErrorCategory::Internal,
                true,
            )
        })?;
        Ok(values.get(key).cloned())
    }

    fn apply(&self, operations: &[(String, Option<Vec<u8>>)]) -> Result<(), VrtCError> {
        let mut values = self.values.write().map_err(|_| {
            storage_error(
                "STORAGE_LOCK_POISONED",
                "The in-memory storage write lock is unavailable",
                ErrorCategory::Internal,
                true,
            )
        })?;
        for (key, value) in operations {
            if let Some(value) = value {
                values.insert(key.clone(), value.clone());
            } else {
                values.remove(key);
            }
        }
        Ok(())
    }

    fn scan_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<(String, Vec<u8>)>, VrtCError> {
        let values = self.values.read().map_err(|_| {
            storage_error(
                "STORAGE_LOCK_POISONED",
                "The in-memory storage read lock is unavailable",
                ErrorCategory::Internal,
                true,
            )
        })?;
        Ok(values
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .take(limit)
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

struct RedbBackend {
    database: Database,
}

impl RedbBackend {
    fn open(path: &Path) -> Result<Self, VrtCError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|_| {
                storage_error(
                    "STORAGE_DIRECTORY_CREATE_FAILED",
                    "The embedded database directory could not be created",
                    ErrorCategory::Internal,
                    false,
                )
            })?;
        }
        let database = if path.exists() {
            Database::open(path)
        } else {
            Database::create(path)
        }
        .map_err(|_| {
            storage_error(
                "STORAGE_DATABASE_OPEN_FAILED",
                "The embedded database could not be opened or created",
                ErrorCategory::Internal,
                false,
            )
        })?;
        let transaction = database.begin_write().map_err(|_| {
            storage_error(
                "STORAGE_TABLE_INIT_FAILED",
                "The embedded database could not initialize its key-value table",
                ErrorCategory::Internal,
                false,
            )
        })?;
        {
            let _table = transaction.open_table(KV_TABLE).map_err(|_| {
                storage_error(
                    "STORAGE_TABLE_INIT_FAILED",
                    "The embedded key-value table could not be initialized",
                    ErrorCategory::Internal,
                    false,
                )
            })?;
        }
        transaction.commit().map_err(|_| {
            storage_error(
                "STORAGE_TABLE_INIT_FAILED",
                "The embedded key-value table initialization could not be committed",
                ErrorCategory::Internal,
                false,
            )
        })?;
        Ok(Self { database })
    }
}

impl StorageBackend for RedbBackend {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, VrtCError> {
        let transaction = self.database.begin_read().map_err(|_| {
            storage_error(
                "STORAGE_READ_TRANSACTION_FAILED",
                "The embedded database could not begin a read transaction",
                ErrorCategory::Internal,
                true,
            )
        })?;
        let table = transaction.open_table(KV_TABLE).map_err(|_| {
            storage_error(
                "STORAGE_TABLE_OPEN_FAILED",
                "The embedded key-value table could not be opened",
                ErrorCategory::Internal,
                false,
            )
        })?;
        let value = table.get(key).map_err(|_| {
            storage_error(
                "STORAGE_READ_FAILED",
                "The embedded database read failed",
                ErrorCategory::Internal,
                true,
            )
        })?;
        Ok(value.map(|guard| guard.value().to_vec()))
    }

    fn apply(&self, operations: &[(String, Option<Vec<u8>>)]) -> Result<(), VrtCError> {
        let transaction = self.database.begin_write().map_err(|_| {
            storage_error(
                "STORAGE_WRITE_TRANSACTION_FAILED",
                "The embedded database could not begin a write transaction",
                ErrorCategory::Internal,
                true,
            )
        })?;
        {
            let mut table = transaction.open_table(KV_TABLE).map_err(|_| {
                storage_error(
                    "STORAGE_TABLE_OPEN_FAILED",
                    "The embedded key-value table could not be opened",
                    ErrorCategory::Internal,
                    false,
                )
            })?;
            for (key, value) in operations {
                if let Some(value) = value {
                    table.insert(key.as_str(), value.as_slice()).map_err(|_| {
                        storage_error(
                            "STORAGE_WRITE_FAILED",
                            "The embedded database write failed",
                            ErrorCategory::Internal,
                            true,
                        )
                    })?;
                } else {
                    table.remove(key.as_str()).map_err(|_| {
                        storage_error(
                            "STORAGE_DELETE_FAILED",
                            "The embedded database delete failed",
                            ErrorCategory::Internal,
                            true,
                        )
                    })?;
                }
            }
        }
        transaction.commit().map_err(|_| {
            storage_error(
                "STORAGE_COMMIT_FAILED",
                "The embedded database transaction could not be committed",
                ErrorCategory::Internal,
                true,
            )
        })?;
        Ok(())
    }

    fn scan_prefix(&self, prefix: &str, limit: usize) -> Result<Vec<(String, Vec<u8>)>, VrtCError> {
        let transaction = self.database.begin_read().map_err(|_| {
            storage_error(
                "STORAGE_READ_TRANSACTION_FAILED",
                "The embedded database could not begin a read transaction",
                ErrorCategory::Internal,
                true,
            )
        })?;
        let table = transaction.open_table(KV_TABLE).map_err(|_| {
            storage_error(
                "STORAGE_TABLE_OPEN_FAILED",
                "The embedded key-value table could not be opened",
                ErrorCategory::Internal,
                false,
            )
        })?;
        let mut entries = Vec::new();
        let iterator = table.iter().map_err(|_| {
            storage_error(
                "STORAGE_SCAN_FAILED",
                "The embedded database scan failed",
                ErrorCategory::Internal,
                true,
            )
        })?;
        for entry in iterator {
            let (key, value) = entry.map_err(|_| {
                storage_error(
                    "STORAGE_SCAN_FAILED",
                    "An embedded database scan entry could not be read",
                    ErrorCategory::Internal,
                    true,
                )
            })?;
            let key = key.value().to_owned();
            if key.starts_with(prefix) {
                entries.push((key, value.value().to_vec()));
                if entries.len() >= limit {
                    break;
                }
            }
        }
        Ok(entries)
    }
}

pub struct StorageEngine {
    config: StorageConfig,
    backend: Arc<dyn StorageBackend>,
}

impl StorageEngine {
    pub fn memory(config: StorageConfig) -> Result<Self, VrtCError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            backend: Arc::new(MemoryBackend::default()),
        })
    }

    pub fn open_redb(path: impl AsRef<Path>, config: StorageConfig) -> Result<Self, VrtCError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            backend: Arc::new(RedbBackend::open(path.as_ref())?),
        })
    }

    pub fn get(&self, namespace: &str, key: &str) -> Result<Option<Vec<u8>>, VrtCError> {
        let key = self.storage_key(namespace, key)?;
        self.backend.get(&key)
    }

    pub fn put(&self, namespace: &str, key: &str, value: Vec<u8>) -> Result<(), VrtCError> {
        self.apply_batch(vec![StorageOperation::Put {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
            value,
        }])?;
        Ok(())
    }

    pub fn delete(&self, namespace: &str, key: &str) -> Result<(), VrtCError> {
        self.apply_batch(vec![StorageOperation::Delete {
            namespace: namespace.to_owned(),
            key: key.to_owned(),
        }])?;
        Ok(())
    }

    /// Apply a validated batch atomically in the selected backend.
    pub fn apply_batch(
        &self,
        operations: Vec<StorageOperation>,
    ) -> Result<StorageBatchResult, VrtCError> {
        if operations.len() > self.config.max_batch_operations {
            return Err(storage_error(
                "STORAGE_BATCH_TOO_LARGE",
                "The storage batch exceeds the configured operation limit",
                ErrorCategory::ResourceExhausted,
                false,
            ));
        }
        let mut validated = Vec::with_capacity(operations.len());
        for operation in &operations {
            match operation {
                StorageOperation::Put {
                    namespace,
                    key,
                    value,
                } => {
                    let storage_key = self.storage_key(namespace, key)?;
                    if value.len() > self.config.max_value_bytes {
                        return Err(storage_error(
                            "STORAGE_VALUE_TOO_LARGE",
                            "The storage value exceeds the configured byte limit",
                            ErrorCategory::ResourceExhausted,
                            false,
                        ));
                    }
                    validated.push((storage_key, Some(value.clone())));
                }
                StorageOperation::Delete { namespace, key } => {
                    validated.push((self.storage_key(namespace, key)?, None));
                }
            }
        }
        self.backend.apply(&validated)?;
        Ok(StorageBatchResult {
            operations_applied: validated.len(),
        })
    }

    pub fn scan_namespace(
        &self,
        namespace: &str,
        limit: usize,
    ) -> Result<Vec<StorageEntry>, VrtCError> {
        validate_namespace(namespace)?;
        if limit == 0 || limit > self.config.max_scan_items {
            return Err(storage_error(
                "INVALID_STORAGE_SCAN_LIMIT",
                "The storage scan limit is outside the configured range",
                ErrorCategory::Validation,
                false,
            ));
        }
        let prefix = format!("{namespace}/");
        self.backend
            .scan_prefix(&prefix, limit)?
            .into_iter()
            .map(|(full_key, value)| {
                let key = full_key
                    .strip_prefix(&prefix)
                    .unwrap_or_default()
                    .to_owned();
                Ok(StorageEntry {
                    namespace: namespace.to_owned(),
                    key,
                    value,
                })
            })
            .collect()
    }

    fn storage_key(&self, namespace: &str, key: &str) -> Result<String, VrtCError> {
        validate_namespace(namespace)?;
        if key.trim().is_empty()
            || key.len() > self.config.max_key_bytes
            || key.chars().any(char::is_control)
        {
            return Err(storage_error(
                "INVALID_STORAGE_KEY",
                "Storage key must be non-empty, bounded, and contain no control characters",
                ErrorCategory::Validation,
                false,
            ));
        }
        Ok(format!("{namespace}/{key}"))
    }
}

fn validate_config(config: &StorageConfig) -> Result<(), VrtCError> {
    if config.max_key_bytes == 0
        || config.max_value_bytes == 0
        || config.max_batch_operations == 0
        || config.max_scan_items == 0
    {
        return Err(storage_error(
            "INVALID_STORAGE_CONFIG",
            "Storage key, value, batch, and scan limits must be positive",
            ErrorCategory::Validation,
            false,
        ));
    }
    Ok(())
}

fn validate_namespace(namespace: &str) -> Result<(), VrtCError> {
    if namespace.is_empty()
        || namespace.len() > 128
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(storage_error(
            "INVALID_STORAGE_NAMESPACE",
            "Namespace must use lowercase ASCII letters, digits, dots, underscores, or hyphens",
            ErrorCategory::Validation,
            false,
        ));
    }
    Ok(())
}

fn storage_error(code: &str, message: &str, category: ErrorCategory, retryable: bool) -> VrtCError {
    VrtCError::new(code, message, category, retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn memory_backend_supports_namespaces_atomic_batches_and_scans() {
        let engine = StorageEngine::memory(StorageConfig::default()).unwrap();
        let result = engine
            .apply_batch(vec![
                StorageOperation::Put {
                    namespace: "crawler".to_owned(),
                    key: "page-1".to_owned(),
                    value: b"first".to_vec(),
                },
                StorageOperation::Put {
                    namespace: "index".to_owned(),
                    key: "page-1".to_owned(),
                    value: b"second".to_vec(),
                },
            ])
            .unwrap();
        assert_eq!(result.operations_applied, 2);
        assert_eq!(
            engine.get("crawler", "page-1").unwrap(),
            Some(b"first".to_vec())
        );
        assert_eq!(
            engine.get("index", "page-1").unwrap(),
            Some(b"second".to_vec())
        );
        assert_eq!(engine.scan_namespace("crawler", 10).unwrap().len(), 1);
        engine.delete("crawler", "page-1").unwrap();
        assert_eq!(engine.get("crawler", "page-1").unwrap(), None);
    }

    #[test]
    fn validates_limits_and_namespaces() {
        let engine = StorageEngine::memory(StorageConfig {
            max_value_bytes: 3,
            ..StorageConfig::default()
        })
        .unwrap();
        assert!(engine.put("Bad Namespace", "key", b"x".to_vec()).is_err());
        assert!(engine.put("crawler", "key", b"large".to_vec()).is_err());
        assert!(engine.scan_namespace("crawler", 0).is_err());
    }

    #[test]
    fn redb_backend_persists_values_across_reopen() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vrtc-storage-{}-{suffix}.redb", std::process::id()));
        {
            let engine = StorageEngine::open_redb(&path, StorageConfig::default()).unwrap();
            engine
                .put("crawler", "persisted", b"durable".to_vec())
                .unwrap();
        }
        {
            let engine = StorageEngine::open_redb(&path, StorageConfig::default()).unwrap();
            assert_eq!(
                engine.get("crawler", "persisted").unwrap(),
                Some(b"durable".to_vec())
            );
        }
        let _ = std::fs::remove_file(path);
    }
}
