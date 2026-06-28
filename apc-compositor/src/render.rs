#[cfg(target_os = "linux")]
use anyhow::Result;

#[cfg(target_os = "linux")]
use smithay::{
    backend::renderer::element::AsRenderElements,
    backend::{
        allocator::{Buffer, dmabuf::AsDmabuf, dumb::DumbAllocator, dumb::DumbBuffer},
        drm::{
            DrmDeviceFd,
            compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement, PrimarySwapchainElement},
            dumb::DumbFramebuffer,
        },
        renderer::{
            Bind, ExportMem, ImportAll, ImportMem, Renderer,
            element::{
                Id, Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::SolidColorRenderElement,
                surface::WaylandSurfaceRenderElement,
            },
            pixman::PixmanRenderer,
            sync::SyncPoint,
        },
    },
    desktop::{Window, space::SpaceRenderElements},
    utils::{Physical, Rectangle, Scale, Transform},
};

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "linux")]
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use super::state::ApcCompositor;

#[cfg(target_os = "linux")]
use super::taskbar::get_window_title;

#[cfg(target_os = "linux")]
use super::font;

#[cfg(target_os = "linux")]
smithay::backend::renderer::element::render_elements! {
    pub(crate) OutputRenderElements<R, E> where R: ImportAll + ImportMem;
    Space=SpaceRenderElements<R, E>,
    Cursor=MemoryRenderBufferRenderElement<R>,
    Solid=SolidColorRenderElement,
}

#[cfg(target_os = "linux")]
pub(crate) type SoftwareDrmCompositor = DrmCompositor<DumbAllocator, DrmDeviceFd, (), DrmDeviceFd>;

#[cfg(target_os = "linux")]
pub(crate) const SSD_TITLE_BAR_HEIGHT: i32 = 30;
#[cfg(target_os = "linux")]
const BASE_TASKBAR_HEIGHT: i32 = 36;
#[cfg(target_os = "linux")]
const BASE_TASKBAR_BTN_WIDTH: i32 = 140;
#[cfg(target_os = "linux")]
const BASE_TASKBAR_BTN_HEIGHT: i32 = 28;
#[cfg(target_os = "linux")]
const BASE_TASKBAR_BTN_GAP: i32 = 4;
#[cfg(target_os = "linux")]
const BASE_TASKBAR_BTN_MARGIN: i32 = 4;

#[cfg(target_os = "linux")]
pub(crate) struct ScreenSnapshot {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels_rgba: Vec<u8>,
}

#[cfg(target_os = "linux")]
pub(crate) fn taskbar_height(scale: i32) -> i32 {
    BASE_TASKBAR_HEIGHT * scale
}
#[cfg(target_os = "linux")]
pub(crate) fn taskbar_btn_width(scale: i32) -> i32 {
    BASE_TASKBAR_BTN_WIDTH * scale
}
#[cfg(target_os = "linux")]
pub(crate) fn taskbar_btn_height(scale: i32) -> i32 {
    BASE_TASKBAR_BTN_HEIGHT * scale
}
#[cfg(target_os = "linux")]
pub(crate) fn taskbar_btn_gap(scale: i32) -> i32 {
    BASE_TASKBAR_BTN_GAP * scale
}
#[cfg(target_os = "linux")]
pub(crate) fn taskbar_btn_margin(scale: i32) -> i32 {
    BASE_TASKBAR_BTN_MARGIN * scale
}

#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
pub(crate) enum RedrawState {
    #[default]
    Idle,
    Queued,
    WaitingForVBlank {
        redraw_needed: bool,
        submitted_at: Instant,
    },
}

#[cfg(target_os = "linux")]
pub(crate) fn create_solid_buffer(
    w: i32,
    h: i32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
    scale: i32,
) -> MemoryRenderBuffer {
    let data = vec![[r, g, b, a]; (w * h) as usize]
        .into_iter()
        .flatten()
        .collect::<Vec<u8>>();
    MemoryRenderBuffer::from_slice(
        &data,
        DrmFourcc::Abgr8888,
        (w, h),
        scale,
        Transform::Normal,
        None,
    )
}

