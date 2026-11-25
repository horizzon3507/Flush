//! `CanvasService` + `CanvasBuilder` (mirrors `services/canvas.ts`).

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use flush_core::Snowflake;
use flush_rest::RestClient;

use crate::error::CanvasError;
use crate::render::render_scene;
use crate::types::*;

/// Fonts registered process-wide by [`register_fonts`], mirroring the
/// `GlobalFonts.registerFromPath` surface of `@napi-rs/canvas`.
static GLOBAL_FONTS: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

const FONT_EXTENSIONS: [&str; 4] = [".ttf", ".otf", ".woff", ".woff2"];

/// `registerFonts`: registers every font file in `dir` under its file name
/// (without extension). Returns the number of fonts registered; a missing
/// directory registers none.
pub fn register_fonts(dir: impl AsRef<std::path::Path>) -> Result<usize, CanvasError> {
    let dir = dir.as_ref();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(CanvasError::Options(format!("Could not read font directory: {e}"))),
    };
    if !dir.is_dir() {
        return Err(CanvasError::Options("registerFonts requires a directory.".into()));
    }
    let mut count = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let ext = name.rfind('.').map(|i| name[i..].to_ascii_lowercase());
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            || !ext.map(|e| FONT_EXTENSIONS.contains(&e.as_str())).unwrap_or(false)
        {
            continue;
        }
        let alias = &name[..name.rfind('.').unwrap()];
        if let Ok(bytes) = std::fs::read(entry.path()) {
            GLOBAL_FONTS.lock().unwrap().insert(alias.to_string(), bytes);
            count += 1;
        }
    }
    Ok(count)
}

/// Removes every process-wide font (`GlobalFonts.removeAll`).
pub fn clear_fonts() {
    GLOBAL_FONTS.lock().unwrap().clear();
}

/// Runtime configuration shared by every canvas of a service.
#[derive(Clone)]
pub struct CanvasConfig {
    /// `asset:` aliases to bytes.
    pub assets: HashMap<String, Vec<u8>>,
    /// Configured families: `regular` required, `bold` optional.
    pub fonts: HashMap<String, ConfiguredFont>,
    /// Default render timeout (10-30000 ms).
    pub timeout_ms: Option<u64>,
    /// Whether remote HTTP/HTTPS sources may be fetched. Default true.
    pub fetch_remote: bool,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            assets: HashMap::new(),
            fonts: HashMap::new(),
            timeout_ms: None,
            fetch_remote: true,
        }
    }
}

fn clone_config(config: &CanvasConfig) -> Result<CanvasConfig, CanvasError> {
    if let Some(t) = config.timeout_ms {
        if !(CANVAS_LIMITS.min_timeout_ms..=CANVAS_LIMITS.max_timeout_ms).contains(&t) {
            return Err(CanvasError::Options(format!(
                "Timeout must be {} to {} ms.",
                CANVAS_LIMITS.min_timeout_ms, CANVAS_LIMITS.max_timeout_ms
            )));
        }
    }
    for (name, value) in &config.assets {
        if name.is_empty() || name.len() > 64
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(CanvasError::Options(format!("Invalid asset alias: {name}.")));
        }
        if value.is_empty() {
            return Err(CanvasError::Options(format!("Asset {name} must contain bytes.")));
        }
    }
    for family in config.fonts.keys() {
        if !valid_family(family) {
            return Err(CanvasError::Options(format!(
                "Invalid configuration for font {family}."
            )));
        }
    }
    Ok(config.clone())
}

fn valid_family(family: &str) -> bool {
    let f = family.trim();
    !f.is_empty()
        && f.chars().count() <= 64
        && f.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '_' | '.' | '-'))
}

fn opt_error<T>(msg: &str) -> Result<T, CanvasError> {
    Err(CanvasError::Options(msg.to_string()))
}
fn char_count(value: &str) -> usize {
    value.chars().count()
}

fn validate_paint(paint: &PaintOptions) -> Result<(), CanvasError> {
    if let Some(op) = paint.opacity {
        finite(op, "Opacity", 0.0, 1.0)?;
    }
    if let Some(sh) = &paint.shadow {
        validate_color(&sh.color, "Shadow color")?;
        finite(sh.blur, "Shadow blur", 0.0, 128.0)?;
        finite(sh.offset_x, "Shadow offsetX", -100_000.0, 100_000.0)?;
        finite(sh.offset_y, "Shadow offsetY", -100_000.0, 100_000.0)?;
    }
    Ok(())
}

fn validate_border(border: &Border) -> Result<(), CanvasError> {
    validate_color(&border.color, "Border color")?;
    finite(border.width, "Border width", f64::MIN_POSITIVE, 128.0)?;
    Ok(())
}

fn positive(v: f64, label: &str, max: f64) -> Result<(), CanvasError> {
    finite(v, label, f64::MIN_POSITIVE, max)?;
    Ok(())
}

