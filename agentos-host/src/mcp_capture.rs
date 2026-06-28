use agentos_protocol::JsonRpcResponse;

pub(crate) enum InterceptedResponse {
    Response(Vec<u8>),
    NoResponse,
}

#[cfg(target_os = "macos")]
pub(crate) fn try_handle_screen_capture(
    message: &serde_json::Value,
) -> anyhow::Result<Option<InterceptedResponse>> {
    let Some(kind) = screen_capture_kind(message) else {
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
    let png_b64 = encode_png_base64(capture.width, capture.height, &capture.pixels_rgba)?;
    let result = match kind {
        CaptureRequestKind::LegacyToolCall => serde_json::json!({
            "width": capture.width,
            "height": capture.height,
            "format": "png_base64",
            "data": png_b64,
        }),
        CaptureRequestKind::McpToolsCall => serde_json::json!({
            "content": [{
                "type": "image",
                "data": png_b64,
                "mimeType": "image/png",
            }]
        }),
    };
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
enum CaptureRequestKind {
    LegacyToolCall,
    McpToolsCall,
}

fn screen_capture_kind(message: &serde_json::Value) -> Option<CaptureRequestKind> {
    match message.get("method").and_then(serde_json::Value::as_str) {
        Some("tools/call") => {
            let params = message.get("params")?;
            let name = params.get("name").and_then(serde_json::Value::as_str)?;
            (name == "screen_capture").then_some(CaptureRequestKind::McpToolsCall)
        }
        _ => {
            let params = message.get("params")?;
            let tool = params.get("tool").and_then(serde_json::Value::as_str)?;
            (tool == "screen_capture").then_some(CaptureRequestKind::LegacyToolCall)
        }
    }
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
