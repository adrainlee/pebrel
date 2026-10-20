//! Document resource loading with bounded reads and first-frame raster decoding.
use crate::gpui_shell::image_decode;
use futures::AsyncReadExt;
use gpui::{App, ImageCacheError, RenderImage, Resource};
use std::sync::Arc;

// Process-wide admission, shared across documents; no worker waits on a mutex
// while holding a terminal/UI lock. Cancellation releases an acquired permit.
static LOADS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

pub(super) fn load(
    resource: Resource,
    byte_limit: usize,
    cx: &mut App,
) -> impl std::future::Future<Output = Result<Arc<RenderImage>, ImageCacheError>> + Send + 'static {
    let client = cx.http_client();
    let assets = cx.asset_source().clone();
    let svg = cx.svg_renderer();
    async move {
        let _permit = LOADS.acquire().await.expect("image admission never closes");
        let bytes = match resource {
            Resource::Path(path) => image_decode::read_file(&path)?,
            Resource::Uri(uri) => {
                let mut response = client.get(uri.as_ref(), ().into(), true).await?;
                if !response.status().is_success() {
                    return Err(ImageCacheError::BadStatus {
                        uri,
                        status: response.status(),
                        body: String::new(),
                    });
                }
                let mut bytes = Vec::new();
                response
                    .body_mut()
                    .take(image_decode::MAX_ENCODED_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await?;
                bytes
            },
            Resource::Embedded(path) => assets
                .load(&path)?
                .ok_or_else(|| ImageCacheError::Asset("embedded image is missing".into()))?
                .into_owned(),
        };
        if bytes.len() > image_decode::MAX_ENCODED_BYTES {
            return Err(image_decode::limit_error().into());
        }
        if let Ok(format) = image::guess_format(&bytes) {
            let mut rgba =
                image_decode::rgba(&bytes, format, image_decode::MAX_ENCODED_BYTES, byte_limit)?;
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            Ok(Arc::new(RenderImage::new([image::Frame::new(rgba)])))
        } else {
            // SVG remains on GPUI's native renderer (which caps either dimension
            // at 8192); keep its encoded input small and share global admission.
            if bytes.len() > 1024 * 1024 {
                return Err(image_decode::limit_error().into());
            }
            svg.render_single_frame(&bytes, 1.0).map_err(Into::into)
        }
    }
}