/// Options for [`CanvasService::send`].
#[derive(Clone, Default)]
pub struct SendOptions {
    pub channel_id: Option<Snowflake>,
    pub name: Option<String>,
    pub content: Option<String>,
    pub description: Option<String>,
    pub format: Option<CanvasFormat>,
    pub quality: Option<u32>,
    pub matte: Option<CanvasColor>,
    pub timeout_ms: Option<u64>,
}

fn format_options(options: &RenderOptions, fallback: u64) -> Result<(CanvasFormat, u32, Option<CanvasColor>, u64), CanvasError> {
    let format = options.format.unwrap_or(CanvasFormat::Png);
    if let Some(q) = options.quality {
        if format == CanvasFormat::Png || !(1..=100).contains(&q) {
            return opt_error("Quality is only valid for JPEG/WebP and must be 1 to 100.");
        }
    }
    if let Some(matte) = &options.matte {
        validate_color(matte, "Matte")?;
    }
    let timeout_ms = options.timeout_ms.unwrap_or(fallback);
    if !(CANVAS_LIMITS.min_timeout_ms..=CANVAS_LIMITS.max_timeout_ms).contains(&timeout_ms) {
        return opt_error(&format!(
            "Timeout must be {} to {} ms.",
            CANVAS_LIMITS.min_timeout_ms, CANVAS_LIMITS.max_timeout_ms
        ));
    }
    let quality = if format == CanvasFormat::Png { 85 } else { options.quality.unwrap_or(85) };
    Ok((format, quality, options.matte.clone(), timeout_ms))
}

fn attachment_name(name: Option<&str>, format: CanvasFormat) -> Result<String, CanvasError> {
    let extension = if format == CanvasFormat::Jpeg { "jpg" } else { format.as_str() };
    let Some(name) = name else {
        return Ok(format!("canvas.{extension}"));
    };
    if name.trim().is_empty() || name.len() > 255 || name.contains(['\r', '\n', '/', '\\']) {
        return opt_error("Invalid attachment name.");
    }
    let supplied = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !name.contains('.') {
        return Ok(format!("{name}.{extension}"));
    }
    if supplied != extension && !(format == CanvasFormat::Jpeg && supplied == "jpeg") {
        return opt_error("Name extension does not match the image format.");
    }
    Ok(name.to_string())
}

/// A decoded image (`loadImage`), ready for `Canvas::draw`.
#[derive(Clone, Debug)]
pub struct CanvasImage {
    pub(crate) rgba: image::RgbaImage,
    pub width: u32,
    pub height: u32,
}

/// A raw canvas like `@napi-rs/canvas`'s `createCanvas`: own the `tiny_skia`
/// pixmap and expose it plus `encode`. Re-exported `tiny_skia` gives the full
/// path/paint surface (`fill_rect`, `fill_path`, `stroke_path`, `draw_pixmap`).
#[derive(Debug)]
pub struct Canvas {
    pixmap: tiny_skia::Pixmap,
}

impl Canvas {
    /// `createCanvas`: logical 1x-scale pixmap with the physical-dimension checks.
    pub fn new(width: u32, height: u32) -> Result<Self, CanvasError> {
        if width == 0
            || height == 0
            || width > CANVAS_LIMITS.max_physical_dimension
            || height > CANVAS_LIMITS.max_physical_dimension
            || width as u64 * height as u64 > CANVAS_LIMITS.max_pixels
        {
            return opt_error("Physical dimensions exceed the Canvas limits.");
        }
        let pixmap = tiny_skia::Pixmap::new(width, height)
            .ok_or_else(|| CanvasError::Process("Failed to allocate the canvas.".into()))?;
        Ok(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }
    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }
    pub fn pixmap(&self) -> &tiny_skia::Pixmap {
        &self.pixmap
    }
    pub fn pixmap_mut(&mut self) -> &mut tiny_skia::Pixmap {
        &mut self.pixmap
    }

    /// Draws a decoded image at `(x, y)` (source over).
    pub fn draw_image(&mut self, image: &CanvasImage, x: i32, y: i32) {
        if let Some(pix) = crate::render::rgba_to_pixmap_pub(image.rgba.clone()) {
            self.pixmap.draw_pixmap(
                x,
                y,
                pix.as_ref(),
                &tiny_skia::PixmapPaint::default(),
                tiny_skia::Transform::default(),
                None,
            );
        }
    }

    /// `encode(format, quality)` → bytes.
    pub fn encode(&self, format: CanvasFormat, quality: u32) -> Result<Vec<u8>, CanvasError> {
        let rgba = image::RgbaImage::from_raw(
            self.pixmap.width(),
            self.pixmap.height(),
            self.pixmap.clone().take_demultiplied(),
        )
        .ok_or_else(|| CanvasError::Process("Failed to read the canvas.".into()))?;
        let data = crate::image::encode(&image::DynamicImage::ImageRgba8(rgba), format, quality)?;
        if data.len() > CANVAS_LIMITS.max_output_bytes {
            return Err(CanvasError::ExecutionLimit("Encoded image exceeds 10 MB.".into()));
        }
        Ok(data)
    }
}

