use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub worker_metrics_address: Option<SocketAddr>,
    pub database_url: String,
    pub object_store_url: String,
    pub object_store_path: PathBuf,
    pub s3_endpoint: Option<String>,
    pub s3_region: String,
    pub s3_access_key_id: Option<String>,
    pub s3_secret_access_key: Option<String>,
    pub cors_origin: String,
    pub trust_proxy_headers: bool,
    pub rate_limit_salt: String,
    pub worker_id: String,
    pub lease_duration: Duration,
    pub heartbeat_interval: Duration,
    pub processing_timeout: Duration,
    pub poll_interval: Duration,
    pub demo_processing_delay: Duration,
    pub artifact_retention: Duration,
    pub metadata_retention: Duration,
    pub max_upload_bytes: usize,
    pub max_image_pixels: u64,
    pub max_pdf_pages: usize,
    pub pdf_preview_pages: usize,
    pub session_hourly_limit: i64,
    pub ip_daily_limit: i64,
    pub global_daily_limit: i64,
    pub pdfium_library_path: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let hostname = env::var("HOSTNAME").unwrap_or_else(|_| "local".to_owned());
        let worker_id =
            env::var("WORKER_ID").unwrap_or_else(|_| format!("{hostname}-{}", std::process::id()));
        let worker_metrics_address = optional("WORKER_METRICS_ADDRESS")
            .map(|value| value.parse())
            .transpose()
            .context("WORKER_METRICS_ADDRESS must be a socket address")?;

        let config = Self {
            bind_address: value("BIND_ADDRESS", "0.0.0.0:8080")
                .parse()
                .context("BIND_ADDRESS must be a socket address")?,
            worker_metrics_address,
            database_url: required("DATABASE_URL")?,
            object_store_url: value("OBJECT_STORE_URL", "file:///"),
            object_store_path: PathBuf::from(value("OBJECT_STORE_PATH", "./data")),
            s3_endpoint: optional("S3_ENDPOINT"),
            s3_region: value("S3_REGION", "us-east-1"),
            s3_access_key_id: optional("S3_ACCESS_KEY_ID")
                .or_else(|| optional("AWS_ACCESS_KEY_ID")),
            s3_secret_access_key: optional("S3_SECRET_ACCESS_KEY")
                .or_else(|| optional("AWS_SECRET_ACCESS_KEY")),
            cors_origin: value("CORS_ORIGIN", "http://localhost:5173"),
            trust_proxy_headers: boolean("TRUST_PROXY_HEADERS", false)?,
            rate_limit_salt: value("RATE_LIMIT_SALT", "forgequeue-local-only-change-me"),
            worker_id,
            lease_duration: seconds("LEASE_SECONDS", 60)?,
            heartbeat_interval: seconds("HEARTBEAT_SECONDS", 15)?,
            processing_timeout: seconds("PROCESSING_TIMEOUT_SECONDS", 90)?,
            poll_interval: millis("WORKER_POLL_MILLISECONDS", 750)?,
            demo_processing_delay: millis("DEMO_PROCESSING_DELAY_MILLISECONDS", 0)?,
            artifact_retention: seconds("ARTIFACT_RETENTION_SECONDS", 3600)?,
            metadata_retention: seconds("METADATA_RETENTION_SECONDS", 86400)?,
            max_upload_bytes: number("MAX_UPLOAD_BYTES", 10 * 1024 * 1024)?,
            max_image_pixels: number("MAX_IMAGE_PIXELS", 25_000_000)?,
            max_pdf_pages: number("MAX_PDF_PAGES", 20)?,
            pdf_preview_pages: number("PDF_PREVIEW_PAGES", 3)?,
            session_hourly_limit: number("SESSION_HOURLY_LIMIT", 5)?,
            ip_daily_limit: number("IP_DAILY_LIMIT", 20)?,
            global_daily_limit: number("GLOBAL_DAILY_LIMIT", 50)?,
            pdfium_library_path: optional("PDFIUM_LIBRARY_PATH").map(PathBuf::from),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        self.cors_origin
            .parse::<axum::http::HeaderValue>()
            .context("CORS_ORIGIN must be a valid HTTP header value")?;
        if self.worker_id.trim().is_empty() {
            bail!("WORKER_ID cannot be empty");
        }
        if self.lease_duration.is_zero() {
            bail!("LEASE_SECONDS must be greater than zero");
        }
        if self.heartbeat_interval.is_zero() || self.heartbeat_interval >= self.lease_duration {
            bail!("HEARTBEAT_SECONDS must be greater than zero and shorter than LEASE_SECONDS");
        }
        if self.processing_timeout.is_zero() || self.poll_interval.is_zero() {
            bail!("processing timeout and worker poll interval must be greater than zero");
        }
        if self.demo_processing_delay >= self.processing_timeout {
            bail!("DEMO_PROCESSING_DELAY_MILLISECONDS must be shorter than the processing timeout");
        }
        if self.max_upload_bytes == 0 || self.max_image_pixels == 0 || self.max_pdf_pages == 0 {
            bail!("upload and document limits must be greater than zero");
        }
        if self.pdf_preview_pages == 0 || self.pdf_preview_pages > self.max_pdf_pages {
            bail!("PDF_PREVIEW_PAGES must be between one and MAX_PDF_PAGES");
        }
        if self.session_hourly_limit <= 0
            || self.ip_daily_limit <= 0
            || self.global_daily_limit <= 0
        {
            bail!("rate limits must be greater than zero");
        }
        Ok(())
    }
}

fn value(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn optional(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("missing required environment variable {name}"))
}

fn number<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr + std::fmt::Display + Copy,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value(name, &default.to_string())
        .parse()
        .with_context(|| format!("{name} must be a number"))
}

fn seconds(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_secs(number(name, default)?))
}

fn millis(name: &str, default: u64) -> Result<Duration> {
    Ok(Duration::from_millis(number(name, default)?))
}

fn boolean(name: &str, default: bool) -> Result<bool> {
    value(name, if default { "true" } else { "false" })
        .parse()
        .with_context(|| format!("{name} must be true or false"))
}
