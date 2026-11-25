//! Scene domain types and validation (mirrors `domain/canvas.ts`).

use crate::error::CanvasError;

/// Encoded output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanvasFormat {
    Png,
    Jpeg,
    WebP,
}

impl CanvasFormat {
    pub fn mime(&self) -> &'static str {
        match self {
            CanvasFormat::Png => "image/png",
            CanvasFormat::Jpeg => "image/jpeg",
            CanvasFormat::WebP => "image/webp",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
        }
    }
}

/// Image/font input for a scene: raw bytes, an `asset:name` alias or an
/// HTTP/HTTPS URL fetched during resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanvasSource {
    Bytes(Vec<u8>),
    Asset(String),
    Url(String),
}

impl CanvasSource {
    /// Parses a string the way yuzuha does: `asset:<name>` aliases and
    /// HTTP/HTTPS URLs are allowed, nothing else.
    pub fn parse(text: impl Into<String>) -> Result<Self, CanvasError> {
        let text = text.into();
        if text.is_empty() || text.len() > 4096 {
            return Err(CanvasError::Input("Source is invalid.".into()));
        }
        if let Some(name) = text.strip_prefix("asset:") {
            if name.is_empty() || name.len() > 64
                || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            {
                return Err(CanvasError::Input("Invalid asset alias.".into()));
            }
            return Ok(Self::Asset(name.to_string()));
        }
        let url = url::Url::parse(&text)
            .map_err(|_| CanvasError::Input("Source must be bytes, asset:name or an HTTP/HTTPS URL.".into()))?;
        if (url.scheme() != "http" && url.scheme() != "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(CanvasError::Input(
                "Source must use an HTTP/HTTPS URL without credentials.".into(),
            ));
        }
        Ok(Self::Url(text))
    }

    /// From raw bytes; enforces the 1 byte..10 MB window.
    pub fn bytes(data: impl Into<Vec<u8>>) -> Result<Self, CanvasError> {
        let data = data.into();
        if data.is_empty() {
            return Err(CanvasError::Input("Source must contain bytes.".into()));
        }
        if data.len() > CANVAS_LIMITS.max_item_bytes {
            return Err(CanvasError::ExecutionLimit("Source exceeds 10 MB.".into()));
        }
        Ok(Self::Bytes(data))
    }

    pub fn is_asset_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }

    /// Stable cache key, like `canvasSourceKey`.
    pub fn key(&self) -> String {
        match self {
            Self::Bytes(b) => format!("b:{:016x}", hash_bytes(b)),
            Self::Asset(n) => format!("s:asset:{n}"),
            Self::Url(u) => format!("s:{u}"),
        }
    }
}

impl From<Vec<u8>> for CanvasSource {
    fn from(data: Vec<u8>) -> Self {
        Self::Bytes(data)
    }
}
impl From<&[u8]> for CanvasSource {
    fn from(data: &[u8]) -> Self {
        Self::Bytes(data.to_vec())
    }
}
impl From<&str> for CanvasSource {
    fn from(text: &str) -> Self {
        Self::parse(text).unwrap_or(Self::Asset(String::new()))
    }
}
impl From<String> for CanvasSource {
    fn from(text: String) -> Self {
        Self::parse(&text).unwrap_or(Self::Asset(String::new()))
    }
}

