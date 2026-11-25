//! Renders a demo card to card.png (offline visual check).

use flush_canvas::*;

#[async_std::main]
async fn main() {
    let service = CanvasService::new(CanvasConfig {
        fetch_remote: false,
        ..Default::default()
    })
    .unwrap();
    let mut card = service.create(480, 220, &CreateOptions::default()).unwrap();

    let img_bytes = {
        let mut img = image::RgbaImage::new(64, 64);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgba([x as u8 * 4, y as u8 * 4, 200, 255]);
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    };
    let src = CanvasSource::bytes(img_bytes).unwrap();

    card.rect(RectOptions {
        x: 0.0, y: 0.0, width: 480.0, height: 220.0,
        fill: Some(CanvasFill::Linear(LinearGradient {
            x0: 0.0, y0: 0.0, x1: 0.0, y1: 220.0,
            stops: vec![
                GradientStop { offset: 0.0, color: "#101828".into() },
                GradientStop { offset: 1.0, color: "#203050".into() },
            ],
        })),
        ..Default::default()
    })
    .unwrap()
    .avatar(src.clone(), AvatarOptions {
        x: 24.0, y: 24.0, size: 72.0,
        border: Some(Border { color: "#70d8bc".into(), width: 3.0 }),
        ..Default::default()
    })
    .unwrap()
    .text("Flush canvas", TextOptions {
        x: 116.0, y: 34.0, font_size: Some(30.0), weight: Some(FontWeight::Bold),
        color: Some("#ffffff".into()), max_width: Some(340.0),
        ..Default::default()
    })
    .unwrap()
    .text("tiny-skia + fontdue render", TextOptions {
        x: 116.0, y: 66.0, font_size: Some(16.0),
        color: Some("#9aa7bd".into()), max_width: Some(340.0),
        ..Default::default()
    })
    .unwrap()
    .badges(
        vec![
            BadgeItem { text: "admin".into(), selected: Some(true), icon: None },
            BadgeItem { text: "rust".into(), selected: None, icon: None },
        ],
        BadgesOptions { x: 116.0, y: 96.0, max_width: 340.0, ..Default::default() },
    )
    .unwrap()
    .image(src, ImageOptions {
        x: 396.0, y: 140.0, width: 60.0, height: 60.0,
        clip: Some(ImageClip::Rounded), radius: Some(12.0),
        border: Some(Border { color: "#ffffff".into(), width: 2.0 }),
        ..Default::default()
    })
    .unwrap()
    .progress(ProgressOptions {
        x: 24.0, y: 184.0, width: 340.0, height: 16.0, value: 62.0, max: 100.0,
        ..Default::default()
    })
    .unwrap();

    let result = card.render(&RenderOptions::default(), service.config()).await.unwrap();
    std::fs::write("/tmp/card.png", &result.data).unwrap();
    println!("wrote /tmp/card.png ({} bytes)", result.data.len());
}