/// `createCanvas` free function, like yuzuha's.
pub fn create_canvas(width: u32, height: u32) -> Result<Canvas, CanvasError> {
    Canvas::new(width, height)
}

/// `loadImage`: resolves a `CanvasSource` (bytes, `asset:` via `config`, or
/// an HTTP/HTTPS URL) and decodes it with the shared header validation.
pub async fn load_image(
    source: &CanvasSource,
    config: &CanvasConfig,
) -> Result<CanvasImage, CanvasError> {
    let bytes = resolve_source(source, config, "image", None).await?;
    inspect_image(&bytes)?;
    let rgba = crate::image::decode_rgba(&bytes)?;
    Ok(CanvasImage { width: rgba.width(), height: rgba.height(), rgba })
}

async fn resolve_source(
    source: &CanvasSource,
    config: &CanvasConfig,
    kind: &str,
    timeout: Option<Duration>,
) -> Result<Vec<u8>, CanvasError> {
    let data = match source {
        CanvasSource::Bytes(b) => b.clone(),
        CanvasSource::Asset(name) => config
            .assets
            .get(name)
            .cloned()
            .ok_or_else(|| CanvasError::Asset(format!("Asset not found: {name}.")))?,
        CanvasSource::Url(url) => {
            if !config.fetch_remote {
                return Err(CanvasError::Asset(
                    "Remote sources require a configured HTTP service.".into(),
                ));
            }
            let req = surf::get(url);
            let fut = async {
                let mut res = req.await.map_err(|e| {
                    CanvasError::Asset(format!("Could not resolve remote {kind}: {e}"))
                })?;
                if !res.status().is_success() {
                    return Err(CanvasError::Asset(format!(
                        "Downloading {kind} returned HTTP {}.",
                        res.status()
                    )));
                }
                res.body_bytes()
                    .await
                    .map_err(|e| CanvasError::Asset(format!("Could not read remote {kind}: {e}")))
            };
            match timeout {
                Some(t) => async_std::future::timeout(t, fut).await.map_err(|_| {
                    CanvasError::ExecutionLimit("Render exceeded the configured time.".into())
                })??,
                None => fut.await?,
            }
        }
    };
    if data.len() > CANVAS_LIMITS.max_item_bytes {
        return Err(CanvasError::ExecutionLimit(format!(
            "{} exceeds 10 MB.",
            if kind == "image" { "Image" } else { "Font" }
        )));
    }
    Ok(data)
}

/// Fluent scene builder (`CanvasBuilder` in yuzuha).
#[derive(Debug)]
pub struct Card {
    width: u32,
    height: u32,
    scale: u32,
    background: Option<CanvasColor>,
    operations: Vec<Operation>,
    fonts: HashMap<(String, FontWeight), FontSource>,
    image_keys: HashSet<String>,
    text_length: usize,
    weighted_operations: u32,
}

impl Card {
    /// `font(family, source, weight)` — a font available to later `text` calls.
    pub fn font(
        &mut self,
        family: impl Into<String>,
        source: CanvasSource,
        weight: FontWeight,
    ) -> Result<&mut Self, CanvasError> {
        let family = family.into().trim().to_string();
        if !valid_family(&family) {
            return opt_error("Invalid font family.");
        }
        let mut next = self.fonts.clone();
        next.insert((family.clone(), weight), FontSource { family, source, weight });
        let distinct: HashSet<String> = next.values().map(|f| f.source.key()).collect();
        if distinct.len() > CANVAS_LIMITS.max_font_files {
            return Err(CanvasError::ExecutionLimit(
                "Scene exceeds 8 custom font files.".into(),
            ));
        }
        self.fonts = next;
        Ok(self)
    }

