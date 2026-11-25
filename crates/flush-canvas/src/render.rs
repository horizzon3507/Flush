//! Scene renderer backed by tiny-skia + fontdue
//! (the in-process counterpart of `infrastructure/canvas/worker.mjs`).

use std::collections::HashMap;
use std::sync::OnceLock;

use fontdue::{Font, FontSettings};
use tiny_skia::{
    FillRule, GradientStop as SkGradientStop, LinearGradient, Mask, Paint, PathBuilder, Pixmap,
    PixmapPaint, Point, RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::error::CanvasError;
use crate::types::*;

static INTER_REGULAR: &[u8] = include_bytes!("../assets/canvas/Inter-Regular.ttf");
static INTER_BOLD: &[u8] = include_bytes!("../assets/canvas/Inter-Bold.ttf");

static INTER: OnceLock<fontdue::Font> = OnceLock::new();
static INTER_BOLD_FONT: OnceLock<fontdue::Font> = OnceLock::new();

fn default_font(weight: FontWeight) -> &'static Font {
    let (cell, bytes) = match weight {
        FontWeight::Regular => (&INTER, INTER_REGULAR),
        FontWeight::Bold => (&INTER_BOLD_FONT, INTER_BOLD),
    };
    cell.get_or_init(|| {
        Font::from_bytes(bytes, FontSettings::default())
            .expect("bundled Inter font must parse")
    })
}

/// Font set for one render: `Inter` plus the fonts resolved for the scene.
struct FontSet<'a> {
    custom: HashMap<(String, FontWeight), Font>,
    inter: &'a Font,
    inter_bold: &'a Font,
}

impl FontSet<'_> {
    fn font(&self, family: &str, weight: FontWeight) -> Result<&Font, CanvasError> {
        if family == "Inter" {
            return Ok(match weight {
                FontWeight::Regular => self.inter,
                FontWeight::Bold => self.inter_bold,
            });
        }
        self.custom
            .get(&(family.to_string(), weight))
            .or_else(|| self.custom.get(&(family.to_string(), FontWeight::Regular)))
            .or_else(|| self.custom.get(&(family.to_string(), FontWeight::Bold)))
            .ok_or_else(|| CanvasError::Input(format!("Unknown font family: {family}.")))
    }
}

fn graphemes(value: &str) -> Vec<&str> {
    UnicodeSegmentation::graphemes(value, true).collect()
}

fn rounded_path(x: f64, y: f64, width: f64, height: f64, radius: f64) -> tiny_skia::Path {
    let r = (radius.max(0.0).min(width / 2.0).min(height / 2.0)) as f32;
    let (x, y, w, h) = (x as f32, y as f32, width as f32, height as f32);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish().unwrap_or_else(|| {
        PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap())
    })
}

fn circle_path(x: f64, y: f64, r: f64) -> tiny_skia::Path {
    let mut pb = PathBuilder::new();
    pb.push_circle(x as f32, y as f32, r as f32);
    pb.finish().unwrap_or_else(|| {
        PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, 1.0, 1.0).unwrap())
    })
}

fn shader_color(color: &str, opacity: f64, label: &str) -> Result<tiny_skia::Color, CanvasError> {
    let (r, g, b, a) =
        parse_color(color).ok_or_else(|| CanvasError::Options(format!("Invalid {label}.")))?;
    let mut c = tiny_skia::Color::from_rgba8(r, g, b, a);
    c.apply_opacity(opacity as f32);
    Ok(c)
}

fn gradient_stops(stops: &[GradientStop], opacity: f64) -> Result<Vec<SkGradientStop>, CanvasError> {
    stops
        .iter()
        .map(|s| Ok(SkGradientStop::new(s.offset as f32, shader_color(&s.color, opacity, "gradient color")?)))
        .collect()
}

fn fill_shader(fill: &CanvasFill, opacity: f64) -> Result<Shader<'static>, CanvasError> {
    match fill {
        CanvasFill::Color(c) => Ok(Shader::SolidColor(shader_color(c, opacity, "fill color")?)),
        CanvasFill::Linear(g) => LinearGradient::new(
            Point::from_xy(g.x0 as f32, g.y0 as f32),
            Point::from_xy(g.x1 as f32, g.y1 as f32),
            gradient_stops(&g.stops, opacity)?,
            SpreadMode::Pad,
            Transform::default(),
        )
        .ok_or_else(|| CanvasError::Options("Invalid linear gradient.".into())),
        CanvasFill::Radial(g) => RadialGradient::new(
            Point::from_xy(g.x0 as f32, g.y0 as f32),
            g.r0 as f32,
            Point::from_xy(g.x1 as f32, g.y1 as f32),
            g.r1 as f32,
            gradient_stops(&g.stops, opacity)?,
            SpreadMode::Pad,
            Transform::default(),
        )
        .ok_or_else(|| CanvasError::Options("Invalid radial gradient.".into())),
    }
}

