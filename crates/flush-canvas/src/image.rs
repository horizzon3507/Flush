//! `transformImage` and decoding helpers (mirrors `infrastructure/images`).

use image::{DynamicImage, ImageFormat};

use crate::error::CanvasError;
use crate::types::{inspect_image, CanvasFormat, CANVAS_LIMITS};

/// One resize/rotate/convert step, mirroring `ImageTransform`.
#[derive(Debug, Clone, Default)]
pub struct ImageTransform {
    pub format: Option<CanvasFormat>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fit: Option<TransformFit>,
    pub rotate: Option<u32>,
    pub grayscale: Option<bool>,
    pub quality: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformFit {
    Inside,
    Contain,
    Cover,
}

#[derive(Debug, Clone, Default)]
pub struct TransformOptions {
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ImageResult {
    pub data: Vec<u8>,
    pub format: CanvasFormat,
    pub width: u32,
    pub height: u32,
}

fn validate_transform(t: &ImageTransform) -> Result<(), CanvasError> {
    if let Some(f) = t.fit {
        let _ = f;
    }
    if let Some(r) = t.rotate {
        if ![0, 90, 180, 270].contains(&r) {
            return Err(CanvasError::Options(
                "Invalid transform; dimensions 1-4096, quality 1-100.".into(),
            ));
        }
    }
    if let Some(q) = t.quality {
        if !(1..=100).contains(&q) {
            return Err(CanvasError::Options(
                "Invalid transform; dimensions 1-4096, quality 1-100.".into(),
            ));
        }
    }
    for size in [t.width, t.height].into_iter().flatten() {
        if !(1..=4096).contains(&size) {
            return Err(CanvasError::Options(
                "Invalid transform; dimensions 1-4096, quality 1-100.".into(),
            ));
        }
    }
    Ok(())
}

/// Decodes bytes into a `DynamicImage` after the shared header validation.
pub(crate) fn decode(bytes: &[u8]) -> Result<DynamicImage, CanvasError> {
    let info = inspect_image(bytes)?;
    let format = match info.format {
        CanvasFormat::Png => ImageFormat::Png,
        CanvasFormat::Jpeg => ImageFormat::Jpeg,
        CanvasFormat::WebP => ImageFormat::WebP,
    };
    image::load_from_memory_with_format(bytes, format)
        .map_err(|e| CanvasError::Input(format!("Failed to decode image: {e}")))
}

pub(crate) fn encode(
    img: &DynamicImage,
    format: CanvasFormat,
    quality: u32,
) -> Result<Vec<u8>, CanvasError> {
    use image::ImageEncoder;
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());
    let mut out = Vec::new();
    match format {
        CanvasFormat::Png => {
            image::codecs::png::PngEncoder::new(&mut out)
                .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        }
        CanvasFormat::Jpeg => {
            // JPEG has no alpha: flatten onto an opaque background first.
            let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality as u8)
                .write_image(&rgb, w, h, image::ExtendedColorType::Rgb8)
        }
        CanvasFormat::WebP => {
            // The `image` crate encodes WebP losslessly; `quality` is ignored.
            let _ = quality;
            image::codecs::webp::WebPEncoder::new_lossless(&mut out)
                .write_image(&rgba, w, h, image::ExtendedColorType::Rgba8)
        }
    }
    .map_err(|e| CanvasError::Process(format!("Failed to encode image: {e}")))?;
    Ok(out)
}

/// `transformImage`: validates, decodes, applies rotate → resize → grayscale,
/// and re-encodes. Runs inline; the timeout bound mirrors the 10-30000 ms rule.
pub async fn transform_image(
    data: &[u8],
    transform: &ImageTransform,
    options: &TransformOptions,
) -> Result<ImageResult, CanvasError> {
    if data.is_empty() || data.len() > CANVAS_LIMITS.max_item_bytes {
        return Err(CanvasError::Input(
            "Image must be between 1 byte and 10 MB.".into(),
        ));
    }
    validate_transform(transform)?;
    let timeout_ms = options.timeout_ms.unwrap_or(5000);
    if !(CANVAS_LIMITS.min_timeout_ms..=CANVAS_LIMITS.max_timeout_ms).contains(&timeout_ms) {
        return Err(CanvasError::Options("Timeout must be 10 to 30000 ms.".into()));
    }
    let data = data.to_vec();
    let transform = transform.clone();
    async_std::future::timeout(
        std::time::Duration::from_millis(timeout_ms),
        async_std::task::spawn_blocking(move || -> Result<ImageResult, CanvasError> {
            let mut img = decode(&data)?;
            match transform.rotate.unwrap_or(0) {
                90 => img = img.rotate90(),
                180 => img = img.rotate180(),
                270 => img = img.rotate270(),
                _ => {}
            }
            if transform.width.is_some() || transform.height.is_some() {
                let w = transform.width.unwrap_or(img.width());
                let h = transform.height.unwrap_or(img.height());
                img = match transform.fit.unwrap_or(TransformFit::Inside) {
                    // sharp `inside`: shrink to fit, never enlarge.
                    TransformFit::Inside => {
                        img.resize(w.min(img.width()), h.min(img.height()), image::imageops::FilterType::Lanczos3)
                    }
                    // sharp `contain`: fit inside the box, padded, never enlarged.
                    TransformFit::Contain => {
                        let inner = img.resize(
                            w.min(img.width()),
                            h.min(img.height()),
                            image::imageops::FilterType::Lanczos3,
                        );
                        let mut canvas = image::RgbaImage::new(w, h);
                        let x = ((w.saturating_sub(inner.width())) / 2) as i64;
                        let y = ((h.saturating_sub(inner.height())) / 2) as i64;
                        image::imageops::overlay(&mut canvas, &inner.to_rgba8(), x, y);
                        DynamicImage::ImageRgba8(canvas)
                    }
                    TransformFit::Cover => {
                        img.resize_to_fill(w, h, image::imageops::FilterType::Lanczos3)
                    }
                };
            }
            if transform.grayscale.unwrap_or(false) {
                img = DynamicImage::ImageLuma8(img.to_luma8());
            }
            let format = transform.format.unwrap_or(CanvasFormat::Png);
            let quality = transform.quality.unwrap_or(80);
            let out = encode(&img, format, quality)?;
            if out.len() > CANVAS_LIMITS.max_item_bytes {
                return Err(CanvasError::ExecutionLimit("Output exceeds 10 MB.".into()));
            }
            Ok(ImageResult { data: out, format, width: img.width(), height: img.height() })
        }),
    )
    .await
    .map_err(|_| CanvasError::ExecutionLimit("Image exceeded the time limit.".into()))?
}

/// `loadImage` for raw canvases: validates and decodes `bytes` (which callers
/// fetch or read), returning RGBA pixels ready for `draw_pixmap`.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<image::RgbaImage, CanvasError> {
    Ok(decode(bytes)?.to_rgba8())
}
