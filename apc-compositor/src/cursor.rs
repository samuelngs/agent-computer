#[cfg(target_os = "linux")]
use smithay::backend::renderer::element::memory::MemoryRenderBuffer;
#[cfg(target_os = "linux")]
use smithay::{
    backend::drm::DrmDevice,
    utils::{Logical, Point, Transform},
};

#[cfg(target_os = "linux")]
use drm::{
    buffer::Buffer as DrmBuffer,
    control::{Device as ControlDevice, crtc, dumbbuffer::DumbBuffer},
};
#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "linux")]
const DEFAULT_THEME: &str = "Adwaita";
#[cfg(target_os = "linux")]
const DEFAULT_SIZE: u32 = 24;

#[cfg(target_os = "linux")]
const FALLBACK_HOTSPOT: (u32, u32) = (1, 1);

#[cfg(target_os = "linux")]
pub struct CursorTheme {
    theme: xcursor::CursorTheme,
    size: u32,
    scale: i32,
}

#[cfg(target_os = "linux")]
pub struct LoadedCursor {
    pub buffer: MemoryRenderBuffer,
    pub hotspot: (i32, i32),
}

#[cfg(target_os = "linux")]
pub struct LegacyHardwareCursor {
    crtc: crtc::Handle,
    buffer: DumbBuffer,
    hotspot: (i32, i32),
    active: bool,
}

#[cfg(target_os = "linux")]
impl LegacyHardwareCursor {
    pub fn new(device: &DrmDevice, crtc: crtc::Handle, scale: i32) -> Option<Self> {
        let supported = device.cursor_size();
        let width = supported.w.max(16);
        let height = supported.h.max(16);
        let mut buffer = match device.create_dumb_buffer((width, height), DrmFourcc::Argb8888, 32) {
            Ok(buffer) => buffer,
            Err(e) => {
                tracing::warn!(%e, width, height, "failed to create legacy hardware cursor buffer");
                return None;
            }
        };

        if let Err(e) = fill_hardware_cursor_buffer(device, &mut buffer, width, height, scale) {
            tracing::warn!(%e, width, height, "failed to initialize legacy hardware cursor buffer");
            let _ = device.destroy_dumb_buffer(buffer);
            return None;
        }

        let hotspot_scale = scale.max(1);
        Some(Self {
            crtc,
            buffer,
            hotspot: (
                FALLBACK_HOTSPOT.0 as i32 * hotspot_scale,
                FALLBACK_HOTSPOT.1 as i32 * hotspot_scale,
            ),
            active: false,
        })
    }

    pub fn move_to(
        &mut self,
        device: &DrmDevice,
        location: Point<f64, Logical>,
        scale: i32,
    ) -> std::io::Result<()> {
        if !self.active {
            #[allow(deprecated)]
            device.set_cursor(self.crtc, Some(&self.buffer))?;
            self.active = true;
            tracing::info!("legacy hardware cursor enabled");
        }

        let scale = scale.max(1) as f64;
        let x = (location.x * scale).round() as i32 - self.hotspot.0;
        let y = (location.y * scale).round() as i32 - self.hotspot.1;
        #[allow(deprecated)]
        device.move_cursor(self.crtc, (x, y))
    }
}

#[cfg(target_os = "linux")]
impl CursorTheme {
    pub fn load(scale: i32) -> Self {
        let theme_name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| DEFAULT_THEME.into());
        let size: u32 = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_SIZE);
        let theme = xcursor::CursorTheme::load(&theme_name);
        tracing::info!(%theme_name, size, scale, "cursor theme loaded");
        Self { theme, size, scale }
    }

    pub fn load_cursor(&self, name: &str) -> LoadedCursor {
        if let Some(path) = self.theme.load_icon(name) {
            if let Ok(data) = std::fs::read(&path) {
                if let Some(images) = xcursor::parser::parse_xcursor(&data) {
                    if let Some(img) = select_best_image(&images, self.size) {
                        return LoadedCursor {
                            buffer: image_to_buffer(img, self.scale),
                            hotspot: (img.xhot as i32, img.yhot as i32),
                        };
                    }
                }
            }
            tracing::warn!(name, ?path, "failed to parse cursor file");
        } else {
            tracing::warn!(name, "cursor not found in theme");
        }
        fallback_cursor(self.scale)
    }
}