    pub fn rect(&mut self, o: RectOptions) -> Result<&mut Self, CanvasError> {
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.width, "Width", 100_000.0)?;
        positive(o.height, "Height", 100_000.0)?;
        validate_paint(&o.paint)?;
        if o.fill.is_none() && o.stroke.is_none() {
            return opt_error("Rectangle requires fill or stroke.");
        }
        if let Some(f) = &o.fill {
            validate_fill(f)?;
        }
        if let Some(s) = &o.stroke {
            validate_color(s, "Stroke")?;
        }
        if let Some(w) = o.stroke_width {
            positive(w, "Stroke width", 128.0)?;
        }
        if let Some(r) = o.radius {
            finite(r, "Radius", 0.0, 100_000.0)?;
        }
        self.push(Operation::Rect(o), 1)
    }

    pub fn circle(&mut self, o: CircleOptions) -> Result<&mut Self, CanvasError> {
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.radius, "Radius", 100_000.0)?;
        validate_paint(&o.paint)?;
        if o.fill.is_none() && o.stroke.is_none() {
            return opt_error("Circle requires fill or stroke.");
        }
        if let Some(f) = &o.fill {
            validate_fill(f)?;
        }
        if let Some(s) = &o.stroke {
            validate_color(s, "Stroke")?;
        }
        if let Some(w) = o.stroke_width {
            positive(w, "Stroke width", 128.0)?;
        }
        self.push(Operation::Circle(o), 1)
    }

    pub fn line(&mut self, o: LineOptions) -> Result<&mut Self, CanvasError> {
        for (v, k) in [(o.x1, "x1"), (o.y1, "y1"), (o.x2, "x2"), (o.y2, "y2")] {
            finite(v, k, -100_000.0, 100_000.0)?;
        }
        validate_paint(&o.paint)?;
        if let Some(w) = o.width {
            positive(w, "Line width", 128.0)?;
        }
        if let Some(c) = &o.color {
            validate_color(c, "Line color")?;
        }
        self.push(Operation::Line(o), 1)
    }

    fn check_images(&self, sources: &[&CanvasSource]) -> Result<Vec<String>, CanvasError> {
        let keys: Vec<String> = sources.iter().map(|s| s.key()).collect();
        if self.image_keys.iter().chain(keys.iter()).collect::<HashSet<_>>().len()
            > CANVAS_LIMITS.max_images
        {
            return Err(CanvasError::ExecutionLimit(
                "Scene exceeds 64 distinct images.".into(),
            ));
        }
        Ok(keys)
    }

    pub fn image(&mut self, source: CanvasSource, o: ImageOptions) -> Result<&mut Self, CanvasError> {
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.width, "Width", 100_000.0)?;
        positive(o.height, "Height", 100_000.0)?;
        validate_paint(&o.paint)?;
        for (v, k) in [(o.crop_x, "cropX"), (o.crop_y, "cropY")] {
            if let Some(v) = v {
                finite(v, k, 0.0, 1.0)?;
            }
        }
        if let Some(r) = o.radius {
            finite(r, "Radius", 0.0, 100_000.0)?;
        }
        if let Some(b) = &o.border {
            validate_border(b)?;
        }
        let keys = self.check_images(&[&source])?;
        self.image_keys.extend(keys);
        self.push(Operation::Image(source, o), 1)
    }

    pub fn avatar(&mut self, source: CanvasSource, o: AvatarOptions) -> Result<&mut Self, CanvasError> {
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.size, "Avatar size", 100_000.0)?;
        validate_paint(&o.paint)?;
        if let Some(b) = &o.border {
            validate_border(b)?;
        }
        let keys = self.check_images(&[&source])?;
        self.image_keys.extend(keys);
        self.push(Operation::Avatar(source, o), 1)
    }

    pub fn text(&mut self, value: impl Into<String>, o: TextOptions) -> Result<&mut Self, CanvasError> {
        let value = value.into();
        if char_count(&value) > CANVAS_LIMITS.max_text_length {
            return opt_error("Text exceeds 4096 characters.");
        }
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        let max_width = o.max_width.unwrap_or((self.width as f64 - o.x).max(1.0));
        positive(max_width, "maxWidth", 100_000.0)?;
        if let Some(s) = o.font_size {
            positive(s, "fontSize", 512.0)?;
        }
        if let Some(s) = o.min_font_size {
            positive(s, "minFontSize", 512.0)?;
        }
        if let (Some(min), _) = (o.min_font_size, o.font_size) {
            if min > o.font_size.unwrap_or(24.0) {
                return opt_error("minFontSize cannot exceed fontSize.");
            }
        }
        if let Some(f) = &o.family {
            if !valid_family(f) {
                return opt_error("Invalid font family.");
            }
        }
        if let Some(c) = &o.color {
            validate_color(c, "Text color")?;
        }
        if let Some(n) = o.max_lines {
            if !(1..=100).contains(&n) {
                return opt_error("maxLines must be an integer from 1 to 100.");
            }
        }
        if let Some(lh) = o.line_height {
            finite(lh, "lineHeight", 0.5, 4.0)?;
        }
        validate_paint(&o.paint)?;
        let mut stored = o.clone();
        stored.max_width = Some(max_width);
        stored.family = stored.family.map(|f| f.trim().to_string());
        let len = char_count(&value);
        self.check_text_length(len)?;
        self.push(Operation::Text(value, stored), 1)?;
        self.text_length += len;
        Ok(self)
    }

    pub fn badges(&mut self, items: Vec<BadgeItem>, o: BadgesOptions) -> Result<&mut Self, CanvasError> {
        if items.len() > CANVAS_LIMITS.max_badges {
            return opt_error("Badges must contain at most 32 items.");
        }
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.max_width, "maxWidth", 100_000.0)?;
        validate_paint(&o.paint)?;
        if let Some(h) = o.height {
            positive(h, "Badge height", 512.0)?;
        }
        for (v, k) in [(o.gap, "gap"), (o.padding_x, "paddingX"), (o.radius, "radius")] {
            if let Some(v) = v {
                finite(v, k, 0.0, 512.0)?;
            }
        }
        for c in [&o.fill, &o.selected_fill, &o.color, &o.selected_color].into_iter().flatten() {
            validate_color(c, "Badge color")?;
        }
        if let Some(s) = o.font_size {
            positive(s, "fontSize", 128.0)?;
        }
        let mut text_length = 0usize;
        for item in &items {
            if item.text.is_empty() || char_count(&item.text) > CANVAS_LIMITS.max_text_length {
                return opt_error("Badge text exceeds 4096 characters or is empty.");
            }
            text_length += char_count(&item.text);
        }
        self.check_text_length(text_length)?;
        let icon_keys: Vec<String> = items
            .iter()
            .filter_map(|i| i.icon.as_ref())
            .map(|s| s.key())
            .collect();
        let weight = icon_keys.len() as u32 + 1;
        let keys = self.check_images(
            &items.iter().filter_map(|i| i.icon.as_ref()).collect::<Vec<_>>(),
        )?;
        self.image_keys.extend(keys);
        self.push(Operation::Badges(items, o), weight)?;
        self.text_length += text_length;
        Ok(self)
    }

    pub fn progress(&mut self, o: ProgressOptions) -> Result<&mut Self, CanvasError> {
        finite(o.x, "x", -100_000.0, 100_000.0)?;
        finite(o.y, "y", -100_000.0, 100_000.0)?;
        positive(o.width, "Width", 100_000.0)?;
        positive(o.height, "Height", 100_000.0)?;
        finite(o.value, "value", -f64::MAX, f64::MAX)?;
        positive(o.max, "max", 100_000.0)?;
        validate_paint(&o.paint)?;
        if let Some(f) = &o.fill {
            validate_fill(f)?;
        }
        if let Some(b) = &o.background {
            validate_fill(b)?;
        }
        if let Some(r) = o.radius {
            finite(r, "Radius", 0.0, 100_000.0)?;
        }
        self.push(Operation::Progress(o), 1)
    }

    fn push(&mut self, op: Operation, weight: u32) -> Result<&mut Self, CanvasError> {
        if self.weighted_operations + weight > CANVAS_LIMITS.max_operations {
            return Err(CanvasError::ExecutionLimit("Scene exceeds 512 operations.".into()));
        }
        self.operations.push(op);
        self.weighted_operations += weight;
        Ok(self)
    }

    fn check_text_length(&self, additional: usize) -> Result<(), CanvasError> {
        if self.text_length + additional > CANVAS_LIMITS.max_scene_text_length {
            return Err(CanvasError::ExecutionLimit(
                "Total text exceeds 32768 characters.".into(),
            ));
        }
        Ok(())
    }

    fn snapshot(&self) -> Scene {
        Scene {
            width: self.width,
            height: self.height,
            scale: self.scale,
            background: self.background.clone(),
            operations: self.operations.clone(),
            fonts: self.fonts.values().cloned().collect(),
        }
    }

    /// `render(options)`: resolves sources and fonts, enforces the scene
    /// limits and paints + encodes within the timeout.
    pub async fn render(
        &self,
        options: &RenderOptions,
        config: &CanvasConfig,
    ) -> Result<CanvasResult, CanvasError> {
        let (format, quality, matte, timeout_ms) =
            format_options(options, config.timeout_ms.unwrap_or(CANVAS_LIMITS.default_timeout_ms))?;
        let config = clone_config(config)?;
        let scene = self.snapshot();
        async_std::future::timeout(Duration::from_millis(timeout_ms), async move {
            resolve_and_render(scene, &config, format, quality, matte, timeout_ms).await
        })
        .await
        .map_err(|_| CanvasError::ExecutionLimit("Render exceeded the configured time.".into()))?
    }
}