fn fill_paint(fill: &CanvasFill, opacity: f64) -> Result<Paint<'static>, CanvasError> {
    Ok(Paint { shader: fill_shader(fill, opacity)?, ..Default::default() })
}

fn solid_paint(color: &str, opacity: f64) -> Result<Paint<'static>, CanvasError> {
    Ok(Paint { shader: Shader::SolidColor(shader_color(color, opacity, "color")?), ..Default::default() })
}

/// Straight-alpha RGBA → premultiplied tiny-skia pixmap.
fn rgba_to_pixmap(rgba: image::RgbaImage) -> Result<Pixmap, CanvasError> {
    let (w, h) = rgba.dimensions();
    let mut data = rgba.into_raw();
    for px in data.chunks_exact_mut(4) {
        let a = px[3] as u16;
        px[0] = ((px[0] as u16 * a + 127) / 255) as u8;
        px[1] = ((px[1] as u16 * a + 127) / 255) as u8;
        px[2] = ((px[2] as u16 * a + 127) / 255) as u8;
    }
    let size = tiny_skia::IntSize::from_wh(w, h)
        .ok_or_else(|| CanvasError::Process("Invalid image dimensions.".into()))?;
    Pixmap::from_vec(data, size)
        .ok_or_else(|| CanvasError::Process("Failed to build image pixmap.".into()))
}

/// Public hook for `Canvas::draw_image`.
pub(crate) fn rgba_to_pixmap_pub(rgba: image::RgbaImage) -> Option<Pixmap> {
    rgba_to_pixmap(rgba).ok()
}

/// Gaussian blur over a mask's alpha channel (the shadow recipe).
fn blur_mask(mask: &mut Mask, blur: f64) {
    if blur <= 0.0 {
        return;
    }
    let (w, h) = (mask.width(), mask.height());
    if let Some(img) = image::GrayImage::from_raw(w, h, mask.data().to_vec()) {
        let blurred = image::imageops::blur(&img, (blur / 2.0).max(0.4) as f32);
        mask.data_mut().copy_from_slice(&blurred);
    }
}

/// Draws the op's shadow: silhouette path, blurred, tinted and composited
/// at the shadow offset.
fn draw_shadow(canvas: &mut Pixmap, path: &tiny_skia::Path, shadow: &CanvasShadow, base: Transform) {
    let Some((r, g, b, a)) = parse_color(&shadow.color) else { return };
    let mut mask = Mask::new(canvas.width(), canvas.height()).unwrap();
    mask.fill_path(
        path,
        FillRule::Winding,
        true,
        base.post_translate(shadow.offset_x as f32, shadow.offset_y as f32),
    );
    blur_mask(&mut mask, shadow.blur);
    let mut shadow_pix = Pixmap::new(canvas.width(), canvas.height()).unwrap();
    shadow_pix.fill(tiny_skia::Color::from_rgba8(r, g, b, a));
    shadow_pix.apply_mask(&mask);
    canvas.draw_pixmap(0, 0, shadow_pix.as_ref(), &PixmapPaint::default(), Transform::default(), None);
}

fn opacity(options: &PaintOptions) -> f64 {
    options.opacity.unwrap_or(1.0)
}

struct TextLayout {
    lines: Vec<String>,
    size: f64,
    line_height: f64,
}

fn measure(font: &Font, text: &str, size: f64) -> f64 {
    text.chars()
        .map(|c| font.metrics(c, size as f32).advance_width as f64)
        .sum()
}