#[cfg(target_os = "linux")]
fn create_label_buffer(
    w: i32,
    h: i32,
    bg_r: u8,
    bg_g: u8,
    bg_b: u8,
    text: &str,
    font_size: f32,
    padding_x: i32,
    scale: i32,
) -> MemoryRenderBuffer {
    let mut data = vec![[bg_r, bg_g, bg_b, 255u8]; (w * h) as usize]
        .into_iter()
        .flatten()
        .collect::<Vec<u8>>();
    font::render_text_onto(&mut data, w, h, text, font_size, 220, 220, 220, padding_x);
    MemoryRenderBuffer::from_slice(
        &data,
        DrmFourcc::Abgr8888,
        (w, h),
        scale,
        Transform::Normal,
        None,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn queue_redraw(state: &mut ApcCompositor) {
    match &state.redraw_state {
        RedrawState::Idle => {
            state.redraw_state = RedrawState::Queued;
        }
        RedrawState::Queued => {}
        RedrawState::WaitingForVBlank { submitted_at, .. } => {
            state.redraw_state = RedrawState::WaitingForVBlank {
                redraw_needed: true,
                submitted_at: *submitted_at,
            };
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn render_frame(state: &mut ApcCompositor) {
    state.last_render_at = Instant::now();

    static INITIAL_RENDER_STARTED: AtomicBool = AtomicBool::new(false);
    if !INITIAL_RENDER_STARTED.swap(true, Ordering::Relaxed) {
        tracing::info!("render_frame: initial render started");
    }

    static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);
    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    let trace_frame = frame < 256;
    if trace_frame {
        tracing::info!(frame, redraw_state = ?state.redraw_state, "render_frame begin");
    }

    let s = state.scale_factor;
    let sf = s as f64;
    let pointer_loc = state.pointer.current_location();
    let mut elements: Vec<
        OutputRenderElements<PixmanRenderer, WaylandSurfaceRenderElement<PixmanRenderer>>,
    > = Vec::new();

    if state.render_software_cursor() {
        let (cursor_buffer, cursor_hotspot) = match state.cursor_shape {
            crate::input::CursorShape::Default => (
                state.cursor_default.buffer.clone(),
                state.cursor_default.hotspot,
            ),
            crate::input::CursorShape::ResizeNWSE => (
                state.cursor_resize_nwse.buffer.clone(),
                state.cursor_resize_nwse.hotspot,
            ),
            crate::input::CursorShape::ResizeNESW => (
                state.cursor_resize_nesw.buffer.clone(),
                state.cursor_resize_nesw.hotspot,
            ),
            crate::input::CursorShape::ResizeNS => (
                state.cursor_resize_ns.buffer.clone(),
                state.cursor_resize_ns.hotspot,
            ),
            crate::input::CursorShape::ResizeEW => (
                state.cursor_resize_ew.buffer.clone(),
                state.cursor_resize_ew.hotspot,
            ),
        };
        match MemoryRenderBufferRenderElement::from_buffer(
            &mut state.renderer,
            (
                pointer_loc.x * sf - cursor_hotspot.0 as f64,
                pointer_loc.y * sf - cursor_hotspot.1 as f64,
            ),
            &cursor_buffer,
            None,
            None,
            None,
            Kind::Cursor,
        ) {
            Ok(cursor_element) => elements.push(OutputRenderElements::Cursor(cursor_element)),
            Err(e) => tracing::warn!("failed to create cursor render element: {e}"),
        }
    }

    let tb_h = taskbar_height(s);
    let btn_w = taskbar_btn_width(s);
    let btn_h = taskbar_btn_height(s);
    let btn_gap = taskbar_btn_gap(s);
    let btn_margin = taskbar_btn_margin(s);

    let output_size = state
        .output
        .current_mode()
        .map(|m| m.size)
        .unwrap_or((1920, 1080).into());
    let output_w = output_size.w;
    let output_h = output_size.h;
    let taskbar_y = (output_h - tb_h) as f64;

    let focused_surface = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
    let mut desired: Vec<(String, bool, bool)> = Vec::new();
    for window in &state.window_order {
        let is_minimized = state.minimized_windows.iter().any(|(w, _)| w == window);
        let is_visible = state.space.elements().any(|w| w == window);
        if !is_minimized && !is_visible {
            continue;
        }
        let title = get_window_title(window);
        let label = if title.is_empty() {
            "Window".to_string()
        } else {
            title
        };
        if is_minimized {
            desired.push((label, false, true));
        } else {
            let is_focused = window
                .toplevel()
                .map(|t| focused_surface.as_ref() == Some(t.wl_surface()))
                .unwrap_or(false);
            desired.push((label, is_focused, false));
        }
    }
    let taskbar_changed = desired.len() != state.taskbar_buttons.len()
        || desired.iter().zip(state.taskbar_buttons.iter()).any(
            |((label, focused, minimized), (cur_label, cur_focused, cur_minimized, _))| {
                label != cur_label || focused != cur_focused || minimized != cur_minimized
            },
        );
    if taskbar_changed {
        let mut new_buttons: Vec<(String, bool, bool, MemoryRenderBuffer)> = Vec::new();
        let font_size = 13.0 * s as f32;
        let text_pad = 8 * s;
        for (label, is_focused, is_minimized) in desired {
            let (r, g, b) = if is_minimized {
                (35, 35, 35)
            } else if is_focused {
                (80, 80, 120)
            } else {
                (50, 50, 50)
            };
            let btn_buf =
                create_label_buffer(btn_w, btn_h, r, g, b, &label, font_size, text_pad, s);
            new_buttons.push((label, is_focused, is_minimized, btn_buf));
        }
        state.taskbar_buttons = new_buttons;
        state.taskbar_button_ids = (0..state.taskbar_buttons.len())
            .map(|_| Id::new())
            .collect();
    }

    for (i, (_, _, _, btn_buf)) in state.taskbar_buttons.iter().enumerate() {
        let x = (btn_margin + i as i32 * (btn_w + btn_gap)) as f64;
        let y = taskbar_y + ((tb_h - btn_h) / 2) as f64;
        let _ = btn_buf;
        let id = state
            .taskbar_button_ids
            .get(i)
            .cloned()
            .unwrap_or_else(Id::new);
        push_solid(
            &mut elements,
            id,
            x,
            y,
            btn_w,
            btn_h,
            [0.31, 0.31, 0.47, 1.0],
            Kind::Unspecified,
        );
    }

    push_solid(
        &mut elements,
        state.taskbar_bg_id.clone(),
        0.0,
        taskbar_y,
        output_w,
        tb_h,
        [0.12, 0.12, 0.12, 1.0],
        Kind::Unspecified,
    );

    let windows: Vec<Window> = state.space.elements().rev().cloned().collect();
    state.ssd_titlebar_buffers.clear();
    state
        .ssd_titlebar_ids
        .retain(|(window, _)| windows.iter().any(|current| current == window));

    let s_f64 = s as f64;
    let output_scale = state.output.current_scale().fractional_scale();
    let skip_client_surfaces = std::env::var_os("APC_DEBUG_SKIP_CLIENT_SURFACES").is_some();
    for window in &windows {
        if state.is_ssd(window) {
            if let Some(loc) = state.space.element_location(window) {
                let bar_x = loc.x as f64 * s_f64;
                let bar_y = (loc.y - SSD_TITLE_BAR_HEIGHT) as f64 * s_f64;
                let titlebar_id = solid_id_for_window(&mut state.ssd_titlebar_ids, window);
                push_solid(
                    &mut elements,
                    titlebar_id,
                    bar_x,
                    bar_y,
                    window.geometry().size.w * s,
                    SSD_TITLE_BAR_HEIGHT * s,
                    [0.20, 0.20, 0.24, 1.0],
                    Kind::Unspecified,
                );
            }
        }

        if !skip_client_surfaces {
            let loc = state.space.element_location(window).unwrap_or_default();
            let physical_loc = loc.to_physical_precise_round(output_scale);
            let win_elems: Vec<
                SpaceRenderElements<PixmanRenderer, WaylandSurfaceRenderElement<PixmanRenderer>>,
            > = window.render_elements(
                &mut state.renderer,
                physical_loc,
                Scale::from(output_scale),
                1.0,
            );
            elements.extend(win_elems.into_iter().map(OutputRenderElements::Space));
        }
    }

    let clear_color = [0.1, 0.1, 0.3, 1.0];
    if trace_frame {
        tracing::info!(frame, elements = elements.len(), "render elements ready");
    }

    match state.drm_compositor.render_frame(
        &mut state.renderer,
        &elements,
        clear_color,
        FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT,
    ) {
        Ok(result) => {
            if state.screen_snapshot_requested {
                state.screen_snapshot_requested = false;
                if let PrimaryPlaneElement::Swapchain(primary) = &result.primary_element {
                    match snapshot_primary_plane(state, primary) {
                        Ok(snapshot) => state.last_screen_snapshot = Some(snapshot),
                        Err(e) => tracing::warn!(%e, "failed to snapshot rendered primary plane"),
                    }
                }
            }

            if trace_frame {
                tracing::info!(
                    frame,
                    is_empty = result.is_empty,
                    needs_sync = result.needs_sync(),
                    "drm render_frame done",
                );
            }
            if result.needs_sync() {
                static EXPLICIT_SYNC_WARNED: AtomicBool = AtomicBool::new(false);
                if !EXPLICIT_SYNC_WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!(
                        "render result requested explicit sync; waiting before legacy commit"
                    );
                }
                if let PrimaryPlaneElement::Swapchain(primary) = &result.primary_element {
                    wait_for_primary_sync(&primary.sync, frame, trace_frame);
                }
            }
            if !result.is_empty {
                if std::env::var_os("SMITHAY_USE_LEGACY").is_some() {
                    if trace_frame {
                        tracing::info!(frame, "legacy commit_frame begin");
                    }
                    match state.drm_compositor.commit_frame() {
                        Ok(()) => {
                            if trace_frame {
                                tracing::info!(frame, "legacy commit_frame done");
                            }
                            mark_frame_presented();
                            state.redraw_state = RedrawState::Idle;
                            send_frame_callbacks(state);
                        }
                        Err(e) => {
                            tracing::error!("legacy commit_frame failed: {e}");
                            state.redraw_state = RedrawState::Idle;
                        }
                    }
                } else {
                    if trace_frame {
                        tracing::info!(frame, "queue_frame begin");
                    }
                    match state.drm_compositor.queue_frame(()) {
                        Ok(()) => {
                            if trace_frame {
                                tracing::info!(frame, "queue_frame done");
                            }
                            mark_frame_presented();
                            state.redraw_state = RedrawState::WaitingForVBlank {
                                redraw_needed: false,
                                submitted_at: Instant::now(),
                            };
                        }
                        Err(e) => {
                            tracing::error!("queue_frame failed: {e}");
                            state.redraw_state = RedrawState::Idle;
                        }
                    }
                }
            } else {
                state.redraw_state = RedrawState::Idle;
                send_frame_callbacks(state);
            }
        }
        Err(e) => {
            tracing::error!("render failed: {e}");
            state.redraw_state = RedrawState::Idle;
        }
    }
}

#[cfg(target_os = "linux")]
fn snapshot_primary_plane(
    state: &mut ApcCompositor,
    primary: &PrimarySwapchainElement<DumbBuffer, DumbFramebuffer>,
) -> Result<ScreenSnapshot> {
    state
        .renderer
        .wait(&primary.sync)
        .map_err(|e| anyhow::anyhow!("wait for primary sync: {e}"))?;

    let mut dmabuf = primary
        .buffer()
        .export()
        .map_err(|e| anyhow::anyhow!("export primary plane dmabuf: {e}"))?;
    let size = dmabuf.size();
    let width = size.w.max(1) as u32;
    let height = size.h.max(1) as u32;
    let row_len = width as usize * 4;

    let framebuffer = state
        .renderer
        .bind(&mut dmabuf)
        .map_err(|e| anyhow::anyhow!("bind primary plane dmabuf: {e}"))?;
    let mapping = state
        .renderer
        .copy_framebuffer(
            &framebuffer,
            Rectangle::from_size(size),
            DrmFourcc::Abgr8888,
        )
        .map_err(|e| anyhow::anyhow!("copy primary plane framebuffer: {e}"))?;
    let mapped = state
        .renderer
        .map_texture(&mapping)
        .map_err(|e| anyhow::anyhow!("map primary plane framebuffer copy: {e}"))?;

    let stride = mapped.len() / height as usize;
    if stride < row_len {
        anyhow::bail!("mapped framebuffer stride too small: stride={stride}, row_len={row_len}");
    }

    let mut pixels_rgba = Vec::with_capacity(row_len * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        pixels_rgba.extend_from_slice(&mapped[start..start + row_len]);
    }

    Ok(ScreenSnapshot {
        width,
        height,
        pixels_rgba,
    })
}

#[cfg(target_os = "linux")]
fn wait_for_primary_sync(sync: &SyncPoint, frame: u64, trace_frame: bool) {
    let started = Instant::now();
    if let Err(e) = sync.wait() {
        tracing::warn!(frame, ?e, "primary render sync wait interrupted");
        return;
    }

    let waited = started.elapsed();
    if waited >= Duration::from_millis(16) {
        tracing::warn!(
            frame,
            waited_ms = waited.as_millis(),
            "primary render sync wait was slow"
        );
    } else if trace_frame {
        tracing::info!(
            frame,
            waited_us = waited.as_micros(),
            "primary render sync reached"
        );
    }
}

#[cfg(target_os = "linux")]
fn push_solid(
    elements: &mut Vec<
        OutputRenderElements<PixmanRenderer, WaylandSurfaceRenderElement<PixmanRenderer>>,
    >,
    id: Id,
    x: f64,
    y: f64,
    width: i32,
    height: i32,
    color: [f32; 4],
    kind: Kind,
) {
    let rect: Rectangle<i32, Physical> = Rectangle::new(
        (x.round() as i32, y.round() as i32).into(),
        (width.max(1), height.max(1)).into(),
    );
    elements.push(OutputRenderElements::Solid(SolidColorRenderElement::new(
        id, rect, 0usize, color, kind,
    )));
}

#[cfg(target_os = "linux")]
fn solid_id_for_window(ids: &mut Vec<(Window, Id)>, window: &Window) -> Id {
    if let Some((_, id)) = ids.iter().find(|(existing, _)| existing == window) {
        return id.clone();
    }

    let id = Id::new();
    ids.push((window.clone(), id.clone()));
    id
}

#[cfg(target_os = "linux")]
fn mark_frame_presented() {
    static FIRST_FRAME_RENDERED: AtomicBool = AtomicBool::new(false);
    if !FIRST_FRAME_RENDERED.swap(true, Ordering::Relaxed) {
        tracing::info!("startup: first frame rendered");
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn send_frame_callbacks(state: &mut ApcCompositor) {
    let time = state.start_time.elapsed();
    let output = &state.output;
    state.space.elements().for_each(|window| {
        window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
            Some(output.clone())
        });
    });
}

#[cfg(target_os = "linux")]
pub(crate) fn capture_screen(
    state: &mut ApcCompositor,
    region: Option<apc_protocol::Rect>,
    scale: Option<f32>,
) -> Result<(u32, u32, String)> {
    use base64::Engine;

    state.screen_snapshot_requested = true;
    render_frame(state);
    let snapshot = state
        .last_screen_snapshot
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no rendered screen frame available"))?;
    let snapshot = apc_protocol::screenshot::apply_capture_options(
        snapshot.width,
        snapshot.height,
        &snapshot.pixels_rgba,
        region,
        scale,
    )
    .map_err(|e| anyhow::anyhow!(e))?;

    let mut png_buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_buf, snapshot.width, snapshot.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| anyhow::anyhow!("png header: {e}"))?;
        writer
            .write_image_data(&snapshot.pixels_rgba)
            .map_err(|e| anyhow::anyhow!("png write: {e}"))?;
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
    Ok((snapshot.width, snapshot.height, b64))
}