async fn resolve_and_render(
    mut scene: Scene,
    config: &CanvasConfig,
    format: CanvasFormat,
    quality: u32,
    matte: Option<CanvasColor>,
    timeout_ms: u64,
) -> Result<CanvasResult, CanvasError> {
    let started = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    let mut source_cache: HashMap<String, Vec<u8>> = HashMap::new();
    let mut image_bytes: HashMap<String, Vec<u8>> = HashMap::new();
    for op in scene.operations.iter_mut() {
        match op {
            Operation::Image(source, _) | Operation::Avatar(source, _) => {
                let bytes =
                    resolve_cached(source, config, "image", &mut source_cache, started, timeout)
                        .await?;
                inspect_image(&bytes)?;
                image_bytes.insert(source.key(), bytes.clone());
                *source = CanvasSource::Bytes(bytes);
            }
            Operation::Badges(items, _) => {
                for item in items.iter_mut() {
                    if let Some(icon) = &item.icon {
                        let bytes = resolve_cached(
                            icon,
                            config,
                            "image",
                            &mut source_cache,
                            started,
                            timeout,
                        )
                        .await?;
                        inspect_image(&bytes)?;
                        image_bytes.insert(icon.key(), bytes.clone());
                        item.icon = Some(CanvasSource::Bytes(bytes));
                    }
                }
            }
            _ => {}
        }
    }

    // Requested families: every family used by a text op must resolve.
    let mut requested: HashMap<String, HashSet<FontWeight>> = HashMap::new();
    for op in &scene.operations {
        if let Operation::Text(_, o) = op {
            let family = o.family.clone().unwrap_or_else(|| "Inter".to_string());
            requested
                .entry(family)
                .or_default()
                .insert(o.weight.unwrap_or(FontWeight::Regular));
        }
    }
    let mut scene_fonts: HashMap<(String, FontWeight), FontSource> = scene
        .fonts
        .into_iter()
        .map(|f| ((f.family.clone(), f.weight), f))
        .collect();
    for (family, weights) in &requested {
        if family == "Inter" {
            continue;
        }
        let configured = config.fonts.get(family);
        if configured.is_none()
            && !scene_fonts.keys().any(|(f, _)| f == family)
            && !GLOBAL_FONTS.lock().unwrap().contains_key(family)
        {
            return Err(CanvasError::Input(format!("Unknown font family: {family}.")));
        }
        for weight in weights {
            if scene_fonts.contains_key(&(family.clone(), *weight)) {
                continue;
            }
            let source = configured.map(|c| {
                if *weight == FontWeight::Bold {
                    c.bold.clone().unwrap_or_else(|| c.regular.clone())
                } else {
                    c.regular.clone()
                }
            });
            let existing = scene_fonts
                .get(&(family.clone(), FontWeight::Regular))
                .or_else(|| scene_fonts.get(&(family.clone(), FontWeight::Bold)))
                .map(|f| f.source.clone());
            let global = GLOBAL_FONTS.lock().unwrap().get(family).map(|b| CanvasSource::Bytes(b.clone()));
            let selected = source.or(existing).or(global).ok_or_else(|| {
                CanvasError::Input(format!("Unknown font family: {family}."))
            })?;
            let bytes =
                resolve_cached(&selected, config, "font", &mut source_cache, started, timeout)
                    .await?;
            validate_font_bytes(&bytes)?;
            scene_fonts.insert(
                (family.clone(), *weight),
                FontSource { family: family.clone(), source: CanvasSource::Bytes(bytes), weight: *weight },
            );
        }
    }
    let mut fonts_cache: HashMap<(String, FontWeight), Vec<u8>> = HashMap::new();
    for ((family, weight), font) in scene_fonts.iter_mut() {
        let bytes = match &font.source {
            CanvasSource::Bytes(b) => b.clone(),
            other => {
                let bytes =
                    resolve_cached(other, config, "font", &mut source_cache, started, timeout)
                        .await?;
                validate_font_bytes(&bytes)?;
                font.source = CanvasSource::Bytes(bytes.clone());
                bytes
            }
        };
        validate_font_bytes(&bytes)?;
        fonts_cache.insert((family.clone(), *weight), bytes);
    }
    let distinct_font_files: HashSet<String> = scene_fonts
        .values()
        .map(|f| f.source.key())
        .collect();
    if distinct_font_files.len() > CANVAS_LIMITS.max_font_files {
        return Err(CanvasError::ExecutionLimit(
            "Scene exceeds 8 custom font files.".into(),
        ));
    }
    scene.fonts = scene_fonts.into_values().collect();

    // Scene-level budgets.
    let mut unique: HashSet<u64> = HashSet::new();
    let mut total = 0usize;
    for data in image_bytes.values().chain(fonts_cache.values()) {
        let h = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            data.hash(&mut hasher);
            hasher.finish()
        };
        if unique.insert(h) {
            total += data.len();
        }
    }
    if total > CANVAS_LIMITS.max_scene_asset_bytes {
        return Err(CanvasError::ExecutionLimit(
            "Assets and fonts exceed 20 MB per scene.".into(),
        ));
    }
    let distinct_images: HashSet<u64> = image_bytes
        .values()
        .map(|d| {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            d.hash(&mut hasher);
            hasher.finish()
        })
        .collect();
    if distinct_images.len() > CANVAS_LIMITS.max_images {
        return Err(CanvasError::ExecutionLimit(
            "Scene exceeds 64 distinct images.".into(),
        ));
    }
    if started.elapsed() > timeout {
        return Err(CanvasError::ExecutionLimit("Render exceeded the configured time.".into()));
    }
    // Painting and encoding are CPU-bound: run them on a blocking thread so
    // the enclosing `future::timeout` can cancel the await promptly.
    let render_options = RenderOptions {
        format: Some(format),
        quality: Some(quality),
        matte,
        timeout_ms: Some(timeout.as_millis() as u64),
    };
    async_std::task::spawn_blocking(move || render_scene(&scene, &render_options)).await
}