fn fit_lines(
    fonts: &FontSet,
    value: &str,
    options: &TextOptions,
    initial_size: f64,
) -> Result<TextLayout, CanvasError> {
    let family = options.family.as_deref().unwrap_or("Inter");
    let weight = options.weight.unwrap_or(FontWeight::Regular);
    let font = fonts.font(family, weight)?;
    let max_lines = options.max_lines.unwrap_or(1).max(1) as usize;
    let line_height = options.line_height.unwrap_or(1.2);
    let wrap = options.wrap.unwrap_or(TextWrap::None);
    let overflow = options.overflow.unwrap_or(TextOverflow::Ellipsis);
    let min_size = options.min_font_size.unwrap_or(initial_size);
    let max_width = options.max_width.unwrap_or(1.0);

    // Words + whitespace tokens, like `raw.split(/(\s+)/u)`.
    let tokens = |raw: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut last_ws: Option<bool> = None;
        for c in raw.chars() {
            let ws = c.is_whitespace();
            if last_ws.is_some() && last_ws != Some(ws) {
                out.push(std::mem::take(&mut current));
            }
            current.push(c);
            last_ws = Some(ws);
        }
        if !current.is_empty() {
            out.push(current);
        }
        out.into_iter().filter(|s| !s.is_empty()).collect()
    };

    let wrap_lines = |size: f64| -> Vec<String> {
        let mut out = Vec::new();
        for raw in value.split('\n') {
            if wrap == TextWrap::Word {
                let mut line = String::new();
                for word in tokens(raw) {
                    let combined = format!("{line}{word}");
                    if measure(font, &combined, size) <= max_width || line.is_empty() {
                        line = combined;
                    } else {
                        out.push(line.trim_end().to_string());
                        line = word.trim_start().to_string();
                    }
                    if measure(font, &line, size) > max_width {
                        let mut part = String::new();
                        for unit in graphemes(&line) {
                            if !part.is_empty()
                                && measure(font, &format!("{part}{unit}"), size) > max_width
                            {
                                out.push(part);
                                part = unit.to_string();
                            } else {
                                part.push_str(unit);
                            }
                        }
                        line = part;
                    }
                }
                out.push(line.trim_end().to_string());
            } else {
                out.push(raw.to_string());
            }
        }
        out
    };

    let fits = |size: f64| -> bool {
        let lines = wrap_lines(size);
        lines.len() <= max_lines
            && lines.iter().all(|line| measure(font, line, size) <= max_width)
    };

    let mut size = initial_size;
    while size > min_size && !fits(size) {
        size = (size - 1.0).max(min_size);
    }
    let mut lines = wrap_lines(size);
    let overflowed = lines.len() > max_lines
        || lines.iter().any(|line| measure(font, line, size) > max_width);
    if overflowed && overflow == TextOverflow::Error {
        return Err(CanvasError::Options("Text exceeds the configured box.".into()));
    }
    lines.truncate(max_lines);
    if overflowed && overflow == TextOverflow::Ellipsis {
        if let Some(last) = lines.last_mut() {
            let mut units: Vec<String> = graphemes(last).iter().map(|s| s.to_string()).collect();
            while !units.is_empty() && measure(font, &(units.concat() + "…"), size) > max_width {
                units.pop();
            }
            *last = format!("{}…", units.concat());
        }
    } else if overflow == TextOverflow::Clip {
        lines = lines
            .iter()
            .map(|line| {
                let mut value = String::new();
                for unit in graphemes(line) {
                    if measure(font, &format!("{value}{unit}"), size) > max_width {
                        break;
                    }
                    value.push_str(unit);
                }
                value
            })
            .collect();
    }
    Ok(TextLayout { lines, size, line_height })
}

/// Style + position for one glyph run.
struct GlyphPen {
    pen_x: f64,
    baseline_y: f64,
    color: (u8, u8, u8, u8),
    alpha: f64,
}

/// Draws one glyph: rasterize coverage, tint, composite.
fn draw_glyph(canvas: &mut Pixmap, font: &Font, ch: char, size: f64, pen: &GlyphPen) -> f64 {
    let (metrics, coverage) = font.rasterize(ch, size as f32);
    if metrics.width > 0 && metrics.height > 0 {
        let (r, g, b, a) = pen.color;
        let mut data = Vec::with_capacity(coverage.len() * 4);
        let max_alpha = a as f64 * pen.alpha;
        for cov in coverage {
            let ca = (cov as f64 / 255.0 * max_alpha).round() as u8;
            data.push((r as f64 * ca as f64 / 255.0).round() as u8);
            data.push((g as f64 * ca as f64 / 255.0).round() as u8);
            data.push((b as f64 * ca as f64 / 255.0).round() as u8);
            data.push(ca);
        }
        if let Some(size_) = tiny_skia::IntSize::from_wh(metrics.width as u32, metrics.height as u32) {
            if let Some(glyph) = Pixmap::from_vec(data, size_) {
                let top = pen.baseline_y - metrics.ymin as f64 - metrics.height as f64;
                canvas.draw_pixmap(
                    (pen.pen_x + metrics.xmin as f64).round() as i32,
                    top.round() as i32,
                    glyph.as_ref(),
                    &PixmapPaint::default(),
                    Transform::default(),
                    None,
                );
            }
        }
    }
    metrics.advance_width as f64
}