#[cfg(target_os = "linux")]
fn fill_hardware_cursor_buffer(
    device: &DrmDevice,
    buffer: &mut DumbBuffer,
    width: u32,
    height: u32,
    scale: i32,
) -> std::io::Result<()> {
    #[rustfmt::skip]
    let arrow: [[u8; 16]; 16] = [
        [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,1,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,1,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,1,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,1,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,2,1,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,2,2,1,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,1,1,1,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,0,0,0,0,0,0,0,0,0],
        [1,2,2,1,1,2,2,1,0,0,0,0,0,0,0,0],
        [1,2,1,0,0,1,2,1,0,0,0,0,0,0,0,0],
        [1,1,0,0,0,1,2,2,1,0,0,0,0,0,0,0],
        [1,0,0,0,0,0,1,2,1,0,0,0,0,0,0,0],
        [0,0,0,0,0,0,1,1,1,0,0,0,0,0,0,0],
    ];

    let pitch = buffer.pitch() as usize;
    let scale = scale.max(1) as usize;
    let mut mapping = device.map_dumb_buffer(buffer)?;
    let bytes = mapping.as_mut();
    bytes.fill(0);

    for (src_y, row) in arrow.iter().enumerate() {
        for (src_x, px) in row.iter().enumerate() {
            if *px == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let x = src_x * scale + dx;
                    let y = src_y * scale + dy;
                    if x >= width as usize || y >= height as usize {
                        continue;
                    }
                    let i = y * pitch + x * 4;
                    if i + 3 >= bytes.len() {
                        continue;
                    }
                    let color = if *px == 1 { 0 } else { 255 };
                    bytes[i] = color;
                    bytes[i + 1] = color;
                    bytes[i + 2] = color;
                    bytes[i + 3] = 255;
                }
            }
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn select_best_image<'a>(
    images: &'a [xcursor::parser::Image],
    target_size: u32,
) -> Option<&'a xcursor::parser::Image> {
    images
        .iter()
        .min_by_key(|img| (img.size as i64 - target_size as i64).unsigned_abs())
}

#[cfg(target_os = "linux")]
fn image_to_buffer(img: &xcursor::parser::Image, scale: i32) -> MemoryRenderBuffer {
    let w = img.width as i32;
    let h = img.height as i32;
    let expected = (w as usize) * (h as usize) * 4;
    let data = if img.pixels_rgba.len() == expected {
        &img.pixels_rgba[..]
    } else {
        tracing::warn!(
            w,
            h,
            got = img.pixels_rgba.len(),
            expected,
            "xcursor pixel size mismatch, using fallback"
        );
        return fallback_cursor(scale).buffer;
    };
    MemoryRenderBuffer::from_slice(
        data,
        DrmFourcc::Abgr8888,
        (w, h),
        scale,
        Transform::Normal,
        None,
    )
}

#[cfg(target_os = "linux")]
fn fallback_cursor(scale: i32) -> LoadedCursor {
    #[rustfmt::skip]
    let arrow: [[u8; 16]; 16] = [
        [1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,1,0,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,1,0,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,1,0,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,1,0,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,0,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,2,1,0,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,2,2,1,0,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,1,1,1,0,0,0,0,0,0],
        [1,2,2,2,2,2,1,0,0,0,0,0,0,0,0,0],
        [1,2,2,1,1,2,2,1,0,0,0,0,0,0,0,0],
        [1,2,1,0,0,1,2,1,0,0,0,0,0,0,0,0],
        [1,1,0,0,0,1,2,2,1,0,0,0,0,0,0,0],
        [1,0,0,0,0,0,1,2,1,0,0,0,0,0,0,0],
        [0,0,0,0,0,0,1,1,1,0,0,0,0,0,0,0],
    ];
    let mut data = vec![0u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            let i = (y * 16 + x) * 4;
            match arrow[y][x] {
                1 => {
                    data[i] = 0;
                    data[i + 1] = 0;
                    data[i + 2] = 0;
                    data[i + 3] = 255;
                }
                2 => {
                    data[i] = 255;
                    data[i + 1] = 255;
                    data[i + 2] = 255;
                    data[i + 3] = 255;
                }
                _ => {}
            }
        }
    }
    LoadedCursor {
        buffer: MemoryRenderBuffer::from_slice(
            &data,
            DrmFourcc::Abgr8888,
            (16, 16),
            scale,
            Transform::Normal,
            None,
        ),
        hotspot: (FALLBACK_HOTSPOT.0 as i32, FALLBACK_HOTSPOT.1 as i32),
    }
}