async fn resolve_cached(
    source: &CanvasSource,
    config: &CanvasConfig,
    kind: &str,
    cache: &mut HashMap<String, Vec<u8>>,
    started: Instant,
    timeout: Duration,
) -> Result<Vec<u8>, CanvasError> {
    let key = source.key();
    if let Some(existing) = cache.get(&key) {
        return Ok(existing.clone());
    }
    let remaining = timeout.saturating_sub(started.elapsed());
    let data = resolve_source(source, config, kind, Some(remaining)).await?;
    if data.len() > CANVAS_LIMITS.max_item_bytes {
        return Err(CanvasError::ExecutionLimit(format!(
            "{} exceeds 10 MB.",
            if kind == "image" { "Image" } else { "Font" }
        )));
    }
    cache.insert(key, data.clone());
    Ok(data)
}

/// `CanvasService`: creates cards, renders and sends them.
pub struct CanvasService<'a> {
    config: CanvasConfig,
    rest: Option<&'a RestClient>,
    channel_id: Option<Snowflake>,
}

impl<'a> CanvasService<'a> {
    /// A service without a message transport (`send` is unavailable).
    pub fn new(config: CanvasConfig) -> Result<Self, CanvasError> {
        Ok(Self { config: clone_config(&config)?, rest: None, channel_id: None })
    }

