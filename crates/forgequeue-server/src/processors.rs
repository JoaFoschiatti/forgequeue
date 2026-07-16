use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use forgequeue_core::JobKind;
use image::{DynamicImage, GenericImageView, ImageFormat};
use pdfium_render::prelude::*;
use serde_json::json;
use thiserror::Error;
use uuid::Uuid;

use crate::{config::Config, db::Database, storage::BlobStore};

#[derive(Debug)]
pub struct Artifact {
    pub name: String,
    pub content_type: &'static str,
    pub bytes: Bytes,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub page_number: Option<i32>,
}

#[derive(Debug, Error)]
pub enum ProcessingError {
    #[error("processing was cancelled")]
    Cancelled,
    #[error("processing lease was lost")]
    LeaseLost,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Temporary(String),
}

impl ProcessingError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::LeaseLost => "lease_lost",
            Self::Invalid(_) => "invalid_input",
            Self::Temporary(_) => "processing_failed",
        }
    }

    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Temporary(_))
    }
}

#[derive(Clone)]
pub struct ProcessorContext {
    pub config: Arc<Config>,
    pub db: Database,
    pub storage: BlobStore,
    pdfium: Arc<OnceLock<Result<Pdfium, String>>>,
}

impl ProcessorContext {
    pub fn new(config: Arc<Config>, db: Database, storage: BlobStore) -> Self {
        Self {
            config,
            db,
            storage,
            pdfium: Arc::new(OnceLock::new()),
        }
    }

    pub async fn process(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        kind: JobKind,
        input_key: &str,
        sha256: &str,
    ) -> Result<Vec<Artifact>, ProcessingError> {
        self.check_cancelled(job_id).await?;
        self.update_progress(job_id, attempt_id, worker_id, "loading_input", 12)
            .await?;
        let bytes = self.storage.get(input_key).await.map_err(temporary)?;
        self.check_cancelled(job_id).await?;
        if !self.config.demo_processing_delay.is_zero() {
            self.update_progress(job_id, attempt_id, worker_id, "demo_delay", 18)
                .await?;
            tokio::time::sleep(self.config.demo_processing_delay).await;
            self.check_cancelled(job_id).await?;
        }

        match kind {
            JobKind::Image => {
                self.process_image(job_id, attempt_id, worker_id, bytes, sha256)
                    .await
            }
            JobKind::Pdf => {
                self.process_pdf(job_id, attempt_id, worker_id, bytes, sha256)
                    .await
            }
        }
    }

    async fn process_image(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        bytes: Bytes,
        sha256: &str,
    ) -> Result<Vec<Artifact>, ProcessingError> {
        self.update_progress(job_id, attempt_id, worker_id, "decoding_image", 25)
            .await?;
        let sha256 = sha256.to_owned();
        let artifacts = tokio::task::spawn_blocking(move || {
            let image = image::load_from_memory(&bytes)
                .map_err(|error| ProcessingError::Invalid(format!("Imagen inválida: {error}")))?;
            let (width, height) = image.dimensions();
            let thumbnail = image.thumbnail(320, 320);
            let preview = image.thumbnail(1280, 1280);

            let mut thumbnail_bytes = Cursor::new(Vec::new());
            thumbnail
                .write_to(&mut thumbnail_bytes, ImageFormat::WebP)
                .map_err(|error| ProcessingError::Temporary(error.to_string()))?;
            let mut preview_bytes = Cursor::new(Vec::new());
            preview
                .write_to(&mut preview_bytes, ImageFormat::WebP)
                .map_err(|error| ProcessingError::Temporary(error.to_string()))?;

            let (thumbnail_width, thumbnail_height) = thumbnail.dimensions();
            let (preview_width, preview_height) = preview.dimensions();
            let metadata = serde_json::to_vec_pretty(&json!({
                "kind": "image",
                "sha256": sha256,
                "width": width,
                "height": height,
                "thumbnail": {"width": thumbnail_width, "height": thumbnail_height},
                "preview": {"width": preview_width, "height": preview_height}
            }))
            .map_err(|error| ProcessingError::Temporary(error.to_string()))?;

            Ok::<_, ProcessingError>(vec![
                image_artifact("thumbnail.webp", thumbnail_bytes.into_inner(), &thumbnail),
                image_artifact("preview.webp", preview_bytes.into_inner(), &preview),
                Artifact {
                    name: "metadata.json".to_owned(),
                    content_type: "application/json",
                    bytes: Bytes::from(metadata),
                    width: None,
                    height: None,
                    page_number: None,
                },
            ])
        })
        .await
        .map_err(|error| ProcessingError::Temporary(error.to_string()))??;
        self.update_progress(job_id, attempt_id, worker_id, "image_encoded", 75)
            .await?;
        Ok(artifacts)
    }

