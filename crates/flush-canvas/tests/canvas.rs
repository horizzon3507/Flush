//! Tests for flush-canvas: validators, `inspect_image`, `transform_image`,
//! the `Card` builder and the scene renderer. Loopback only.

use flush_canvas::*;

fn png_bytes(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    let mut img = image::RgbaImage::new(width, height);
    for px in img.pixels_mut() {
        *px = image::Rgba(color);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn jpeg_bytes(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(width, height, image::Rgb([200, 30, 30]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .unwrap();
    out
}

fn webp_bytes(width: u32, height: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 200, 30, 255]));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::WebP)
        .unwrap();
    out
}

fn code(err: &CanvasError) -> &'static str {
    err.code()
}

// ---------- validators ----------

#[test]
fn color_validation() {
    assert!(validate_color("#fff", "c").is_ok());
    assert!(validate_color("#ff0080aa", "c").is_ok());
    assert!(validate_color("rgb(1,2,3)", "c").is_ok());
    assert!(validate_color("rgba(1,2,3,0.5)", "c").is_ok());
    assert!(validate_color("red", "c").is_err());
    assert!(validate_color("#gg0000", "c").is_err());
    assert!(validate_color("rgba(1,2,3,2)", "c").is_err());
    assert!(validate_color("#00000000", "c").is_ok());
}

#[test]
fn fill_validation() {
    let two_stops = CanvasFill::Linear(LinearGradient {
        x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0,
        stops: vec![
            GradientStop { offset: 0.0, color: "#000".into() },
            GradientStop { offset: 1.0, color: "#fff".into() },
        ],
    });
    assert!(validate_fill(&two_stops).is_ok());
    let unsorted = CanvasFill::Linear(LinearGradient {
        x0: 0.0, y0: 0.0, x1: 1.0, y1: 0.0,
        stops: vec![
            GradientStop { offset: 0.5, color: "#000".into() },
            GradientStop { offset: 0.0, color: "#fff".into() },
        ],
    });
    assert!(validate_fill(&unsorted).is_err());
    let one_stop = CanvasFill::Radial(RadialGradient {
        x0: 0.0, y0: 0.0, r0: 0.0, x1: 1.0, y1: 0.0, r1: 4.0,
        stops: vec![GradientStop { offset: 0.0, color: "#000".into() }],
    });
    assert!(validate_fill(&one_stop).is_err());
}

#[test]
fn source_validation() {
    assert!(CanvasSource::parse("asset:avatar_1").is_ok());
    assert!(CanvasSource::parse("https://example.com/a.png").is_ok());
    assert!(matches!(
        CanvasSource::parse("https://user:pass@example.com/a.png"),
        Err(CanvasError::Input(_))
    ));
    assert!(CanvasSource::parse("ftp://example.com/a.png").is_err());
    assert!(CanvasSource::parse("asset:bad name!").is_err());
    assert!(CanvasSource::bytes(vec![1, 2, 3]).is_ok());
    assert!(CanvasSource::bytes(Vec::<u8>::new()).is_err());
    assert!(CanvasSource::bytes(vec![0u8; 10_000_001]).is_err());
}

#[test]
fn font_bytes_validation() {
    let mut ttf = vec![0x00, 0x01, 0x00, 0x00];
    ttf.extend_from_slice(&[0u8; 16]);
    assert!(validate_font_bytes(&ttf).is_ok());
    let mut otto = b"OTTO".to_vec();
    otto.extend_from_slice(&[0u8; 16]);
    assert!(validate_font_bytes(&otto).is_ok());
    assert!(validate_font_bytes(b"tiny").is_err());
    assert!(validate_font_bytes(b"not-a-font-bytes").is_err());
}

// ---------- inspect_image ----------

#[test]
fn inspect_valid_formats() {
    let png = png_bytes(20, 10, [1, 2, 3, 255]);
    let info = inspect_image(&png).unwrap();
    assert_eq!((info.width, info.height, info.format), (20, 10, CanvasFormat::Png));

    let jpeg = jpeg_bytes(30, 15);
    let info = inspect_image(&jpeg).unwrap();
    assert_eq!((info.width, info.height, info.format), (30, 15, CanvasFormat::Jpeg));

    let webp = webp_bytes(40, 20);
    let info = inspect_image(&webp).unwrap();
    assert_eq!((info.width, info.height, info.format), (40, 20, CanvasFormat::WebP));
}

