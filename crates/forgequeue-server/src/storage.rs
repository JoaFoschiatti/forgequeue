use std::{path::Path as FilePath, sync::Arc};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use futures_util::TryStreamExt;
use object_store::{
    ObjectStore, ObjectStoreExt, PutPayload, aws::AmazonS3Builder, local::LocalFileSystem,
    memory::InMemory, path::Path,
};
use url::Url;

use crate::config::Config;

#[derive(Clone)]
pub struct BlobStore {
    inner: Arc<dyn ObjectStore>,
}

impl BlobStore {
    pub async fn from_config(config: &Config) -> Result<Self> {
        let url = Url::parse(&config.object_store_url)
            .with_context(|| "OBJECT_STORE_URL must be a valid URL")?;

        let inner: Arc<dyn ObjectStore> = match url.scheme() {
            "memory" => Arc::new(InMemory::new()),
            "file" => {
                let path = if url.path().is_empty() || url.path() == "/" {
                    config.object_store_path.clone()
                } else {
                    let raw = url.path().trim_start_matches('/');
                    if cfg!(windows) && raw.len() > 2 && raw.as_bytes()[1] == b':' {
                        raw.into()
                    } else {
                        FilePath::new(url.path()).into()
                    }
                };
                tokio::fs::create_dir_all(&path).await.with_context(|| {
                    format!("failed to create object store at {}", path.display())
                })?;
                Arc::new(LocalFileSystem::new_with_prefix(path)?)
            }
            "s3" => {
                let bucket = url
                    .host_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| anyhow!("s3 OBJECT_STORE_URL must contain a bucket"))?;
                let access_key = config
                    .s3_access_key_id
                    .as_deref()
                    .context("S3_ACCESS_KEY_ID is required for S3 storage")?;
                let secret_key = config
                    .s3_secret_access_key
                    .as_deref()
                    .context("S3_SECRET_ACCESS_KEY is required for S3 storage")?;
                let mut builder = AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(&config.s3_region)
                    .with_access_key_id(access_key)
                    .with_secret_access_key(secret_key)
                    .with_virtual_hosted_style_request(false);
                if let Some(endpoint) = &config.s3_endpoint {
                    builder = builder
                        .with_endpoint(endpoint)
                        .with_allow_http(endpoint.starts_with("http://"));
                }
                Arc::new(builder.build()?)
            }
            scheme => return Err(anyhow!("unsupported object store scheme: {scheme}")),
        };

        Ok(Self { inner })
    }

    pub async fn put(&self, key: &str, bytes: Bytes) -> Result<(), object_store::Error> {
        self.inner
            .put(&Path::from(key), PutPayload::from_bytes(bytes))
            .await?;
        Ok(())
    }

    pub async fn get(&self, key: &str) -> Result<Bytes, object_store::Error> {
        self.inner.get(&Path::from(key)).await?.bytes().await
    }

    pub async fn delete(&self, key: &str) -> Result<(), object_store::Error> {
        match self.inner.delete(&Path::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn delete_prefix(&self, prefix: &str) -> Result<(), object_store::Error> {
        let objects = self
            .inner
            .list(Some(&Path::from(prefix)))
            .try_collect::<Vec<_>>()
            .await?;
        for object in objects {
            self.delete(object.location.as_ref()).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use object_store::memory::InMemory;

    use super::BlobStore;

    #[tokio::test]
    async fn prefix_cleanup_is_scoped_to_one_job() {
        let storage = BlobStore {
            inner: Arc::new(InMemory::new()),
        };
        storage
            .put("sessions/s/jobs/a/input", Bytes::from_static(b"input-a"))
            .await
            .unwrap();
        storage
            .put(
                "sessions/s/jobs/a/outputs/preview.webp",
                Bytes::from_static(b"preview-a"),
            )
            .await
            .unwrap();
        storage
            .put("sessions/s/jobs/b/input", Bytes::from_static(b"input-b"))
            .await
            .unwrap();

        storage.delete_prefix("sessions/s/jobs/a").await.unwrap();

        assert!(storage.get("sessions/s/jobs/a/input").await.is_err());
        assert!(
            storage
                .get("sessions/s/jobs/a/outputs/preview.webp")
                .await
                .is_err()
        );
        assert_eq!(
            storage.get("sessions/s/jobs/b/input").await.unwrap(),
            Bytes::from_static(b"input-b")
        );
    }
}