    async fn process_pdf(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        bytes: Bytes,
        sha256: &str,
    ) -> Result<Vec<Artifact>, ProcessingError> {
        self.update_progress(job_id, attempt_id, worker_id, "rendering_pdf", 25)
            .await?;
        let preview_pages = self.config.pdf_preview_pages;
        let max_pages = self.config.max_pdf_pages;
        let library_path = self.config.pdfium_library_path.clone();
        let pdfium = self.pdfium.clone();
        let sha256 = sha256.to_owned();
        let artifacts = tokio::task::spawn_blocking(move || {
            render_pdf(
                bytes,
                &sha256,
                preview_pages,
                max_pages,
                library_path,
                pdfium,
            )
        })
        .await
        .map_err(|error| ProcessingError::Temporary(error.to_string()))??;
        self.update_progress(job_id, attempt_id, worker_id, "pdf_rendered", 75)
            .await?;
        Ok(artifacts)
    }

    async fn update_progress(
        &self,
        job_id: Uuid,
        attempt_id: Uuid,
        worker_id: &str,
        stage: &str,
        progress: u8,
    ) -> Result<(), ProcessingError> {
        let owned = self
            .db
            .update_progress(job_id, attempt_id, worker_id, stage, progress)
            .await
            .map_err(temporary)?;
        if owned {
            Ok(())
        } else {
            Err(ProcessingError::LeaseLost)
        }
    }

    async fn check_cancelled(&self, job_id: Uuid) -> Result<(), ProcessingError> {
        if self
            .db
            .is_cancel_requested(job_id)
            .await
            .map_err(temporary)?
        {
            return Err(ProcessingError::Cancelled);
        }
        Ok(())
    }
}

fn image_artifact(name: &str, bytes: Vec<u8>, image: &DynamicImage) -> Artifact {
    let (width, height) = image.dimensions();
    Artifact {
        name: name.to_owned(),
        content_type: "image/webp",
        bytes: Bytes::from(bytes),
        width: i32::try_from(width).ok(),
        height: i32::try_from(height).ok(),
        page_number: None,
    }
}