    /// A service that can also `send` its cards to a channel.
    pub fn with_transport(
        rest: &'a RestClient,
        channel_id: Snowflake,
        config: CanvasConfig,
    ) -> Result<Self, CanvasError> {
        Ok(Self { config: clone_config(&config)?, rest: Some(rest), channel_id: Some(channel_id) })
    }

    /// The config the service was built with.
    pub fn config(&self) -> &CanvasConfig {
        &self.config
    }

    /// `create(width, height, options)`.
    pub fn create(&self, width: u32, height: u32, options: &CreateOptions) -> Result<Card, CanvasError> {
        if width == 0 || height == 0 {
            return opt_error("Width and height must be positive integers.");
        }
        let scale = options.scale.unwrap_or(1);
        if !(1..=4).contains(&scale) {
            return opt_error("scale must be an integer from 1 to 4.");
        }
        let (pw, ph) = (width * scale, height * scale);
        if pw > 4096 || ph > 4096 || pw as u64 * ph as u64 > 16_777_216 {
            return opt_error("Physical dimensions exceed the Canvas limits.");
        }
        if let Some(bg) = &options.background {
            validate_color(bg, "Background")?;
        }
        Ok(Card {
            width,
            height,
            scale,
            background: options.background.clone(),
            operations: Vec::new(),
            fonts: HashMap::new(),
            image_keys: HashSet::new(),
            text_length: 0,
            weighted_operations: 0,
        })
    }

    /// `send(builder | canvasLike, options)`: render or encode, then upload
    /// the image with `RestClient::send_message_with_files`.
    pub async fn send(&self, card: &Card, options: &SendOptions) -> Result<Snowflake, CanvasError> {
        let Some(rest) = self.rest else {
            return Err(CanvasError::State(
                "This canvas has no message transport for sending.".into(),
            ));
        };
        let channel = options
            .channel_id
            .or(self.channel_id)
            .ok_or_else(|| CanvasError::State("Provide channelID to send the image.".into()))?;
        if let Some(content) = &options.content {
            if char_count(content) > 2000 {
                return opt_error("content must be at most 2000 characters.");
            }
        }
        if let Some(description) = &options.description {
            if char_count(description) > 1024 {
                return opt_error("description must be at most 1024 characters.");
            }
        }
        let render_options = RenderOptions {
            format: options.format,
            quality: options.quality,
            matte: options.matte.clone(),
            timeout_ms: options.timeout_ms,
        };
        let (format, _, _, _) = format_options(
            &render_options,
            self.config.timeout_ms.unwrap_or(CANVAS_LIMITS.default_timeout_ms),
        )?;
        let name = attachment_name(options.name.as_deref(), format)?;
        let result = card.render(&render_options, &self.config).await?;
        if result.format != format {
            return Err(CanvasError::Process(
                "Rendered format does not match the requested one.".into(),
            ));
        }
        let file = flush_rest::uploads::FileUpload::new(name, result.data)
            .content_type(format.mime())
            .description(options.description.clone().unwrap_or_default());
        let payload = flush_core::MessageSend {
            content: Some(options.content.clone().unwrap_or_default()),
            ..Default::default()
        };
        let message = rest.send_message_with_files(channel, &payload, &[file]).await?;
        Ok(message.id)
    }