#[test]
fn inspect_rejects_garbage() {
    assert_eq!(code(&inspect_image(b"not an image").unwrap_err()), "CANVAS_INPUT");
    let gif = {
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Gif)
            .unwrap();
        out
    };
    assert_eq!(code(&inspect_image(&gif).unwrap_err()), "CANVAS_INPUT");
}

// ---------- transform_image ----------

#[async_std::test]
async fn transform_basic_resize() {
    let src = png_bytes(64, 32, [10, 20, 30, 255]);
    let t = ImageTransform {
        width: Some(16),
        height: Some(16),
        ..Default::default()
    };
    let out = transform_image(&src, &t, &TransformOptions::default()).await.unwrap();
    assert_eq!(out.format, CanvasFormat::Png);
    let info = inspect_image(&out.data).unwrap();
    assert!(info.width <= 16 && info.height <= 16);
}

#[async_std::test]
async fn transform_jpeg_quality_and_rotation() {
    let src = png_bytes(40, 20, [255, 0, 0, 255]);
    let t = ImageTransform {
        format: Some(CanvasFormat::Jpeg),
        rotate: Some(90),
        quality: Some(70),
        ..Default::default()
    };
    let out = transform_image(&src, &t, &TransformOptions::default()).await.unwrap();
    let info = inspect_image(&out.data).unwrap();
    assert_eq!(info.format, CanvasFormat::Jpeg);
    assert_eq!((info.width, info.height), (20, 40));
}

#[async_std::test]
async fn transform_validation_errors() {
    let src = png_bytes(8, 8, [0, 0, 0, 255]);
    let bad = ImageTransform { width: Some(5000), ..Default::default() };
    assert_eq!(
        code(&transform_image(&src, &bad, &TransformOptions::default()).await.unwrap_err()),
        "CANVAS_OPTIONS"
    );
    let bad_rotate = ImageTransform { rotate: Some(45), ..Default::default() };
    assert_eq!(
        code(&transform_image(&src, &bad_rotate, &TransformOptions::default()).await.unwrap_err()),
        "CANVAS_OPTIONS"
    );
    assert_eq!(
        code(&transform_image(&[], &ImageTransform::default(), &TransformOptions::default())
            .await
            .unwrap_err()),
        "CANVAS_INPUT"
    );
    let bad_timeout = TransformOptions { timeout_ms: Some(1) };
    assert_eq!(
        code(&transform_image(&src, &ImageTransform::default(), &bad_timeout)
            .await
            .unwrap_err()),
        "CANVAS_OPTIONS"
    );
}

// ---------- Canvas (raw surface) ----------

#[test]
fn create_canvas_limits() {
    assert!(create_canvas(64, 64).is_ok());
    assert_eq!(code(&create_canvas(0, 10).unwrap_err()), "CANVAS_OPTIONS");
    assert_eq!(code(&create_canvas(4097, 10).unwrap_err()), "CANVAS_OPTIONS");
    assert_eq!(code(&create_canvas(8192, 4096).unwrap_err()), "CANVAS_OPTIONS");
}

#[test]
fn canvas_draw_and_encode() {
    let mut canvas = create_canvas(80, 60).unwrap();
    canvas.pixmap_mut().fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
    let png = canvas.encode(CanvasFormat::Png, 85).unwrap();
    let info = inspect_image(&png).unwrap();
    assert_eq!((info.width, info.height, info.format), (80, 60, CanvasFormat::Png));
    let jpeg = canvas.encode(CanvasFormat::Jpeg, 80).unwrap();
    assert_eq!(inspect_image(&jpeg).unwrap().format, CanvasFormat::Jpeg);
}

#[async_std::test]
async fn load_image_bytes() {
    let config = CanvasConfig { fetch_remote: false, ..Default::default() };
    let src = CanvasSource::bytes(png_bytes(12, 12, [9, 9, 9, 255])).unwrap();
    let image = load_image(&src, &config).await.unwrap();
    assert_eq!((image.width, image.height), (12, 12));
    let bad = CanvasSource::bytes(vec![1, 2, 3]).unwrap();
    assert_eq!(code(&load_image(&bad, &config).await.unwrap_err()), "CANVAS_INPUT");
    let asset = CanvasSource::parse("asset:missing").unwrap();
    assert_eq!(code(&load_image(&asset, &config).await.unwrap_err()), "CANVAS_ASSET");
}

#[test]
fn register_fonts_missing_dir() {
    assert_eq!(register_fonts("/nonexistent-dir-for-flush").unwrap(), 0);
}

// ---------- Card builder ----------

fn service() -> CanvasService<'static> {
    CanvasService::new(CanvasConfig { fetch_remote: false, ..Default::default() }).unwrap()
}