fn resolve_image<'a>(
    source: &CanvasSource,
    cache: &'a mut HashMap<String, image::RgbaImage>,
) -> Result<&'a image::RgbaImage, CanvasError> {
    let CanvasSource::Bytes(bytes) = source else {
        return Err(CanvasError::Process("Scene sent without resolved assets.".into()));
    };
    let key = source.key();
    if !cache.contains_key(&key) {
        cache.insert(key.clone(), crate::image::decode_rgba(bytes)?);
    }
    Ok(cache.get(&key).unwrap())
}

/// Renders a resolved scene (bytes only) to encoded bytes.
pub(crate) fn render_scene(scene: &Scene, options: &RenderOptions) -> Result<CanvasResult, CanvasError> {
    let pw = scene.width.checked_mul(scene.scale).unwrap_or(0);
    let ph = scene.height.checked_mul(scene.scale).unwrap_or(0);
    if pw == 0
        || ph == 0
        || pw > CANVAS_LIMITS.max_physical_dimension
        || ph > CANVAS_LIMITS.max_physical_dimension
        || pw as u64 * ph as u64 > CANVAS_LIMITS.max_pixels
    {
        return Err(CanvasError::Options("Physical dimensions exceed the Canvas limits.".into()));
    }

    let mut fonts = FontSet {
        custom: HashMap::new(),
        inter: default_font(FontWeight::Regular),
        inter_bold: default_font(FontWeight::Bold),
    };
    for font in &scene.fonts {
        let CanvasSource::Bytes(bytes) = &font.source else {
            return Err(CanvasError::Process("Scene sent without resolved fonts.".into()));
        };
        let parsed = Font::from_bytes(bytes.clone(), FontSettings::default())
            .map_err(|e| CanvasError::Input(format!("Could not register font {}: {e}", font.family)))?;
        fonts.custom.insert((font.family.clone(), font.weight), parsed);
    }

    // Largest box each source is drawn at, in physical pixels: decoded
    // images are downscaled to it so a gallery of big photos does not hold
    // full-resolution RGBA buffers it never paints.
    let scale = scene.scale as f64;
    let mut wanted: HashMap<String, (u32, u32)> = HashMap::new();
    for op in &scene.operations {
        let (key, w, h) = match op {
            Operation::Image(source, o) => (source.key(), o.width, o.height),
            Operation::Avatar(source, o) => (source.key(), o.size, o.size),
            Operation::Badges(items, _) => {
                for item in items {
                    if let Some(icon) = &item.icon {
                        let e = wanted.entry(icon.key()).or_insert((0, 0));
                        e.0 = e.0.max((16.0 * scale).ceil() as u32);
                        e.1 = e.1.max((16.0 * scale).ceil() as u32);
                    }
                }
                continue;
            }
            _ => continue,
        };
        let e = wanted.entry(key).or_insert((0, 0));
        e.0 = e.0.max((w * scale).ceil() as u32).min(pw);
        e.1 = e.1.max((h * scale).ceil() as u32).min(ph);
    }
    let mut images: HashMap<String, image::RgbaImage> = HashMap::new();
    for op in &scene.operations {
        match op {
            Operation::Image(source, _) | Operation::Avatar(source, _) => {
                resolve_image(source, &mut images)?;
            }
            Operation::Badges(items, _) => {
                for item in items {
                    if let Some(icon) = &item.icon {
                        resolve_image(icon, &mut images)?;
                    }
                }
            }
            _ => {}
        }
    }
    for (key, rgba) in images.iter_mut() {
        if let Some(&(tw, th)) = wanted.get(key) {
            if rgba.width() > tw.max(1) || rgba.height() > th.max(1) {
                *rgba = image::DynamicImage::ImageRgba8(rgba.clone())
                    .resize(tw.max(1), th.max(1), image::imageops::FilterType::Lanczos3)
                    .to_rgba8();
            }
        }
    }

    let mut canvas = Pixmap::new(pw, ph)
        .ok_or_else(|| CanvasError::Process("Failed to allocate the canvas.".into()))?;
    let base = Transform::from_scale(scene.scale as f32, scene.scale as f32);
    let format = options.format.unwrap_or(CanvasFormat::Png);
    if format == CanvasFormat::Jpeg {
        let matte = options.matte.as_deref().unwrap_or("#ffffff");
        canvas.fill_rect(
            Rect::from_xywh(0.0, 0.0, scene.width as f32, scene.height as f32).unwrap(),
            &solid_paint(matte, 1.0)?,
            base,
            None,
        );
    }
    if let Some(bg) = &scene.background {
        canvas.fill_rect(
            Rect::from_xywh(0.0, 0.0, scene.width as f32, scene.height as f32).unwrap(),
            &solid_paint(bg, 1.0)?,
            base,
            None,
        );
    }

    for op in &scene.operations {
        match op {
            Operation::Rect(o) => {
                let path = rounded_path(o.x, o.y, o.width, o.height, o.radius.unwrap_or(0.0));
                if let Some(sh) = &o.paint.shadow {
                    draw_shadow(&mut canvas, &path, sh, base);
                }
                let alpha = opacity(&o.paint);
                if let Some(fill) = &o.fill {
                    canvas.fill_path(&path, &fill_paint(fill, alpha)?, FillRule::Winding, base, None);
                }
                if let Some(stroke) = &o.stroke {
                    let stroke_cfg = Stroke { width: o.stroke_width.unwrap_or(1.0) as f32, ..Default::default() };
                    canvas.stroke_path(&path, &solid_paint(stroke, alpha)?, &stroke_cfg, base, None);
                }
            }
            Operation::Circle(o) => {
                let path = circle_path(o.x, o.y, o.radius);
                if let Some(sh) = &o.paint.shadow {
                    draw_shadow(&mut canvas, &path, sh, base);
                }
                let alpha = opacity(&o.paint);
                if let Some(fill) = &o.fill {
                    canvas.fill_path(&path, &fill_paint(fill, alpha)?, FillRule::Winding, base, None);
                }
                if let Some(stroke) = &o.stroke {
                    let stroke_cfg = Stroke { width: o.stroke_width.unwrap_or(1.0) as f32, ..Default::default() };
                    canvas.stroke_path(&path, &solid_paint(stroke, alpha)?, &stroke_cfg, base, None);
                }
            }
            Operation::Line(o) => {
                let mut pb = PathBuilder::new();
                pb.move_to(o.x1 as f32, o.y1 as f32);
                pb.line_to(o.x2 as f32, o.y2 as f32);
                let path = pb.finish().unwrap();
                let alpha = opacity(&o.paint);
                let cap = match o.cap.unwrap_or(LineCap::Butt) {
                    LineCap::Butt => tiny_skia::LineCap::Butt,
                    LineCap::Round => tiny_skia::LineCap::Round,
                    LineCap::Square => tiny_skia::LineCap::Square,
                };
                let stroke_cfg = Stroke { width: o.width.unwrap_or(1.0) as f32, line_cap: cap, ..Default::default() };
                if let Some(sh) = &o.paint.shadow {
                    if let Some(outline) = path.stroke(
                        &stroke_cfg,
                        tiny_skia_path::PathStroker::compute_resolution_scale(&base),
                    ) {
                        draw_shadow(&mut canvas, &outline, sh, base);
                    }
                }
                canvas.stroke_path(&path, &solid_paint(o.color.as_deref().unwrap_or("#ffffff"), alpha)?, &stroke_cfg, base, None);
            }
            Operation::Image(source, o) => {
                draw_image_op(&mut canvas, &images, source, o, base)?;
            }
            Operation::Avatar(source, o) => {
                let opts = ImageOptions {
                    x: o.x,
                    y: o.y,
                    width: o.size,
                    height: o.size,
                    fit: Some(ImageFit::Cover),
                    clip: Some(ImageClip::Circle),
                    border: o.border.clone(),
                    paint: o.paint.clone(),
                    ..Default::default()
                };
                draw_image_op(&mut canvas, &images, source, &opts, base)?;
            }
            Operation::Text(value, o) => {
                if let Some(sh) = &o.paint.shadow {
                    draw_text(&mut canvas, &fonts, value, o, base, Some(sh))?;
                }
                draw_text(&mut canvas, &fonts, value, o, base, None)?;
            }
            Operation::Badges(items, o) => {
                draw_badges(&mut canvas, &fonts, &images, items, o, base)?;
            }
            Operation::Progress(o) => {
                let radius = o.radius.unwrap_or(o.height / 2.0);
                let path = rounded_path(o.x, o.y, o.width, o.height, radius);
                if let Some(sh) = &o.paint.shadow {
                    draw_shadow(&mut canvas, &path, sh, base);
                }
                let alpha = opacity(&o.paint);
                let bg = o.background.clone().unwrap_or(CanvasFill::Color("#202938".into()));
                canvas.fill_path(&path, &fill_paint(&bg, alpha)?, FillRule::Winding, base, None);
                let amount = o.width * (o.value / o.max).clamp(0.0, 1.0);
                if amount > 0.0 {
                    let fill = o.fill.clone().unwrap_or(CanvasFill::Color("#70d8bc".into()));
                    let fg = rounded_path(o.x, o.y, amount, o.height, radius.min(amount / 2.0));
                    canvas.fill_path(&fg, &fill_paint(&fill, alpha)?, FillRule::Winding, base, None);
                }
            }
        }
    }

    let quality = options.quality.unwrap_or(85);
    let rgba = image::RgbaImage::from_raw(pw, ph, canvas.take_demultiplied())
        .ok_or_else(|| CanvasError::Process("Failed to read the canvas.".into()))?;
    let data = crate::image::encode(&image::DynamicImage::ImageRgba8(rgba), format, quality)?;
    if data.len() > CANVAS_LIMITS.max_output_bytes {
        return Err(CanvasError::ExecutionLimit("Encoded output exceeds 10 MB.".into()));
    }
    let decoded = inspect_image(&data)?;
    if decoded.width != pw || decoded.height != ph || decoded.format != format {
        return Err(CanvasError::Process(
            "Output dimensions or format do not match the scene.".into(),
        ));
    }
    Ok(CanvasResult { data, format, width: pw, height: ph })
}

