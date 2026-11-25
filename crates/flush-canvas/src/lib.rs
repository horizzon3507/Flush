//! flush-canvas: card builder and image tooling
//! (mirrors yuzuha `canvas.ts`, `canvas/card.ts` and `images.ts`).
//!
//! - [`CanvasService`] / [`create_card`]: fluent scene builder with `rect`,
//!   `circle`, `line`, `image`, `avatar`, `text`, `badges` and `progress`,
//!   rendered to PNG/JPEG/WebP.
//! - [`Canvas`] / [`create_canvas`] / [`load_image`]: the raw
//!   `createCanvas` + `loadImage` surface, with `tiny_skia` re-exported for
//!   direct drawing (the `@napi-rs/canvas` counterpart).
//! - [`transform_image`]: sharp-style resize/rotate/grayscale/re-encode.
//! - [`inspect_image`], [`validate_color`], [`validate_fill`],
//!   [`validate_font_bytes`], [`CANVAS_LIMITS`]: the shared validators.

pub mod error;
pub mod types;
mod builder;
mod image;
mod render;

pub use builder::{
    clear_fonts, create_canvas, create_card, load_image, register_fonts, Canvas, CanvasConfig,
    CanvasImage, CanvasService, Card, SendOptions,
};
pub use error::CanvasError;
pub use image::{transform_image, ImageResult, ImageTransform, TransformFit, TransformOptions};
pub use types::*;

pub use tiny_skia;
pub use tiny_skia_path;