fn create_options() -> CreateOptions {
    CreateOptions::default()
}

#[async_std::test]
async fn card_renders_all_ops() {
    let service = service();
    let mut card = service.create(320, 180, &create_options()).unwrap();
    let img = CanvasSource::bytes(png_bytes(40, 40, [200, 50, 50, 255])).unwrap();
    card.rect(RectOptions {
        x: 0.0, y: 0.0, width: 320.0, height: 180.0,
        fill: Some(CanvasFill::Linear(LinearGradient {
            x0: 0.0, y0: 0.0, x1: 0.0, y1: 180.0,
            stops: vec![
                GradientStop { offset: 0.0, color: "#101828".into() },
                GradientStop { offset: 1.0, color: "#203050".into() },
            ],
        })),
        ..Default::default()
    })
    .unwrap()
    .circle(CircleOptions {
        x: 60.0, y: 60.0, radius: 30.0,
        fill: Some("#70d8bc".into()),
        paint: PaintOptions {
            opacity: Some(0.9),
            shadow: Some(CanvasShadow {
                color: "#00000080".into(), blur: 8.0, offset_x: 2.0, offset_y: 4.0,
            }),
        },
        ..Default::default()
    })
    .unwrap()
    .line(LineOptions { x1: 0.0, y1: 0.0, x2: 320.0, y2: 180.0, color: Some("#ffffff".into()), width: Some(2.0), cap: Some(LineCap::Round), ..Default::default() })
    .unwrap()
    .image(img.clone(), ImageOptions {
        x: 200.0, y: 20.0, width: 60.0, height: 60.0,
        clip: Some(ImageClip::Rounded), radius: Some(12.0),
        border: Some(Border { color: "#ffffff".into(), width: 2.0 }),
        ..Default::default()
    })
    .unwrap()
    .avatar(img, AvatarOptions {
        x: 20.0, y: 100.0, size: 48.0,
        border: Some(Border { color: "#70d8bc".into(), width: 3.0 }),
        ..Default::default()
    })
    .unwrap()
    .text("Flush canvas", TextOptions {
        x: 90.0, y: 110.0, font_size: Some(28.0), weight: Some(FontWeight::Bold),
        max_width: Some(220.0), color: Some("#ffffff".into()),
        ..Default::default()
    })
    .unwrap()
    .badges(
        vec![
            BadgeItem { text: "admin".into(), selected: Some(true), icon: None },
            BadgeItem { text: "moderator".into(), selected: None, icon: None },
        ],
        BadgesOptions { x: 8.0, y: 8.0, max_width: 300.0, ..Default::default() },
    )
    .unwrap()
    .progress(ProgressOptions {
        x: 8.0, y: 160.0, width: 300.0, height: 14.0, value: 35.0, max: 100.0,
        ..Default::default()
    })
    .unwrap();
    let result = card.render(&RenderOptions::default(), service.config()).await.unwrap();
    let info = inspect_image(&result.data).unwrap();
    assert_eq!((info.width, info.height, info.format), (320, 180, CanvasFormat::Png));
}

#[async_std::test]
async fn card_scale_and_jpeg_matte() {
    let service = service();
    let mut card = service
        .create(100, 60, &CreateOptions { scale: Some(2), ..Default::default() })
        .unwrap();
    card.rect(RectOptions {
        x: 10.0, y: 10.0, width: 80.0, height: 40.0,
        fill: Some("#203050".into()), radius: Some(8.0),
        ..Default::default()
    })
    .unwrap();
    let result = card
        .render(
            &RenderOptions { format: Some(CanvasFormat::Jpeg), matte: Some("#0a0f1a".into()), ..Default::default() },
            service.config(),
        )
        .await
        .unwrap();
    let info = inspect_image(&result.data).unwrap();
    assert_eq!((info.width, info.height, info.format), (200, 120, CanvasFormat::Jpeg));
}

#[async_std::test]
async fn card_webp() {
    let service = service();
    let mut card = service.create(32, 32, &create_options()).unwrap();
    card.rect(RectOptions {
        x: 0.0, y: 0.0, width: 32.0, height: 32.0, fill: Some("#123456".into()),
        ..Default::default()
    })
    .unwrap();
    let result = card
        .render(
            &RenderOptions { format: Some(CanvasFormat::WebP), ..Default::default() },
            service.config(),
        )
        .await
        .unwrap();
    assert_eq!(inspect_image(&result.data).unwrap().format, CanvasFormat::WebP);
}