/// 64-bit FNV-1a hash for source keys (replaces sha256 as a cache key only).
fn hash_bytes(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub type CanvasColor = String;

#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    pub offset: f64,
    pub color: CanvasColor,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub stops: Vec<GradientStop>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    pub x0: f64,
    pub y0: f64,
    pub r0: f64,
    pub x1: f64,
    pub y1: f64,
    pub r1: f64,
    pub stops: Vec<GradientStop>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CanvasFill {
    Color(CanvasColor),
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl From<String> for CanvasFill {
    fn from(color: String) -> Self {
        Self::Color(color)
    }
}
impl From<&str> for CanvasFill {
    fn from(color: &str) -> Self {
        Self::Color(color.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanvasShadow {
    pub color: CanvasColor,
    pub blur: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PaintOptions {
    pub opacity: Option<f64>,
    pub shadow: Option<CanvasShadow>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateOptions {
    pub scale: Option<u32>,
    pub background: Option<CanvasColor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FontWeight {
    Regular,
    Bold,
}

impl FontWeight {
    pub fn as_u16(&self) -> u16 {
        match self {
            Self::Regular => 400,
            Self::Bold => 700,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RectOptions {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub fill: Option<CanvasFill>,
    pub stroke: Option<CanvasColor>,
    pub stroke_width: Option<f64>,
    pub radius: Option<f64>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Default)]
pub struct CircleOptions {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub fill: Option<CanvasFill>,
    pub stroke: Option<CanvasColor>,
    pub stroke_width: Option<f64>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Default)]
pub struct LineOptions {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub width: Option<f64>,
    pub color: Option<CanvasColor>,
    pub cap: Option<LineCap>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFit {
    Cover,
    Contain,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageClip {
    Rect,
    Rounded,
    Circle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Border {
    pub color: CanvasColor,
    pub width: f64,
}

#[derive(Debug, Clone, Default)]
pub struct ImageOptions {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub fit: Option<ImageFit>,
    pub crop_x: Option<f64>,
    pub crop_y: Option<f64>,
    pub clip: Option<ImageClip>,
    pub radius: Option<f64>,
    pub border: Option<Border>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Default)]
pub struct AvatarOptions {
    pub x: f64,
    pub y: f64,
    pub size: f64,
    pub border: Option<Border>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextWrap {
    None,
    Word,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOverflow {
    Ellipsis,
    Clip,
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct TextOptions {
    pub x: f64,
    pub y: f64,
    pub max_width: Option<f64>,
    pub font_size: Option<f64>,
    pub min_font_size: Option<f64>,
    pub family: Option<String>,
    pub weight: Option<FontWeight>,
    pub color: Option<CanvasColor>,
    pub align: Option<TextAlign>,
    pub wrap: Option<TextWrap>,
    pub max_lines: Option<u32>,
    pub line_height: Option<f64>,
    pub overflow: Option<TextOverflow>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone)]
pub struct BadgeItem {
    pub text: String,
    pub icon: Option<CanvasSource>,
    pub selected: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct BadgesOptions {
    pub x: f64,
    pub y: f64,
    pub max_width: f64,
    pub height: Option<f64>,
    pub gap: Option<f64>,
    pub padding_x: Option<f64>,
    pub radius: Option<f64>,
    pub fill: Option<CanvasColor>,
    pub selected_fill: Option<CanvasColor>,
    pub color: Option<CanvasColor>,
    pub selected_color: Option<CanvasColor>,
    pub font_size: Option<f64>,
    pub weight: Option<FontWeight>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone, Default)]
pub struct ProgressOptions {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub value: f64,
    pub max: f64,
    pub fill: Option<CanvasFill>,
    pub background: Option<CanvasFill>,
    pub radius: Option<f64>,
    pub paint: PaintOptions,
}

#[derive(Debug, Clone)]
pub(crate) enum Operation {
    Rect(RectOptions),
    Circle(CircleOptions),
    Line(LineOptions),
    Image(CanvasSource, ImageOptions),
    Avatar(CanvasSource, AvatarOptions),
    Text(String, TextOptions),
    Badges(Vec<BadgeItem>, BadgesOptions),
    Progress(ProgressOptions),
}

/// A configured font family: `regular` is required, `bold` optional.
#[derive(Debug, Clone)]
pub struct ConfiguredFont {
    pub regular: CanvasSource,
    pub bold: Option<CanvasSource>,
}

/// Resolved font attached to a scene.
#[derive(Debug, Clone)]
pub(crate) struct FontSource {
    pub family: String,
    pub source: CanvasSource,
    pub weight: FontWeight,
}

/// Fully built scene, sent to the renderer.
#[derive(Debug)]
pub(crate) struct Scene {
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub background: Option<CanvasColor>,
    pub operations: Vec<Operation>,
    pub fonts: Vec<FontSource>,
}

#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    pub format: Option<CanvasFormat>,
    pub quality: Option<u32>,
    pub matte: Option<CanvasColor>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct CanvasResult {
    pub data: Vec<u8>,
    pub format: CanvasFormat,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct ImageDimensions {
    pub width: u32,
    pub height: u32,
    pub format: CanvasFormat,
}

/// Hard limits, mirroring `CANVAS_LIMITS`.
#[derive(Debug, Clone, Copy)]
pub struct CanvasLimits {
    pub max_physical_dimension: u32,
    pub max_pixels: u64,
    pub max_input_dimension: u32,
    pub max_operations: u32,
    pub max_images: usize,
    pub max_font_files: usize,
    pub max_item_bytes: usize,
    pub max_scene_asset_bytes: usize,
    pub max_text_length: usize,
    pub max_scene_text_length: usize,
    pub max_badges: usize,
    pub max_output_bytes: usize,
    pub default_timeout_ms: u64,
    pub min_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub max_script_canvases: u32,
    pub max_script_renders: u32,
}

pub const CANVAS_LIMITS: CanvasLimits = CanvasLimits {
    max_physical_dimension: 4096,
    max_pixels: 16_777_216,
    max_input_dimension: 8192,
    max_operations: 512,
    max_images: 64,
    max_font_files: 8,
    max_item_bytes: 10_000_000,
    max_scene_asset_bytes: 20_000_000,
    max_text_length: 4096,
    max_scene_text_length: 32_768,
    max_badges: 32,
    max_output_bytes: 10_000_000,
    default_timeout_ms: 5000,
    min_timeout_ms: 10,
    max_timeout_ms: 30_000,
    max_script_canvases: 4,
    max_script_renders: 4,
};

pub(crate) fn finite(v: f64, label: &str, min: f64, max: f64) -> Result<f64, CanvasError> {
    if !v.is_finite() || v < min || v > max {
        return Err(CanvasError::Options(format!("Invalid {label}.")));
    }
    Ok(v)
}

/// Validates `#rgb(a)`/`#rrggbb(aa)` and `rgb()/rgba()` strings.
pub fn validate_color(value: &str, label: &str) -> Result<(), CanvasError> {
    let c = value.trim();
    let hex = c.strip_prefix('#');
    if let Some(h) = hex {
        if matches!(h.len(), 3 | 4 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Ok(());
        }
    }
    let inner = c
        .strip_prefix("rgba(")
        .or_else(|| c.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'));
    if let Some(body) = inner.filter(|_| {
        c.to_ascii_lowercase().starts_with("rgb(") || c.to_ascii_lowercase().starts_with("rgba(")
    }) {
        let parts: Vec<&str> = body.split(',').map(str::trim).collect();
        if parts.len() == 3 || parts.len() == 4 {
            let ok_rgb = parts[..3]
                .iter()
                .all(|p| p.parse::<u32>().map(|n| n <= 255).unwrap_or(false));
            let ok_a = parts.len() == 3
                || parts[3].parse::<f64>().map(|a| (0.0..=1.0).contains(&a)).unwrap_or(false);
            if ok_rgb && ok_a {
                return Ok(());
            }
        }
    }
    Err(CanvasError::Options(format!(
        "{label} is invalid: use hexadecimal, rgb() or rgba()."
    )))
}

/// Parses a validated color into premultiplied-friendly RGBA components.
pub(crate) fn parse_color(value: &str) -> Option<(u8, u8, u8, u8)> {
    let c = value.trim();
    if let Some(h) = c.strip_prefix('#') {
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        let expand = |i: usize| {
            let b = u8::from_str_radix(&h[i..i + 1], 16).ok()?;
            Some(b * 17)
        };
        return match h.len() {
            3 => Some((expand(0)?, expand(1)?, expand(2)?, 255)),
            4 => Some((expand(0)?, expand(1)?, expand(2)?, expand(3)?)),
            6 => Some((byte(0)?, byte(2)?, byte(4)?, 255)),
            8 => Some((byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        };
    }
    let inner = c
        .strip_prefix("rgba(")
        .or_else(|| c.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))?;
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }
    let r = parts[0].parse::<u8>().ok()?;
    let g = parts[1].parse::<u8>().ok()?;
    let b = parts[2].parse::<u8>().ok()?;
    let a = if parts.len() == 4 {
        (parts[3].parse::<f64>().ok()? * 255.0).round() as u8
    } else {
        255
    };
    Some((r, g, b, a))
}

fn gradient(
    linear: Option<(f64, f64, f64, f64)>,
    radial: Option<(f64, f64, f64, f64, f64, f64)>,
    stops: &[GradientStop],
) -> Result<CanvasFill, CanvasError> {
    if stops.len() < 2 || stops.len() > 16 {
        return Err(CanvasError::Options("A gradient requires 2 to 16 stops.".into()));
    }
    let mut previous = -1.0;
    for stop in stops {
        finite(stop.offset, "Gradient offset", 0.0, 1.0)?;
        if stop.offset < previous {
            return Err(CanvasError::Options(
                "Gradient stops must be in ascending order.".into(),
            ));
        }
        previous = stop.offset;
        validate_color(&stop.color, "Gradient color")?;
    }
    Ok(if let Some((x0, y0, x1, y1)) = linear {
        for (v, k) in [(x0, "x0"), (y0, "y0"), (x1, "x1"), (y1, "y1")] {
            finite(v, &format!("Gradient coordinate {k}"), -100_000.0, 100_000.0)?;
        }
        CanvasFill::Linear(LinearGradient { x0, y0, x1, y1, stops: stops.to_vec() })
    } else {
        let (x0, y0, r0, x1, y1, r1) = radial.unwrap();
        for (v, k) in [(x0, "x0"), (y0, "y0"), (r0, "r0"), (x1, "x1"), (y1, "y1"), (r1, "r1")] {
            finite(v, &format!("Gradient coordinate {k}"), -100_000.0, 100_000.0)?;
        }
        if r0 < 0.0 || r1 < 0.0 {
            return Err(CanvasError::Options("Invalid gradient radius.".into()));
        }
        CanvasFill::Radial(RadialGradient { x0, y0, r0, x1, y1, r1, stops: stops.to_vec() })
    })
}

/// Validates a fill value; helpers for callers building gradients.
pub fn validate_fill(fill: &CanvasFill) -> Result<(), CanvasError> {
    match fill {
        CanvasFill::Color(c) => validate_color(c, "Fill"),
        CanvasFill::Linear(g) => gradient(
            Some((g.x0, g.y0, g.x1, g.y1)),
            None,
            &g.stops,
        )
        .map(|_| ()),
        CanvasFill::Radial(g) => gradient(
            None,
            Some((g.x0, g.y0, g.r0, g.x1, g.y1, g.r1)),
            &g.stops,
        )
        .map(|_| ()),
    }
}

/// A font file must be a TTF/OTF (or WOFF/WOFF2 for directory registration).
pub fn validate_font_bytes(bytes: &[u8]) -> Result<(), CanvasError> {
    if bytes.len() > CANVAS_LIMITS.max_item_bytes {
        return Err(CanvasError::ExecutionLimit("Font exceeds 10 MB.".into()));
    }
    if bytes.len() < 12 {
        return Err(CanvasError::Input("Font must be at least 12 bytes.".into()));
    }
    let sig = &bytes[..4];
    let numeric = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if sig != b"OTTO" && sig != b"true" && sig != b"typ1" && sig != b"wOFF" && sig != b"wOF2"
        && numeric != 0x00010000
    {
        return Err(CanvasError::Input(
            "Font must be a valid TTF or OTF file.".into(),
        ));
    }
    Ok(())
}

fn dimensions(width: u32, height: u32) -> Result<ImageDimensions, CanvasError> {
    if width == 0
        || height == 0
        || width > CANVAS_LIMITS.max_input_dimension
        || height > CANVAS_LIMITS.max_input_dimension
        || width as u64 * height as u64 > CANVAS_LIMITS.max_pixels
    {
        return Err(CanvasError::Input(
            "Input image dimensions exceed the limits.".into(),
        ));
    }
    Ok(ImageDimensions { width, height, format: CanvasFormat::Png })
}

/// Validates a PNG/JPEG/WebP header and reports its dimensions.
/// Static images only: animated PNG/WebP is rejected, like yuzuha.
pub fn inspect_image(bytes: &[u8]) -> Result<ImageDimensions, CanvasError> {
    let input = |m: &str| CanvasError::Input(m.to_string());
    if bytes.is_empty() {
        return Err(input("Image cannot be empty."));
    }
    if bytes.len() > CANVAS_LIMITS.max_item_bytes {
        return Err(CanvasError::ExecutionLimit("Image exceeds 10 MB.".into()));
    }
    let data = bytes;
    // PNG
    if data.len() >= 8 && data[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
        if data.len() < 45 {
            return Err(input("Truncated PNG."));
        }
        let mut offset = 8usize;
        let (mut width, mut height, mut ended) = (0u32, 0u32, false);
        while offset + 12 <= data.len() {
            let length = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            let type_ = &data[offset + 4..offset + 8];
            if length > data.len() - offset - 12 {
                return Err(input("Truncated PNG."));
            }
            match type_ {
                b"IHDR" => {
                    if width != 0 || length != 13 {
                        return Err(input("Invalid PNG header."));
                    }
                    width = u32::from_be_bytes(data[offset + 8..offset + 12].try_into().unwrap());
                    height = u32::from_be_bytes(data[offset + 12..offset + 16].try_into().unwrap());
                }
                b"acTL" => return Err(input("Animated images are not accepted.")),
                b"IEND" => {
                    ended = length == 0 && offset + 12 == data.len();
                    break;
                }
                _ => {}
            }
            offset += length + 12;
        }
        if width == 0 || height == 0 || !ended {
            return Err(input("Incomplete PNG structure."));
        }
        return Ok(ImageDimensions { format: CanvasFormat::Png, ..dimensions(width, height)? });
    }
    // JPEG
    if data.len() >= 4 && data[0] == 0xff && data[1] == 0xd8 {
        let mut offset = 2usize;
        let (mut width, mut height, mut scan) = (0u32, 0u32, false);
        while offset + 4 <= data.len() {
            if data[offset] != 0xff {
                return Err(input("Invalid JPEG structure."));
            }
            while offset < data.len() && data[offset] == 0xff {
                offset += 1;
            }
            if offset >= data.len() {
                return Err(input("Truncated JPEG."));
            }
            let marker = data[offset];
            offset += 1;
            if marker == 0xd9 {
                break;
            }
            if marker == 0xda {
                scan = true;
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            if offset + 2 > data.len() {
                return Err(input("Truncated JPEG."));
            }
            let length = u16::from_be_bytes(data[offset..offset + 2].try_into().unwrap()) as usize;
            if length < 2 || offset + length > data.len() {
                return Err(input("Truncated JPEG."));
            }
            if matches!(marker, 0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf)
            {
                if length < 7 {
                    return Err(input("Invalid JPEG header."));
                }
                height = u16::from_be_bytes(data[offset + 3..offset + 5].try_into().unwrap()) as u32;
                width = u16::from_be_bytes(data[offset + 5..offset + 7].try_into().unwrap()) as u32;
            }
            offset += length;
        }
        if !scan || width == 0 || height == 0 || data.len() < 4
            || data[data.len() - 2] != 0xff || data[data.len() - 1] != 0xd9
        {
            return Err(input("Incomplete JPEG structure."));
        }
        return Ok(ImageDimensions { format: CanvasFormat::Jpeg, ..dimensions(width, height)? });
    }
    // WebP
    if data.len() >= 20 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        if u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize + 8 != data.len() {
            return Err(input("Truncated WebP structure."));
        }
        let mut offset = 12usize;
        let (mut width, mut height) = (0u32, 0u32);
        while offset + 8 <= data.len() {
            let type_ = &data[offset..offset + 4];
            let length = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let body = offset + 8;
            if length > data.len() - body {
                return Err(input("Truncated WebP."));
            }
            match type_ {
                b"VP8X" => {
                    if length < 10 || (data[body] & 0x02) != 0 {
                        return Err(input("Animated or invalid WebP."));
                    }
                    let read24 = |i: usize| -> u32 {
                        data[i] as u32 | (data[i + 1] as u32) << 8 | (data[i + 2] as u32) << 16
                    };
                    width = 1 + read24(body + 4);
                    height = 1 + read24(body + 7);
                }
                b"VP8 " if length >= 10
                    && data[body + 3] == 0x9d
                    && data[body + 4] == 0x01
                    && data[body + 5] == 0x2a =>
                {
                    width = (u16::from_le_bytes(data[body + 6..body + 8].try_into().unwrap())
                        & 0x3fff) as u32;
                    height = (u16::from_le_bytes(data[body + 8..body + 10].try_into().unwrap())
                        & 0x3fff) as u32;
                }
                b"VP8L" if length >= 5 && data[body] == 0x2f => {
                    width = 1 + data[body + 1] as u32 + (((data[body + 2] & 0x3f) as u32) << 8);
                    height = 1
                        + ((data[body + 2] as u32) >> 6)
                        + ((data[body + 3] as u32) << 2)
                        + (((data[body + 4] & 0x0f) as u32) << 10);
                }
                _ => {}
            }
            offset = body + length + (length & 1);
        }
        if width == 0 || height == 0 || offset != data.len() {
            return Err(input("Incomplete WebP structure."));
        }
        return Ok(ImageDimensions { format: CanvasFormat::WebP, ..dimensions(width, height)? });
    }
    Err(input("Input must be a static PNG, JPEG or WebP."))
}
