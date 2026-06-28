use apc_protocol::{JsonRpcResponse, Rect};

pub(crate) enum InterceptedResponse {
    Response(Vec<u8>),
    NoResponse,
}

#[cfg(target_os = "macos")]
pub(crate) fn try_handle_screen_capture(
    message: &serde_json::Value,
) -> anyhow::Result<Option<InterceptedResponse>> {
    let Some(options) = screen_capture_options(message) else {
        return Ok(None);
    };

    if message.get("id").is_none() {
        return Ok(Some(InterceptedResponse::NoResponse));
    }

    let Some(capture) = capture_current_frame()? else {
        return Ok(None);
    };

    let id = message
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let options = match options {
        Ok(options) => options,
        Err(e) => {
            let response = JsonRpcResponse::error(id, -32602, e);
            return Ok(Some(InterceptedResponse::Response(serde_json::to_vec(
                &response,
            )?)));
        }
    };
    let capture = match apc_protocol::screenshot::apply_capture_options(
        capture.width,
        capture.height,
        &capture.pixels_rgba,
        options.region,
        options.scale,
    ) {
        Ok(capture) => capture,
        Err(e) => {
            let response = JsonRpcResponse::error(id, -32602, e);
            return Ok(Some(InterceptedResponse::Response(serde_json::to_vec(
                &response,
            )?)));
        }
    };
    let png_b64 = encode_png_base64(capture.width, capture.height, &capture.pixels_rgba)?;
    let result = serde_json::json!({
        "content": [{
            "type": "image",
            "data": png_b64,
            "mimeType": "image/png",
        }]
    });
    let response = JsonRpcResponse::success(id, result);
    Ok(Some(InterceptedResponse::Response(serde_json::to_vec(
        &response,
    )?)))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn try_handle_screen_capture(
    _message: &serde_json::Value,
) -> anyhow::Result<Option<InterceptedResponse>> {
    Ok(None)
}

#[derive(Clone, Copy)]
struct ScreenCaptureOptions {
    region: Option<Rect>,
    scale: Option<f32>,
}

#[derive(serde::Deserialize)]
struct ScreenCaptureArgs {
    #[serde(default)]
    region: Option<Rect>,
    #[serde(default)]
    scale: Option<f32>,
}

fn screen_capture_options(
    message: &serde_json::Value,
) -> Option<Result<ScreenCaptureOptions, String>> {
    if message.get("method").and_then(serde_json::Value::as_str) != Some("tools/call") {
        return None;
    }

    let params = message.get("params")?;
    let name = params.get("name").and_then(serde_json::Value::as_str)?;
    (name == "screen_capture").then(|| {
        parse_screen_capture_options(
            params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        )
    })
}

fn parse_screen_capture_options(args: serde_json::Value) -> Result<ScreenCaptureOptions, String> {
    let parsed: ScreenCaptureArgs =
        serde_json::from_value(args).map_err(|e| format!("invalid screen_capture params: {e}"))?;
    Ok(ScreenCaptureOptions {
        region: parsed.region,
        scale: parsed.scale,
    })
}

#[cfg(target_os = "macos")]
struct HostCapture {
    width: u32,
    height: u32,
    pixels_rgba: Vec<u8>,
}

#[cfg(target_os = "macos")]
fn capture_current_frame() -> anyhow::Result<Option<HostCapture>> {
    if let Some(capture) = crate::display::global_display().capture_framebuffer() {
        return Ok(Some(HostCapture {
            width: capture.width,
            height: capture.height,
            pixels_rgba: capture.pixels_rgba,
        }));
    }

    if let Some((pixels_bgra, width, height)) =
        crate::headless::global_headless_display().capture_latest_framebuffer()
    {
        return Ok(Some(HostCapture {
            width,
            height,
            pixels_rgba: bgra_to_rgba(&pixels_bgra, width, height)?,
        }));
    }

    Ok(None)
}

#[cfg(target_os = "macos")]
fn bgra_to_rgba(pixels_bgra: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let expected_len = width as usize * height as usize * 4;
    if pixels_bgra.len() < expected_len {
        anyhow::bail!(
            "framebuffer too small: got {} bytes, expected at least {expected_len}",
            pixels_bgra.len()
        );
    }

    let mut pixels_rgba = Vec::with_capacity(expected_len);
    for px in pixels_bgra[..expected_len].chunks_exact(4) {
        pixels_rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    Ok(pixels_rgba)
}

#[cfg(target_os = "macos")]
fn encode_png_base64(width: u32, height: u32, pixels_rgba: &[u8]) -> anyhow::Result<String> {
    use base64::Engine;

    let mut png_buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_buf, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(pixels_rgba)?;
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(png_buf))
}