#[test]
fn card_validation_errors() {
    let service = service();
    let mut card = service.create(100, 100, &create_options()).unwrap();
    // Bad geometry.
    assert!(card.rect(RectOptions { x: f64::NAN, y: 0.0, width: 10.0, height: 10.0, fill: Some("#fff".into()), ..Default::default() }).is_err());
    assert!(card.circle(CircleOptions { x: 0.0, y: 0.0, radius: -1.0, fill: Some("#fff".into()), ..Default::default() }).is_err());
    // Missing fill+stroke.
    assert!(card.rect(RectOptions { x: 0.0, y: 0.0, width: 10.0, height: 10.0, ..Default::default() }).is_err());
    // Bad color.
    assert!(card.rect(RectOptions { x: 0.0, y: 0.0, width: 10.0, height: 10.0, fill: Some("chartreuse".into()), ..Default::default() }).is_err());
    // Oversize text.
    assert!(card.text("x".repeat(CANVAS_LIMITS.max_text_length + 1), TextOptions { x: 0.0, y: 0.0, ..Default::default() }).is_err());
    // Bad opacity.
    assert!(card.progress(ProgressOptions {
        x: 0.0, y: 0.0, width: 10.0, height: 4.0, value: 1.0, max: 2.0,
        paint: PaintOptions { opacity: Some(2.0), ..Default::default() },
        ..Default::default()
    }).is_err());
    // minFontSize > fontSize.
    assert!(card.text("hi", TextOptions {
        x: 0.0, y: 0.0, font_size: Some(10.0), min_font_size: Some(20.0),
        ..Default::default()
    }).is_err());
    // Quality only for lossy formats.
    let render = async {
        card.render(
            &RenderOptions { format: Some(CanvasFormat::Png), quality: Some(50), ..Default::default() },
            service.config(),
        )
        .await
    };
    assert_eq!(code(&async_std::task::block_on(render).unwrap_err()), "CANVAS_OPTIONS");
}

#[test]
fn card_operation_limit() {
    let service = service();
    let mut card = service.create(16, 16, &create_options()).unwrap();
    for _ in 0..CANVAS_LIMITS.max_operations {
        card.line(LineOptions { x1: 0.0, y1: 0.0, x2: 1.0, y2: 1.0, ..Default::default() }).unwrap();
    }
    assert_eq!(
        code(&card.line(LineOptions { x1: 0.0, y1: 0.0, x2: 2.0, y2: 2.0, ..Default::default() }).unwrap_err()),
        "EXECUTION_LIMIT"
    );
}

#[test]
fn card_image_limit() {
    let service = service();
    let mut card = service.create(16, 16, &create_options()).unwrap();
    for i in 0..CANVAS_LIMITS.max_images {
        let src = CanvasSource::Asset(format!("img{i}"));
        card.image(src, ImageOptions { x: 0.0, y: 0.0, width: 4.0, height: 4.0, ..Default::default() }).unwrap();
    }
    let extra = CanvasSource::Asset("one-too-many".into());
    assert_eq!(
        code(&card.image(extra, ImageOptions { x: 0.0, y: 0.0, width: 4.0, height: 4.0, ..Default::default() }).unwrap_err()),
        "EXECUTION_LIMIT"
    );
}

#[async_std::test]
async fn unknown_family_and_asset_resolution() {
    let mut config = CanvasConfig { fetch_remote: false, ..Default::default() };
    config.assets.insert("badge".into(), png_bytes(8, 8, [1, 1, 1, 255]));
    let service = CanvasService::new(config).unwrap();
    let mut card = service.create(64, 64, &create_options()).unwrap();
    card.image(
        CanvasSource::parse("asset:badge").unwrap(),
        ImageOptions { x: 0.0, y: 0.0, width: 8.0, height: 8.0, ..Default::default() },
    )
    .unwrap();
    card.render(&RenderOptions::default(), service.config()).await.unwrap();

    let mut card2 = service.create(64, 64, &create_options()).unwrap();
    card2
        .text("hi", TextOptions { x: 0.0, y: 0.0, family: Some("DoesNotExist".into()), ..Default::default() })
        .unwrap();
    assert_eq!(
        code(&card2.render(&RenderOptions::default(), service.config()).await.unwrap_err()),
        "CANVAS_INPUT"
    );
}

#[async_std::test]
async fn send_without_transport() {
    let service = service();
    let card = service.create(8, 8, &create_options()).unwrap();
    assert_eq!(
        code(&service.send(&card, &SendOptions::default()).await.unwrap_err()),
        "CANVAS_STATE"
    );
}