fn draw_text(
    canvas: &mut Pixmap,
    fonts: &FontSet,
    value: &str,
    o: &TextOptions,
    base: Transform,
    shadow: Option<&CanvasShadow>,
) -> Result<(), CanvasError> {
    let alpha = opacity(&o.paint);
    let color = if let Some(sh) = shadow {
        parse_color(&sh.color).unwrap_or((0, 0, 0, 255))
    } else {
        parse_color(o.color.as_deref().unwrap_or("#ffffff")).unwrap_or((255, 255, 255, 255))
    };
    let scale = base.sx as f64;
    let layout = fit_lines(fonts, value, o, o.font_size.unwrap_or(24.0))?;
    let family = o.family.as_deref().unwrap_or("Inter");
    let weight = o.weight.unwrap_or(FontWeight::Regular);
    let font = fonts.font(family, weight)?;
    let physical_size = layout.size * scale;
    let ascent = font
        .horizontal_line_metrics(physical_size as f32)
        .map(|m| m.ascent as f64)
        .unwrap_or(physical_size);
    let (ox, oy) = shadow.map(|sh| (sh.offset_x * scale, sh.offset_y * scale)).unwrap_or((0.0, 0.0));
    let blur = shadow.map(|sh| sh.blur).unwrap_or(0.0);
    // A blurred shadow is drawn to a scratch pixmap, blurred, then composited.
    let mut scratch = if blur > 0.0 {
        Pixmap::new(canvas.width(), canvas.height())
    } else {
        None
    };
    let dest: &mut Pixmap = scratch.as_mut().unwrap_or(canvas);
    let max_width = o.max_width.unwrap_or(1.0);
    for (i, line) in layout.lines.iter().enumerate() {
        let line_width = measure(font, line, physical_size) / scale;
        let x = match o.align.unwrap_or(TextAlign::Left) {
            TextAlign::Left => o.x,
            TextAlign::Center => o.x + max_width / 2.0 - line_width / 2.0,
            TextAlign::Right => o.x + max_width - line_width,
        } * scale + ox;
        let baseline = (o.y + i as f64 * layout.size * layout.line_height) * scale + ascent + oy;
        let mut pen = GlyphPen { pen_x: x, baseline_y: baseline, color, alpha };
        for ch in line.chars() {
            pen.pen_x += draw_glyph(dest, font, ch, physical_size, &pen);
        }
    }
    if let Some(tmp) = scratch {
        let rgba = image::RgbaImage::from_raw(tmp.width(), tmp.height(), tmp.take_demultiplied());
        if let Some(rgba) = rgba {
            let blurred = image::imageops::blur(&rgba, blur as f32);
            if let Ok(pix) = rgba_to_pixmap(blurred) {
                canvas.draw_pixmap(0, 0, pix.as_ref(), &PixmapPaint::default(), Transform::default(), None);
            }
        }
    }
    Ok(())
}