    /// Encodes a raw [`Canvas`] and sends it (the `canvasLike` branch of `send`).
    pub async fn send_canvas(
        &self,
        canvas: &Canvas,
        options: &SendOptions,
    ) -> Result<Snowflake, CanvasError> {
        let Some(rest) = self.rest else {
            return Err(CanvasError::State(
                "This canvas has no message transport for sending.".into(),
            ));
        };
        let channel = options
            .channel_id
            .or(self.channel_id)
            .ok_or_else(|| CanvasError::State("Provide channelID to send the image.".into()))?;
        if let Some(content) = &options.content {
            if char_count(content) > 2000 {
                return opt_error("content must be at most 2000 characters.");
            }
        }
        if let Some(description) = &options.description {
            if char_count(description) > 1024 {
                return opt_error("description must be at most 1024 characters.");
            }
        }
        let render_options = RenderOptions {
            format: options.format,
            quality: options.quality,
            matte: options.matte.clone(),
            timeout_ms: options.timeout_ms,
        };
        let (format, quality, _, timeout_ms) = format_options(
            &render_options,
            self.config.timeout_ms.unwrap_or(CANVAS_LIMITS.default_timeout_ms),
        )?;
        let name = attachment_name(options.name.as_deref(), format)?;
        if canvas.width() == 0
            || canvas.height() == 0
            || canvas.width() > CANVAS_LIMITS.max_physical_dimension
            || canvas.height() > CANVAS_LIMITS.max_physical_dimension
            || canvas.width() as u64 * canvas.height() as u64 > CANVAS_LIMITS.max_pixels
        {
            return opt_error("Physical dimensions exceed the Canvas limits.");
        }
        let pixmap = canvas.pixmap().clone();
        let matte_opt = options.matte.clone();
        let data = async_std::future::timeout(
            Duration::from_millis(timeout_ms),
            async_std::task::spawn_blocking(move || -> Result<Vec<u8>, CanvasError> {
                let canvas = Canvas { pixmap };
                // JPEG matte: fill transparent pixels before encoding.
                if format == CanvasFormat::Jpeg && matte_opt.is_some() {
                    let mut flat = create_canvas(canvas.width(), canvas.height())?;
                    let (r, g, b, a) = parse_color(matte_opt.as_deref().unwrap()).unwrap();
                    let paint = tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(
                            tiny_skia::Color::from_rgba8(r, g, b, a),
                        ),
                        ..Default::default()
                    };
                    flat.pixmap_mut().fill_rect(
                        tiny_skia::Rect::from_xywh(
                            0.0,
                            0.0,
                            canvas.width() as f32,
                            canvas.height() as f32,
                        )
                        .unwrap(),
                        &paint,
                        tiny_skia::Transform::default(),
                        None,
                    );
                    flat.pixmap_mut().draw_pixmap(
                        0,
                        0,
                        canvas.pixmap().as_ref(),
                        &tiny_skia::PixmapPaint::default(),
                        tiny_skia::Transform::default(),
                        None,
                    );
                    flat.encode(format, quality)
                } else {
                    canvas.encode(format, quality)
                }
            }),
        )
        .await
        .map_err(|_| CanvasError::ExecutionLimit("Render exceeded the configured time.".into()))??;
        if data.len() > CANVAS_LIMITS.max_output_bytes {
            return Err(CanvasError::ExecutionLimit("Encoded image exceeds 10 MB.".into()));
        }
        let file = flush_rest::uploads::FileUpload::new(name, data)
            .content_type(format.mime())
            .description(options.description.clone().unwrap_or_default());
        let payload = flush_core::MessageSend {
            content: Some(options.content.clone().unwrap_or_default()),
            ..Default::default()
        };
        let message = rest.send_message_with_files(channel, &payload, &[file]).await?;
        Ok(message.id)
    }

    /// `loadImage` against this service's config.
    pub async fn load_image(&self, source: &CanvasSource) -> Result<CanvasImage, CanvasError> {
        load_image(source, &self.config).await
    }

    /// `createCard`/`createCanvas` convenience: build one card.
    pub fn card(&self, width: u32, height: u32, options: &CreateOptions) -> Result<Card, CanvasError> {
        self.create(width, height, options)
    }
}

/// `createCard` free function, like yuzuha's `canvas/card.ts`.
pub fn create_card(
    width: u32,
    height: u32,
    options: &CreateOptions,
) -> Result<Card, CanvasError> {
    CanvasService::new(CanvasConfig { fetch_remote: true, ..Default::default() })?.create(width, height, options)
}
