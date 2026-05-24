#[cfg(target_os = "linux")]
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, Event, InputEvent, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent as PointerMotionEventTrait,
    },
    backend::libinput::LibinputInputBackend,
    desktop::{Window, WindowSurfaceType},
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

#[cfg(target_os = "linux")]
use super::grabs::ResizeSurfaceGrab;

#[cfg(target_os = "linux")]
use super::render::{queue_redraw, taskbar_height, taskbar_btn_width, taskbar_btn_gap, taskbar_btn_margin};
#[cfg(target_os = "linux")]
use super::state::AgentCompositor;

#[cfg(target_os = "linux")]
const RESIZE_EDGE_THRESHOLD: f64 = 8.0;

#[cfg(target_os = "linux")]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorShape {
    #[default]
    Default,
    ResizeNWSE,
    ResizeNESW,
    ResizeNS,
    ResizeEW,
}

#[cfg(target_os = "linux")]
pub(crate) fn edges_to_cursor_shape(edges: u32) -> CursorShape {
    let top = edges & 1 != 0;
    let bottom = edges & 2 != 0;
    let left = edges & 4 != 0;
    let right = edges & 8 != 0;
    match (top, bottom, left, right) {
        (true, false, true, false) => CursorShape::ResizeNWSE,
        (false, true, false, true) => CursorShape::ResizeNWSE,
        (true, false, false, true) => CursorShape::ResizeNESW,
        (false, true, true, false) => CursorShape::ResizeNESW,
        (true, false, false, false) | (false, true, false, false) => CursorShape::ResizeNS,
        (false, false, true, false) | (false, false, false, true) => CursorShape::ResizeEW,
        _ => CursorShape::Default,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn minimize_window(state: &mut AgentCompositor, window: &Window) {
    let loc = state.space.element_location(window).unwrap_or_default();
    state.space.unmap_elem(window);
    state.minimized_windows.push((window.clone(), loc));
    if let Some(keyboard) = state.seat.get_keyboard() {
        let next_focus = state.space.elements().next().and_then(|w| {
            w.toplevel().map(|t| t.wl_surface().clone())
        });
        keyboard.set_focus(state, next_focus, SERIAL_COUNTER.next_serial());
    }
    queue_redraw(state);
}

#[cfg(target_os = "linux")]
pub(crate) fn unminimize_window(state: &mut AgentCompositor, idx: usize) {
    let (window, loc) = state.minimized_windows.remove(idx);
    state.space.map_element(window.clone(), loc, true);
    state.space.raise_element(&window, true);
    if let Some(keyboard) = state.seat.get_keyboard() {
        let surface = window.toplevel().map(|t| t.wl_surface().clone());
        keyboard.set_focus(state, surface, SERIAL_COUNTER.next_serial());
    }
    queue_redraw(state);
}

#[cfg(target_os = "linux")]
fn near_any_window_edge(state: &AgentCompositor, location: Point<f64, Logical>) -> bool {
    for window in state.space.elements() {
        let loc = match state.space.element_location(window) {
            Some(l) => l,
            None => continue,
        };
        let geo = window.geometry();
        let bx = (loc.x + geo.loc.x) as f64;
        let by = (loc.y + geo.loc.y) as f64;
        let bw = geo.size.w as f64;
        let bh = geo.size.h as f64;
        let dx_left = (location.x - bx).abs();
        let dx_right = ((bx + bw) - location.x).abs();
        let dy_top = (location.y - by).abs();
        let dy_bottom = ((by + bh) - location.y).abs();
        if dx_left < RESIZE_EDGE_THRESHOLD || dx_right < RESIZE_EDGE_THRESHOLD
            || dy_top < RESIZE_EDGE_THRESHOLD || dy_bottom < RESIZE_EDGE_THRESHOLD
        {
            return true;
        }
        if location.x > bx && location.x < bx + bw && location.y > by && location.y < by + bh {
            return false;
        }
    }
    false
}

pub(crate) fn detect_resize_edge(
    state: &AgentCompositor,
    location: Point<f64, Logical>,
) -> Option<(Window, u32)> {
    let windows: Vec<&Window> = state.space.elements().collect();
    for window in windows.iter().rev() {
        let loc = state.space.element_location(*window)?;
        let geo = window.geometry();
        let x = location.x;
        let y = location.y;
        let bx = (loc.x + geo.loc.x) as f64;
        let by = (loc.y + geo.loc.y) as f64;
        let bw = geo.size.w as f64;
        let bh = geo.size.h as f64;

        let dx_left = x - bx;
        let dx_right = (bx + bw) - x;
        let dy_top = y - by;
        let dy_bottom = (by + bh) - y;

        if dx_left < -RESIZE_EDGE_THRESHOLD
            || dx_right < -RESIZE_EDGE_THRESHOLD
            || dy_top < -RESIZE_EDGE_THRESHOLD
            || dy_bottom < -RESIZE_EDGE_THRESHOLD
        {
            continue;
        }

        let on_left = dx_left.abs() < RESIZE_EDGE_THRESHOLD;
        let on_right = dx_right.abs() < RESIZE_EDGE_THRESHOLD;
        let on_top = dy_top.abs() < RESIZE_EDGE_THRESHOLD;
        let on_bottom = dy_bottom.abs() < RESIZE_EDGE_THRESHOLD;

        if on_left || on_right || on_top || on_bottom {
            let mut edges = 0u32;
            if on_top { edges |= 1; }
            if on_bottom { edges |= 2; }
            if on_left { edges |= 4; }
            if on_right { edges |= 8; }
            return Some(((*window).clone(), edges));
        }

        let inside = dx_left > RESIZE_EDGE_THRESHOLD
            && dx_right > RESIZE_EDGE_THRESHOLD
            && dy_top > RESIZE_EDGE_THRESHOLD
            && dy_bottom > RESIZE_EDGE_THRESHOLD;
        if inside {
            return None;
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(crate) fn handle_input(state: &mut AgentCompositor, event: InputEvent<LibinputInputBackend>) {
    match event {
        InputEvent::Keyboard { event, .. } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                keyboard.input::<(), _>(
                    state,
                    event.key_code(),
                    event.state(),
                    SERIAL_COUNTER.next_serial(),
                    event.time_msec(),
                    |_, _, _| FilterResult::Forward,
                );
            }
        }
        InputEvent::PointerMotionAbsolute { event, .. } => {
            let output_geo = state.space.output_geometry(&state.output);
            if let Some(geo) = output_geo {
                let pos = event.position_transformed(geo.size);
                let serial = SERIAL_COUNTER.next_serial();
                let pointer = state.pointer.clone();

                let under = state
                    .space
                    .element_under(pos.to_f64())
                    .and_then(|(window, loc)| {
                        window
                            .surface_under(
                                pos.to_f64() - loc.to_f64(),
                                WindowSurfaceType::ALL,
                            )
                            .map(|(s, surf_loc)| (s, (surf_loc + loc).to_f64()))
                    });

                pointer.motion(
                    state,
                    under,
                    &MotionEvent {
                        location: pos.to_f64(),
                        serial,
                        time: event.time_msec(),
                    },
                );
                if near_any_window_edge(state, pos.to_f64()) || state.cursor_shape != CursorShape::Default {
                    let new_shape = detect_resize_edge(state, pos.to_f64())
                        .map(|(_, edges)| edges_to_cursor_shape(edges))
                        .unwrap_or(CursorShape::Default);
                    if new_shape != state.cursor_shape {
                        state.cursor_shape = new_shape;
                    }
                } else if state.cursor_shape != CursorShape::Default {
                    state.cursor_shape = CursorShape::Default;
                }
                queue_redraw(state);
            }
        }
        InputEvent::PointerButton { event, .. } => {
            let serial = SERIAL_COUNTER.next_serial();
            let pointer = state.pointer.clone();
            let button_code = event.button_code();
            let is_left = button_code == 0x110;

            if is_left && event.state() == smithay::backend::input::ButtonState::Pressed {
                let location = pointer.current_location();
                let s = state.scale_factor;
                let logical_h = state.output.current_mode().map(|m| m.size.h).unwrap_or(1080) as f64 / s as f64;
                let taskbar_y = logical_h - taskbar_height(1) as f64;

                if location.y >= taskbar_y {
                    let btn_x = location.x - taskbar_btn_margin(1) as f64;
                    let idx = (btn_x / (taskbar_btn_width(1) + taskbar_btn_gap(1)) as f64) as usize;
                    let visible: Vec<Window> = state.space.elements().cloned().collect();
                    let visible_count = visible.len();
                    if idx < visible_count {
                        let window = &visible[idx];
                        let is_focused = window
                            .toplevel()
                            .map(|t| {
                                state.seat.get_keyboard()
                                    .and_then(|kb| kb.current_focus())
                                    .as_ref() == Some(t.wl_surface())
                            })
                            .unwrap_or(false);
                        if is_focused {
                            minimize_window(state, window);
                        } else {
                            state.space.raise_element(window, true);
                            if let Some(keyboard) = state.seat.get_keyboard() {
                                let surface = window.toplevel().map(|t| t.wl_surface().clone());
                                keyboard.set_focus(state, surface, serial);
                            }
                            queue_redraw(state);
                        }
                    } else {
                        let min_idx = idx - visible_count;
                        if min_idx < state.minimized_windows.len() {
                            unminimize_window(state, min_idx);
                        }
                    }
                    return;
                } else if let Some((window, edges)) = detect_resize_edge(state, location) {
                    let initial_loc = state.space.element_location(&window).unwrap_or_default();
                    let initial_size = window
                        .toplevel()
                        .and_then(|t| t.current_state().size)
                        .unwrap_or((800, 600).into());
                    let pointer = state.pointer.clone();
                    let start_data = GrabStartData {
                        focus: None,
                        button: button_code,
                        location,
                    };
                    let grab = ResizeSurfaceGrab {
                        window,
                        start_data,
                        edges,
                        initial_size,
                        initial_loc,
                    };
                    pointer.set_grab(state, grab, serial, Focus::Clear);
                    return;
                }
            }

            if event.state() == smithay::backend::input::ButtonState::Pressed {
                let location = pointer.current_location();
                let window = state
                    .space
                    .element_under(location)
                    .map(|(w, _)| w.clone());
                if let Some(window) = &window {
                    state.space.raise_element(window, true);
                    if let Some(keyboard) = state.seat.get_keyboard() {
                        let surface =
                            window.toplevel().map(|t| t.wl_surface().clone());
                        keyboard.set_focus(state, surface, serial);
                    }
                } else if let Some(keyboard) = state.seat.get_keyboard() {
                    keyboard.set_focus(state, None, serial);
                }
            }

            pointer.button(
                state,
                &ButtonEvent {
                    serial,
                    time: event.time_msec(),
                    button: button_code,
                    state: event.state(),
                },
            );
        }
        InputEvent::PointerMotion { event, .. } => {
            let pointer = state.pointer.clone();
            let current = pointer.current_location();
            let output_geo = state.space.output_geometry(&state.output);
            if let Some(geo) = output_geo {
                let dx = event.delta_x();
                let dy = event.delta_y();
                let new_x = (current.x + dx).clamp(0.0, geo.size.w as f64 - 1.0);
                let new_y = (current.y + dy).clamp(0.0, geo.size.h as f64 - 1.0);
                let pos = (new_x, new_y).into();
                let serial = SERIAL_COUNTER.next_serial();

                let under = state
                    .space
                    .element_under(pos)
                    .and_then(|(window, loc)| {
                        window
                            .surface_under(
                                pos - loc.to_f64(),
                                WindowSurfaceType::ALL,
                            )
                            .map(|(s, surf_loc)| (s, (surf_loc + loc).to_f64()))
                    });

                pointer.motion(
                    state,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                if near_any_window_edge(state, pos) || state.cursor_shape != CursorShape::Default {
                    let new_shape = detect_resize_edge(state, pos)
                        .map(|(_, edges)| edges_to_cursor_shape(edges))
                        .unwrap_or(CursorShape::Default);
                    if new_shape != state.cursor_shape {
                        state.cursor_shape = new_shape;
                    }
                } else if state.cursor_shape != CursorShape::Default {
                    state.cursor_shape = CursorShape::Default;
                }
                queue_redraw(state);
            }
        }
        InputEvent::PointerAxis { event, .. } => {
            let pointer = state.pointer.clone();
            let source = event.source();
            let mut frame = AxisFrame::new(event.time_msec()).source(source);
            for axis in [Axis::Vertical, Axis::Horizontal] {
                let v120 = event.amount_v120(axis);
                let amount = event.amount(axis).or_else(|| {
                    v120.map(|v| v / 120.0 * 15.0)
                });
                if let Some(val) = amount {
                    frame = frame.value(axis, val);
                }
                if let Some(discrete) = v120 {
                    frame = frame.v120(axis, discrete as i32);
                }
            }
            pointer.axis(state, frame);
            pointer.frame(state);
        }
        _ => {}
    }
}