fn draw_image_op(
    canvas: &mut Pixmap,
    images: &HashMap<String, image::RgbaImage>,
    source: &CanvasSource,
    o: &ImageOptions,
    base: Transform,
) -> Result<(), CanvasError> {
    let rgba = images
        .get(&source.key())
        .ok_or_else(|| CanvasError::Process("Missing resolved image.".into()))?;
    let mut img = image::DynamicImage::ImageRgba8(rgba.clone());

    let scale_x = base.sx as f64;
    let scale_y = base.sy as f64;
    let fit = o.fit.unwrap_or(ImageFit::Cover);
    // Resize work is bounded by what the canvas can actually show.
    let (tw, th) = (
        (o.width * scale_x).max(1.0).min(canvas.width() as f64) as u32,
        (o.height * scale_y).max(1.0).min(canvas.height() as f64) as u32,
    );
    img = match fit {
        ImageFit::Stretch => img.resize_exact(tw, th, image::imageops::FilterType::Lanczos3),
        ImageFit::Contain => img.resize(tw, th, image::imageops::FilterType::Lanczos3),
        ImageFit::Cover => {
            let target_ratio = o.width / o.height;
            let source_ratio = img.width() as f64 / img.height() as f64;
            let (mut sw, mut sh) = (img.width() as f64, img.height() as f64);
            if source_ratio > target_ratio {
                sw = img.height() as f64 * target_ratio;
            } else {
                sh = img.width() as f64 / target_ratio;
            }
            let sx = ((img.width() as f64 - sw) * o.crop_x.unwrap_or(0.5)).round() as u32;
            let sy = ((img.height() as f64 - sh) * o.crop_y.unwrap_or(0.5)).round() as u32;
            img.crop_imm(sx, sy, sw.max(1.0) as u32, sh.max(1.0) as u32)
                .resize_exact(tw, th, image::imageops::FilterType::Lanczos3)
        }
    };

    let clip = o.clip.unwrap_or(ImageClip::Rect);
    let clip_path = match clip {
        ImageClip::Rect => rounded_path(o.x, o.y, o.width, o.height, 0.0),
        ImageClip::Rounded => rounded_path(o.x, o.y, o.width, o.height, o.radius.unwrap_or(12.0)),
        ImageClip::Circle => circle_path(
            o.x + o.width / 2.0,
            o.y + o.height / 2.0,
            o.width.min(o.height) / 2.0,
        ),
    };

    // Shadow under the image (clipped silhouette).
    if let Some(sh) = &o.paint.shadow {
        draw_shadow(canvas, &clip_path, sh, base);
    }

    let mut mask = Mask::new(canvas.width(), canvas.height())
        .ok_or_else(|| CanvasError::Process("Failed to allocate the clip mask.".into()))?;
    mask.fill_path(&clip_path, FillRule::Winding, true, base);

    let pixmap = rgba_to_pixmap(img.to_rgba8())?;
    let dx = (o.x * scale_x).round() as i32;
    let dy = (o.y * scale_y).round() as i32;
    let (ix, iy) = if fit == ImageFit::Contain {
        (
            dx + ((o.width * scale_x - pixmap.width() as f64) / 2.0) as i32,
            dy + ((o.height * scale_y - pixmap.height() as f64) / 2.0) as i32,
        )
    } else {
        (dx, dy)
    };
    canvas.draw_pixmap(
        ix,
        iy,
        pixmap.as_ref(),
        &PixmapPaint { opacity: opacity(&o.paint) as f32, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() },
        Transform::default(),
        Some(&mask),
    );

    if let Some(border) = &o.border {
        let alpha = opacity(&o.paint);
        let stroke_cfg = Stroke { width: border.width as f32, ..Default::default() };
        let path = if clip == ImageClip::Circle {
            circle_path(
                o.x + o.width / 2.0,
                o.y + o.height / 2.0,
                (o.width.min(o.height) / 2.0 - border.width / 2.0).max(0.0),
            )
        } else {
            rounded_path(
                o.x + border.width / 2.0,
                o.y + border.width / 2.0,
                o.width - border.width,
                o.height - border.width,
                if clip == ImageClip::Rounded {
                    (o.radius.unwrap_or(12.0) - border.width / 2.0).max(0.0)
                } else {
                    0.0
                },
            )
        };
        canvas.stroke_path(&path, &solid_paint(&border.color, alpha)?, &stroke_cfg, base, None);
    }
    Ok(())
}

