//! Storage backend abstraction (filesystem + S3).
//!
//! Manifest layout:
//!   <root>/manifest.json                       — current pointer (overwritten atomically)
//!   <root>/catalogs/<version_id>.json          — versioned immutable catalog
//!
//! S3 PUT/DELETE provide strong read-after-write consistency in all regions
//! (since 2020). Manifest replacement is atomic from a reader's perspective
//! (old-or-new, never partial). Versioned catalog objects are immutable
//! once written; new versions get new keys, so concurrent readers that
//! pulled the prior manifest still resolve to a valid catalog.
//!
//! Multiple concurrent publishers are NOT prevented by the storage layer.
//! Operators MUST run publishes through a single-writer pipeline (e.g.,
//! a serialized GHA workflow). Phase 2+ should add If-None-Match
//! conditional create on `catalogs/*.json` and a CAS on `manifest.json`.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;

use crate::config::StorageConfig;

#[async_trait]
pub trait Store: Send + Sync {
    /// Read raw bytes by key path (relative to root/prefix).
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// Write raw bytes; backends MUST make the write atomic from a reader's
    /// perspective (filesystem: write tmp + rename; S3: PUT with strong
    /// read-your-writes consistency in the same region).
    async fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<()>;

    /// List keys with a prefix (for listing catalog versions).
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
}

pub fn make_store(cfg: &StorageConfig) -> Result<std::sync::Arc<dyn Store>> {
    match cfg.storage_backend.as_str() {
        "filesystem" => {
            let root = cfg
                .filesystem_root
                .as_deref()
                .ok_or_else(|| anyhow!("filesystem_root required when storage_backend=filesystem"))?;
            Ok(std::sync::Arc::new(FilesystemStore::new(root)?))
        }
        "s3" => {
            let bucket = cfg
                .s3_bucket
                .clone()
                .ok_or_else(|| anyhow!("s3_bucket required when storage_backend=s3"))?;
            // S3 client built lazily once we are on a runtime.
            Ok(std::sync::Arc::new(S3Store {
                bucket,
                prefix: cfg.s3_prefix.clone(),
                region: cfg.region.clone(),
                client: tokio::sync::OnceCell::new(),
            }))
        }
        other => Err(anyhow!("unknown storage_backend: {}", other)),
    }
}

// ============================================================================
// Filesystem store (POC default; tmp+rename atomicity)
// ============================================================================

pub struct FilesystemStore {
    root: std::path::PathBuf,
}

impl FilesystemStore {
    pub fn new(root: &str) -> Result<Self> {
        let root = std::path::PathBuf::from(root);
        std::fs::create_dir_all(&root).with_context(|| format!("mkdir {}", root.display()))?;
        Ok(Self { root })
    }

    fn path(&self, key: &str) -> std::path::PathBuf {
        self.root.join(key)
    }
}

#[async_trait]
impl Store for FilesystemStore {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let p = self.path(key);
        match tokio::fs::read(&p).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
        }
    }

    async fn put(&self, key: &str, body: &[u8], _content_type: &str) -> Result<()> {
        let p = self.path(key);
        if let Some(parent) = p.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Atomic via tmp + rename in same directory.
        let tmp = p.with_extension(format!(
            "{}.tmp.{}",
            p.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("dat"),
            uuid::Uuid::now_v7()
        ));
        tokio::fs::write(&tmp, body)
            .await
            .with_context(|| format!("write tmp {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &p)
            .await
            .with_context(|| format!("rename {} → {}", tmp.display(), p.display()))?;
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let prefix_path = self.path(prefix);
        let mut out = Vec::new();
        let mut rd = match tokio::fs::read_dir(&prefix_path).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = rd.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                out.push(format!("{}{}", prefix, name));
            }
        }
        Ok(out)
    }
}

// ============================================================================
// S3 store
// ============================================================================

pub struct S3Store {
    bucket: String,
    prefix: String,
    region: String,
    client: tokio::sync::OnceCell<aws_sdk_s3::Client>,
}

impl S3Store {
    async fn client(&self) -> &aws_sdk_s3::Client {
        self.client
            .get_or_init(|| async {
                let cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(aws_config::Region::new(self.region.clone()))
                    .load()
                    .await;
                aws_sdk_s3::Client::new(&cfg)
            })
            .await
    }

    fn key(&self, k: &str) -> String {
        format!("{}{}", self.prefix, k)
    }
}

#[async_trait]
impl Store for S3Store {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let client = self.client().await;
        let resp = client
            .get_object()
            .bucket(&self.bucket)
            .key(self.key(key))
            .send()
            .await;
        match resp {
            Ok(o) => {
                let bytes = o.body.collect().await?.into_bytes().to_vec();
                Ok(Some(bytes))
            }
            Err(e) => {
                // Map "no such key" to None.
                let msg = format!("{e:?}");
                if msg.contains("NoSuchKey") || msg.contains("NotFound") {
                    return Ok(None);
                }
                Err(anyhow!("s3 get: {}", e))
            }
        }
    }

    async fn put(&self, key: &str, body: &[u8], content_type: &str) -> Result<()> {
        let client = self.client().await;
        let body_owned = body.to_vec();
        client
            .put_object()
            .bucket(&self.bucket)
            .key(self.key(key))
            .body(aws_sdk_s3::primitives::ByteStream::from(body_owned))
            .content_type(content_type)
            .send()
            .await
            .with_context(|| format!("s3 put {}", key))?;
        Ok(())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let client = self.client().await;
        let resp = client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(self.key(prefix))
            .send()
            .await
            .context("s3 list")?;
        let prefix_full = self.key(prefix);
        Ok(resp
            .contents()
            .iter()
            .filter_map(|o| o.key().map(|k| k.trim_start_matches(&prefix_full).to_string()))
            .map(|name| format!("{}{}", prefix, name))
            .collect())
    }
}