fn render_pdf(
    bytes: Bytes,
    sha256: &str,
    preview_pages: usize,
    max_pages: usize,
    library_path: Option<PathBuf>,
    pdfium: Arc<OnceLock<Result<Pdfium, String>>>,
) -> Result<Vec<Artifact>, ProcessingError> {
    let pdfium = pdfium
        .get_or_init(|| initialize_pdfium(library_path.as_deref()))
        .as_ref()
        .map_err(|error| ProcessingError::Temporary(error.clone()))?;
    let document = pdfium
        .load_pdf_from_byte_slice(&bytes, None)
        .map_err(|error| ProcessingError::Invalid(format!("PDF inválido: {error}")))?;
    let page_count = usize::try_from(document.pages().len()).map_err(|_| {
        ProcessingError::Invalid("El PDF tiene un conteo de páginas inválido.".to_owned())
    })?;
    if page_count > max_pages {
        return Err(ProcessingError::Invalid(format!(
            "El PDF tiene {page_count} páginas; el máximo es {max_pages}."
        )));
    }

    let mut artifacts = Vec::new();
    let render_config = PdfRenderConfig::new()
        .set_target_width(1024)
        .set_maximum_height(1448);
    for (index, page) in document.pages().iter().take(preview_pages).enumerate() {
        let bitmap = page
            .render_with_config(&render_config)
            .map_err(|error| ProcessingError::Temporary(error.to_string()))?;
        let image = bitmap
            .as_image()
            .map_err(|error| ProcessingError::Temporary(error.to_string()))?;
        let (width, height) = image.dimensions();
        let mut png = Cursor::new(Vec::new());
        image
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|error| ProcessingError::Temporary(error.to_string()))?;
        artifacts.push(Artifact {
            name: format!("page-{}.png", index + 1),
            content_type: "image/png",
            bytes: Bytes::from(png.into_inner()),
            width: i32::try_from(width).ok(),
            height: i32::try_from(height).ok(),
            page_number: i32::try_from(index + 1).ok(),
        });
    }
    let metadata = serde_json::to_vec_pretty(&json!({
        "kind": "pdf",
        "sha256": sha256,
        "page_count": page_count,
        "preview_pages": page_count.min(preview_pages)
    }))
    .map_err(|error| ProcessingError::Temporary(error.to_string()))?;
    artifacts.push(Artifact {
        name: "metadata.json".to_owned(),
        content_type: "application/json",
        bytes: Bytes::from(metadata),
        width: None,
        height: None,
        page_number: None,
    });
    Ok(artifacts)
}

fn initialize_pdfium(library_path: Option<&Path>) -> Result<Pdfium, String> {
    let bindings = match library_path {
        Some(path) => Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(path)),
        None => Pdfium::bind_to_system_library(),
    };

    match bindings {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        // pdfium-render keeps thread-safe bindings in a process-global OnceCell. This branch
        // allows another ProcessorContext in the same process to reuse those bindings safely.
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
        Err(error) => Err(format!("PDFium no está disponible: {error}")),
    }
}

fn temporary(error: impl std::fmt::Display) -> ProcessingError {
    ProcessingError::Temporary(error.to_string())
}

pub fn validate_pdf_page_count(bytes: &[u8], max_pages: usize) -> Result<usize> {
    let document = lopdf::Document::load_mem(bytes).context("PDF structure is invalid")?;
    let pages = document.get_pages().len();
    if pages == 0 {
        return Err(anyhow!("PDF has no pages"));
    }
    if pages > max_pages {
        return Err(anyhow!("PDF has {pages} pages; maximum is {max_pages}"));
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, RgbImage};
    use lopdf::{Document, Object, dictionary};

    use super::{image_artifact, validate_pdf_page_count};

    #[test]
    fn image_artifact_records_dimensions() {
        let image = DynamicImage::ImageRgb8(RgbImage::new(10, 20));
        let artifact = image_artifact("test.webp", vec![1, 2], &image);
        assert_eq!(artifact.width, Some(10));
        assert_eq!(artifact.height, Some(20));
        assert_eq!(artifact.content_type, "image/webp");
    }

    #[test]
    fn pdf_page_limit_rejects_excessive_and_corrupt_documents() {
        let pdf = pdf_with_pages(3);
        assert_eq!(validate_pdf_page_count(&pdf, 3).unwrap(), 3);
        assert!(validate_pdf_page_count(&pdf, 2).is_err());
        assert!(validate_pdf_page_count(b"%PDF-1.4\ncorrupt", 20).is_err());
    }

    fn pdf_with_pages(count: usize) -> Vec<u8> {
        let mut document = Document::with_version("1.4");
        let pages_id = document.new_object_id();
        let page_ids = (0..count)
            .map(|_| {
                document.add_object(dictionary! {
                    "Type" => "Page",
                    "Parent" => pages_id,
                    "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                })
            })
            .collect::<Vec<_>>();
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => page_ids.iter().copied().map(Object::Reference).collect::<Vec<_>>(),
                "Count" => count as i64,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }
}