fn draw_badges(
    canvas: &mut Pixmap,
    fonts: &FontSet,
    images: &HashMap<String, image::RgbaImage>,
    items: &[BadgeItem],
    o: &BadgesOptions,
    base: Transform,
) -> Result<(), CanvasError> {
    let font_size = o.font_size.unwrap_or(14.0);
    let weight = o.weight.unwrap_or(FontWeight::Bold);
    let font = fonts.font("Inter", weight)?;
    let padding = o.padding_x.unwrap_or(12.0);
    let gap = o.gap.unwrap_or(8.0);
    let widths: Vec<f64> = items
        .iter()
        .map(|item| {
            padding * 2.0
                + if item.icon.is_some() { 18.0 + 6.0 } else { 0.0 }
                + measure(font, &item.text, font_size)
        })
        .collect();
    let more_width = |count: usize| -> f64 {
        (padding * 2.0 + measure(font, &format!("+{count}"), font_size)).min(o.max_width)
    };
    let mut shown = items.len();
    while shown > 0 {
        let count = items.len() - shown;
        let used: f64 = widths[..shown].iter().sum::<f64>()
            + (shown.saturating_sub(1) as f64) * gap
            + if count > 0 { gap + more_width(count) } else { 0.0 };
        if used <= o.max_width {
            break;
        }
        shown -= 1;
    }
    let overflow = items.len() - shown;
    let alpha = opacity(&o.paint);
    let scale_x = base.sx as f64;
    let scale_y = base.sy as f64;

    let mut cursor = 0.0;
    let height = o.height.unwrap_or(30.0);
    let radius = o.radius.unwrap_or(height / 2.0);
    let mut draw_badge = |label: &str,
                          width: f64,
                          selected: bool,
                          icon: Option<&CanvasSource>,
                          canvas: &mut Pixmap|
     -> Result<(), CanvasError> {
        let fill = if selected {
            o.selected_fill.as_deref().unwrap_or("#334155")
        } else {
            o.fill.as_deref().unwrap_or("#202938")
        };
        let path = rounded_path(o.x + cursor, o.y, width, height, radius);
        if let Some(sh) = &o.paint.shadow {
            draw_shadow(canvas, &path, sh, base);
        }
        canvas.fill_path(&path, &solid_paint(fill, alpha)?, FillRule::Winding, base, None);
        let mut text_x = o.x + cursor + padding;
        if let Some(icon) = icon {
            if let Some(rgba) = images.get(&icon.key()) {
                let src = image::DynamicImage::ImageRgba8(rgba.clone())
                    .resize_exact(
                        (16.0 * scale_x).max(1.0) as u32,
                        (16.0 * scale_y).max(1.0) as u32,
                        image::imageops::FilterType::Lanczos3,
                    )
                    .to_rgba8();
                if let Ok(pix) = rgba_to_pixmap(src) {
                    let mut pb = PathBuilder::new();
                    pb.push_circle((text_x + 8.0) as f32, (o.y + height / 2.0) as f32, 8.0);
                    if let Some(mask_path) = pb.finish() {
                        let mut mask = Mask::new(canvas.width(), canvas.height()).unwrap();
                        mask.fill_path(&mask_path, FillRule::Winding, true, base);
                        canvas.draw_pixmap(
                            (text_x * scale_x).round() as i32,
                            ((o.y + (height - 16.0) / 2.0) * scale_y).round() as i32,
                            pix.as_ref(),
                            &PixmapPaint::default(),
                            Transform::default(),
                            Some(&mask),
                        );
                    }
                }
            }
            text_x += 22.0;
        }
        let color = if selected {
            o.selected_color.as_deref().unwrap_or("#ffffff")
        } else {
            o.color.as_deref().unwrap_or("#d3d9e4")
        };
        let rgba = parse_color(color).unwrap_or((255, 255, 255, 255));
        let physical = font_size * scale_y;
        let ascent = font
            .horizontal_line_metrics(physical as f32)
            .map(|m| m.ascent as f64)
            .unwrap_or(physical);
        let baseline = (o.y + height / 2.0) * scale_y + ascent / 2.0;
        let avail = (width - (text_x - (o.x + cursor)) - padding).max(0.0) * scale_x;
        let mut pen = GlyphPen { pen_x: text_x * scale_x, baseline_y: baseline, color: rgba, alpha };
        let mut used = 0.0;
        for ch in label.chars() {
            let adv = font.metrics(ch, physical as f32).advance_width as f64;
            if used + adv > avail {
                break;
            }
            pen.pen_x += draw_glyph(canvas, font, ch, physical, &pen);
            used += adv;
        }
        cursor += width + gap;
        Ok(())
    };
    for (index, item) in items.iter().take(shown).enumerate() {
        draw_badge(&item.text, widths[index], item.selected.unwrap_or(false), item.icon.as_ref(), canvas)?;
    }
    if overflow > 0 {
        draw_badge(&format!("+{overflow}"), more_width(overflow), false, None, canvas)?;
    }
    Ok(())
}
