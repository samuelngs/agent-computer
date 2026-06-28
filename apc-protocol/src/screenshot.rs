use crate::Rect;

const BYTES_PER_PIXEL: usize = 4;
const MAX_SCREENSHOT_PIXELS: u64 = 67_108_864;

#[derive(Debug, Clone)]
pub struct RgbaScreenshot {
    pub width: u32,
    pub height: u32,
    pub pixels_rgba: Vec<u8>,
}

pub fn apply_capture_options(
    source_width: u32,
    source_height: u32,
    source_pixels_rgba: &[u8],
    region: Option<Rect>,
    scale: Option<f32>,
) -> Result<RgbaScreenshot, String> {
    validate_image(source_width, source_height, source_pixels_rgba)?;
    let region = validate_region(source_width, source_height, region)?;
    let cropped = crop_rgba(source_width, source_pixels_rgba, region)?;

    let scale = scale.unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("scale must be a positive finite number".into());
    }

    let width = scaled_dimension(region.width, scale)?;
    let height = scaled_dimension(region.height, scale)?;
    checked_image_len(width, height)?;

    if width == region.width && height == region.height {
        return Ok(RgbaScreenshot {
            width,
            height,
            pixels_rgba: cropped,
        });
    }

    Ok(RgbaScreenshot {
        width,
        height,
        pixels_rgba: resize_nearest_rgba(region.width, region.height, &cropped, width, height)?,
    })
}

fn validate_image(width: u32, height: u32, pixels_rgba: &[u8]) -> Result<(), String> {
    let expected_len = checked_image_len(width, height)?;
    if pixels_rgba.len() < expected_len {
        return Err(format!(
            "framebuffer too small: got {} bytes, expected at least {expected_len}",
            pixels_rgba.len()
        ));
    }
    Ok(())
}

fn validate_region(
    source_width: u32,
    source_height: u32,
    region: Option<Rect>,
) -> Result<Rect, String> {
    let region = region.unwrap_or(Rect {
        x: 0,
        y: 0,
        width: source_width,
        height: source_height,
    });

    if region.x < 0 || region.y < 0 {
        return Err("region x/y must be non-negative".into());
    }
    if region.width == 0 || region.height == 0 {
        return Err("region width/height must be greater than zero".into());
    }

    let x = region.x as u32;
    let y = region.y as u32;
    let right = x
        .checked_add(region.width)
        .ok_or_else(|| "region width overflows".to_string())?;
    let bottom = y
        .checked_add(region.height)
        .ok_or_else(|| "region height overflows".to_string())?;

    if right > source_width || bottom > source_height {
        return Err(format!(
            "region {x},{y} {}x{} exceeds framebuffer {source_width}x{source_height}",
            region.width, region.height
        ));
    }

    Ok(region)
}

fn crop_rgba(source_width: u32, pixels_rgba: &[u8], region: Rect) -> Result<Vec<u8>, String> {
    let x = region.x as usize;
    let y = region.y as usize;
    let source_width = source_width as usize;
    let row_bytes = region.width as usize * BYTES_PER_PIXEL;
    let mut pixels = Vec::with_capacity(checked_image_len(region.width, region.height)?);

    for row in 0..region.height as usize {
        let start = ((y + row) * source_width + x) * BYTES_PER_PIXEL;
        let end = start + row_bytes;
        pixels.extend_from_slice(&pixels_rgba[start..end]);
    }

    Ok(pixels)
}

fn resize_nearest_rgba(
    source_width: u32,
    source_height: u32,
    source_pixels_rgba: &[u8],
    target_width: u32,
    target_height: u32,
) -> Result<Vec<u8>, String> {
    let mut pixels = vec![0; checked_image_len(target_width, target_height)?];
    let source_width = source_width as usize;
    let source_height = source_height as usize;
    let target_width = target_width as usize;
    let target_height = target_height as usize;

    for target_y in 0..target_height {
        let source_y = target_y * source_height / target_height;
        for target_x in 0..target_width {
            let source_x = target_x * source_width / target_width;
            let source_offset = (source_y * source_width + source_x) * BYTES_PER_PIXEL;
            let target_offset = (target_y * target_width + target_x) * BYTES_PER_PIXEL;
            pixels[target_offset..target_offset + BYTES_PER_PIXEL].copy_from_slice(
                &source_pixels_rgba[source_offset..source_offset + BYTES_PER_PIXEL],
            );
        }
    }

    Ok(pixels)
}

fn scaled_dimension(source: u32, scale: f32) -> Result<u32, String> {
    let scaled = (source as f64 * scale as f64).round();
    if !scaled.is_finite() || scaled < 1.0 || scaled > u32::MAX as f64 {
        return Err(format!("scaled dimension is out of range: {scaled}"));
    }
    Ok(scaled as u32)
}

fn checked_image_len(width: u32, height: u32) -> Result<usize, String> {
    let pixels = width as u64 * height as u64;
    if pixels > MAX_SCREENSHOT_PIXELS {
        return Err(format!(
            "screenshot is too large: {width}x{height} exceeds {MAX_SCREENSHOT_PIXELS} pixels"
        ));
    }
    pixels
        .checked_mul(BYTES_PER_PIXEL as u64)
        .and_then(|len| usize::try_from(len).ok())
        .ok_or_else(|| "screenshot byte length overflows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_region() {
        let pixels = vec![
            0, 0, 0, 255, 1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6,
            0, 0, 255, 7, 0, 0, 255, 8, 0, 0, 255,
        ];

        let out = apply_capture_options(
            3,
            3,
            &pixels,
            Some(Rect {
                x: 1,
                y: 1,
                width: 2,
                height: 2,
            }),
            None,
        )
        .unwrap();

        assert_eq!((out.width, out.height), (2, 2));
        assert_eq!(
            out.pixels_rgba,
            vec![4, 0, 0, 255, 5, 0, 0, 255, 7, 0, 0, 255, 8, 0, 0, 255,]
        );
    }

    #[test]
    fn scales_region() {
        let pixels = vec![10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 40, 0, 0, 255];

        let out = apply_capture_options(2, 2, &pixels, None, Some(0.5)).unwrap();

        assert_eq!((out.width, out.height), (1, 1));
        assert_eq!(out.pixels_rgba, vec![10, 0, 0, 255]);
    }

    #[test]
    fn rejects_out_of_bounds_region() {
        let pixels = vec![0; 4 * 4 * BYTES_PER_PIXEL];
        let err = apply_capture_options(
            4,
            4,
            &pixels,
            Some(Rect {
                x: 3,
                y: 0,
                width: 2,
                height: 1,
            }),
            None,
        )
        .unwrap_err();

        assert!(err.contains("exceeds framebuffer"));
    }
}
