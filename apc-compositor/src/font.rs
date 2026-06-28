#[cfg(target_os = "linux")]
use std::sync::OnceLock;

#[cfg(target_os = "linux")]
static FONT: OnceLock<fontdue::Font> = OnceLock::new();

#[cfg(target_os = "linux")]
const FONT_PATHS: &[&str] = &[
    "/usr/share/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf",
    "/usr/share/fonts/jetbrains-mono/JetBrainsMono-Medium.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/noto/NotoSans-Medium.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Medium.ttf",
];

#[cfg(target_os = "linux")]
fn load_font() -> fontdue::Font {
    for path in FONT_PATHS {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(font) = fontdue::Font::from_bytes(data, fontdue::FontSettings::default()) {
                tracing::info!(path, "loaded taskbar font");
                return font;
            }
        }
    }
    panic!("no suitable font found; install fonts-noto in the guest rootfs")
}

#[cfg(target_os = "linux")]
pub(crate) fn get() -> &'static fontdue::Font {
    FONT.get_or_init(load_font)
}

#[cfg(target_os = "linux")]
pub(crate) fn render_text_onto(
    buf: &mut [u8],
    buf_w: i32,
    buf_h: i32,
    text: &str,
    font_size: f32,
    r: u8,
    g: u8,
    b: u8,
    padding_x: i32,
) {
    let font = get();
    let metrics = font
        .horizontal_line_metrics(font_size)
        .unwrap_or(fontdue::LineMetrics {
            ascent: font_size * 0.8,
            descent: font_size * -0.2,
            line_gap: 0.0,
            new_line_size: font_size,
        });
    let baseline = ((buf_h as f32 + metrics.ascent + metrics.descent) / 2.0) as i32;
    let max_x = buf_w - padding_x;

    let mut cursor_x = padding_x;
    for ch in text.chars() {
        let (glyph_metrics, bitmap) = font.rasterize(ch, font_size);
        let gx = cursor_x + glyph_metrics.xmin;
        let gy = baseline - glyph_metrics.height as i32 - glyph_metrics.ymin;

        for row in 0..glyph_metrics.height {
            for col in 0..glyph_metrics.width {
                let px = gx + col as i32;
                let py = gy + row as i32;
                if px < padding_x || px >= max_x || py < 0 || py >= buf_h {
                    continue;
                }
                let alpha = bitmap[row * glyph_metrics.width + col];
                if alpha == 0 {
                    continue;
                }
                let idx = ((py * buf_w + px) * 4) as usize;
                if idx + 3 >= buf.len() {
                    continue;
                }
                let a = alpha as u16;
                let inv = 255 - a;
                buf[idx] = ((r as u16 * a + buf[idx] as u16 * inv) / 255) as u8;
                buf[idx + 1] = ((g as u16 * a + buf[idx + 1] as u16 * inv) / 255) as u8;
                buf[idx + 2] = ((b as u16 * a + buf[idx + 2] as u16 * inv) / 255) as u8;
            }
        }

        cursor_x += glyph_metrics.advance_width as i32;
        if cursor_x >= max_x {
            break;
        }
    }
}
