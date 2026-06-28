#[cfg(target_os = "linux")]
use smithay::{
    desktop::{Window, WindowSurfaceType},
    input::{
        keyboard::FilterResult,
        pointer::{ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::Display,
    utils::{Logical, Point, SERIAL_COUNTER},
};

#[cfg(target_os = "linux")]
use apc_protocol::{ToolCall, WindowInfo};

#[cfg(target_os = "linux")]
use std::sync::mpsc;

#[cfg(target_os = "linux")]
use super::state::ApcCompositor;

#[cfg(target_os = "linux")]
use super::render::queue_redraw;

#[cfg(target_os = "linux")]
use super::render::capture_screen;

#[cfg(target_os = "linux")]
use super::input::{minimize_window, unminimize_window};

#[cfg(target_os = "linux")]
use super::taskbar::get_window_title;

#[cfg(target_os = "linux")]
pub(crate) fn handle_mcp_tool(
    state: &mut ApcCompositor,
    _display: &mut Display<ApcCompositor>,
    tool: ToolCall,
    reply_tx: mpsc::SyncSender<serde_json::Value>,
) -> Option<serde_json::Value> {
    match tool {
        ToolCall::ShellExec { ref cmd } => {
            let cmd = cmd.clone();
            let wayland_display = state.wayland_display.clone();
            let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
            std::thread::spawn(move || {
                let result = std::process::Command::new("sh")
                    .args(["-lc", &cmd])
                    .env("HOME", "/home/agent")
                    .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
                    .env("WAYLAND_DISPLAY", &wayland_display)
                    .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
                    .output();
                let value = match result {
                    Ok(out) => serde_json::json!({
                        "exit_code": out.status.code().unwrap_or(-1),
                        "stdout": String::from_utf8_lossy(&out.stdout).to_string(),
                        "stderr": String::from_utf8_lossy(&out.stderr).to_string(),
                    }),
                    Err(e) => serde_json::json!({
                        "exit_code": -1,
                        "stdout": "",
                        "stderr": format!("shell exec failed: {e}"),
                    }),
                };
                let _ = reply_tx.send(value);
            });
            return None;
        }

        ToolCall::FileRead { ref path, line } => {
            let data = match std::fs::read_to_string(path) {
                Ok(d) => d,
                Err(e) => {
                    return Some(serde_json::json!({ "error": format!("read failed: {e}") }));
                }
            };
            let size = data.len();

            open_in_editor(state, path, line);

            Some(serde_json::json!({
                "data": data,
                "size": size,
            }))
        }

        ToolCall::FileWrite {
            ref path,
            ref data,
            offset,
        } => {
            if let Err(e) = std::fs::write(path, data) {
                return Some(serde_json::json!({ "error": format!("write failed: {e}") }));
            }
            let written = data.len();

            open_in_editor(state, path, offset);

            Some(serde_json::json!({ "written": written }))
        }

        ToolCall::FsMount {
            ref host_path,
            ref guest_path,
        } => {
            let hp = host_path.clone();
            let gp = guest_path.clone();
            let fs_port = apc_protocol::fs::VSOCK_FS_PORT;
            std::thread::spawn(move || {
                let _ = std::fs::create_dir_all(&gp);
                let result = std::process::Command::new("/usr/local/bin/apc-fuse")
                    .args([
                        "--host-path",
                        &hp,
                        "--mount-point",
                        &gp,
                        "--port",
                        &fs_port.to_string(),
                    ])
                    .spawn();
                match result {
                    Ok(mut child) => {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        match child.try_wait() {
                            Ok(Some(status)) if !status.success() => {
                                let _ = reply_tx.send(serde_json::json!({
                                    "error": format!("fuse mount failed: exit {status}")
                                }));
                            }
                            _ => {
                                let _ = reply_tx.send(serde_json::json!({
                                    "mounted": true,
                                    "host_path": hp,
                                    "guest_path": gp,
                                }));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = reply_tx.send(serde_json::json!({
                            "error": format!("failed to launch apc-fuse: {e}")
                        }));
                    }
                }
            });
            return None;
        }

        ToolCall::FsUnmount { ref guest_path } => {
            let gp = guest_path.clone();
            match std::process::Command::new("fusermount3")
                .args(["-u", &gp])
                .output()
            {
                Ok(out) if out.status.success() => {
                    Some(serde_json::json!({ "unmounted": true, "guest_path": gp }))
                }
                Ok(out) => Some(serde_json::json!({
                    "error": format!(
                        "fusermount3 failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    )
                })),
                Err(e) => Some(serde_json::json!({
                    "error": format!("fusermount3 exec failed: {e}")
                })),
            }
        }

        other => Some(handle_sync_tool(state, other)),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn handle_browser_tool(
    state: &mut ApcCompositor,
    name: String,
    arguments: serde_json::Value,
    reply_tx: mpsc::SyncSender<serde_json::Value>,
) {
    state.browser.handle_tool_async(name, arguments, reply_tx);
}

#[cfg(target_os = "linux")]
fn handle_sync_tool(state: &mut ApcCompositor, tool: ToolCall) -> serde_json::Value {
    match tool {
        ToolCall::WindowList => {
            let focused_surface = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
            let mut windows: Vec<WindowInfo> = state
                .space
                .elements()
                .enumerate()
                .map(|(i, window)| {
                    let loc = state.space.element_location(window).unwrap_or_default();
                    let size = window
                        .toplevel()
                        .and_then(|t| t.current_state().size)
                        .unwrap_or_default();
                    let is_focused = window
                        .toplevel()
                        .map(|t| focused_surface.as_ref() == Some(t.wl_surface()))
                        .unwrap_or(false);
                    let title = get_window_title(window);
                    WindowInfo {
                        id: i as u64,
                        title,
                        x: loc.x,
                        y: loc.y,
                        width: size.w as u32,
                        height: size.h as u32,
                        focused: is_focused,
                        minimized: false,
                    }
                })
                .collect();
            let base_id = windows.len();
            for (i, (window, loc)) in state.minimized_windows.iter().enumerate() {
                let size = window
                    .toplevel()
                    .and_then(|t| t.current_state().size)
                    .unwrap_or_default();
                let title = get_window_title(window);
                windows.push(WindowInfo {
                    id: (base_id + i) as u64,
                    title,
                    x: loc.x,
                    y: loc.y,
                    width: size.w as u32,
                    height: size.h as u32,
                    focused: false,
                    minimized: true,
                });
            }
            serde_json::json!({ "windows": windows })
        }

        ToolCall::WindowFocus { id } => {
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            let visible_count = windows.len();
            if let Some(window) = windows.get(id as usize) {
                state.space.raise_element(window, true);
                if let Some(keyboard) = state.seat.get_keyboard() {
                    let surface = window.toplevel().map(|t| t.wl_surface().clone());
                    keyboard.set_focus(state, surface, SERIAL_COUNTER.next_serial());
                }
                queue_redraw(state);
                serde_json::json!({ "focused": id })
            } else {
                let min_idx = id as usize - visible_count;
                if min_idx < state.minimized_windows.len() {
                    unminimize_window(state, min_idx);
                    serde_json::json!({ "focused": id })
                } else {
                    serde_json::json!({ "error": "window not found" })
                }
            }
        }

        ToolCall::WindowResize { id, width, height } => {
            tracing::info!(id, width, height, "MCP window_resize begin");
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            if let Some(window) = windows.get(id as usize) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|s| {
                        s.size = Some((width as i32, height as i32).into());
                    });
                    toplevel.send_configure();
                    queue_redraw(state);
                }
                tracing::info!(id, "MCP window_resize requested");
                serde_json::json!({ "resize_requested": id, "width": width, "height": height })
            } else {
                tracing::info!(id, "MCP window_resize window not found");
                serde_json::json!({ "error": "window not found" })
            }
        }

        ToolCall::WindowMove { id, x, y } => {
            tracing::info!(id, x, y, "MCP window_move begin");
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            if let Some(window) = windows.get(id as usize) {
                state.space.map_element(window.clone(), (x, y), true);
                queue_redraw(state);
                tracing::info!(id, "MCP window_move done");
                serde_json::json!({ "moved": id })
            } else {
                tracing::info!(id, "MCP window_move window not found");
                serde_json::json!({ "error": "window not found" })
            }
        }

        ToolCall::WindowOpen { ref cmd } => {
            let wayland_display = state.wayland_display.clone();
            let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
            let mut child = match std::process::Command::new("sh")
                .args(["-lc", cmd])
                .env("HOME", "/home/agent")
                .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
                .env("WAYLAND_DISPLAY", &wayland_display)
                .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
                .env("TERM", "xterm-256color")
                .spawn()
            {
                Ok(child) => child,
                Err(e) => {
                    tracing::error!(cmd = %cmd, %e, "window_open failed");
                    return serde_json::json!({ "error": format!("window_open failed: {e}") });
                }
            };

            std::thread::sleep(std::time::Duration::from_millis(100));
            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::error!(cmd = %cmd, %status, "window_open exited before creating a window");
                    serde_json::json!({
                        "error": format!("window_open exited before creating a window: {status}")
                    })
                }
                Ok(None) => {
                    let pid = child.id();
                    state.mcp_pids.push(pid);
                    tracing::info!(cmd = %cmd, pid, "window_open launched");
                    serde_json::json!({ "launched": cmd, "pid": pid })
                }
                Err(e) => {
                    tracing::error!(cmd = %cmd, %e, "window_open status check failed");
                    serde_json::json!({ "error": format!("window_open status check failed: {e}") })
                }
            }
        }

        ToolCall::WindowClose { id } => {
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            let visible_count = windows.len();
            if let Some(window) = windows.get(id as usize) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                    queue_redraw(state);
                }
                serde_json::json!({ "close_requested": id })
            } else {
                let min_idx = id as usize - visible_count;
                if min_idx < state.minimized_windows.len() {
                    let (window, _) = &state.minimized_windows[min_idx];
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_close();
                    }
                    state.minimized_windows.remove(min_idx);
                    queue_redraw(state);
                    serde_json::json!({ "close_requested": id })
                } else {
                    serde_json::json!({ "error": "window not found" })
                }
            }
        }

        ToolCall::WindowMinimize { id } => {
            let visible: Vec<Window> = state.space.elements().cloned().collect();
            let visible_count = visible.len();
            if (id as usize) < visible_count {
                let window = &visible[id as usize];
                minimize_window(state, window);
                serde_json::json!({ "minimized": id })
            } else {
                let min_idx = id as usize - visible_count;
                if min_idx < state.minimized_windows.len() {
                    unminimize_window(state, min_idx);
                    serde_json::json!({ "unminimized": id })
                } else {
                    serde_json::json!({ "error": "window not found" })
                }
            }
        }

        ToolCall::MouseMove { x, y } => {
            let pointer = state.pointer.clone();
            let pos: Point<f64, Logical> = (x as f64, y as f64).into();
            let under = state.space.element_under(pos).and_then(|(window, loc)| {
                window
                    .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, surf_loc)| (s, (surf_loc + loc).to_f64()))
            });
            let serial = SERIAL_COUNTER.next_serial();
            pointer.motion(
                state,
                under,
                &MotionEvent {
                    location: pos,
                    serial,
                    time: state.start_time.elapsed().as_millis() as u32,
                },
            );
            state.finish_pointer_motion();
            serde_json::json!({ "moved": [x, y] })
        }

        ToolCall::MouseClick { button } => {
            let pointer = state.pointer.clone();
            let serial = SERIAL_COUNTER.next_serial();
            let time = state.start_time.elapsed().as_millis() as u32;
            let btn_code = match button {
                apc_protocol::MouseButton::Left => 0x110,
                apc_protocol::MouseButton::Right => 0x111,
                apc_protocol::MouseButton::Middle => 0x112,
            };

            // Set keyboard focus to window under pointer (matches normal input path)
            let location = pointer.current_location();
            if let Some((window, _)) = state.space.element_under(location) {
                let window = window.clone();
                state.space.raise_element(&window, true);
                if let Some(keyboard) = state.seat.get_keyboard() {
                    let surface = window.toplevel().map(|t| t.wl_surface().clone());
                    keyboard.set_focus(state, surface, serial);
                }
            }

            pointer.button(
                state,
                &ButtonEvent {
                    serial,
                    time,
                    button: btn_code,
                    state: smithay::backend::input::ButtonState::Pressed,
                },
            );
            let serial2 = SERIAL_COUNTER.next_serial();
            pointer.button(
                state,
                &ButtonEvent {
                    serial: serial2,
                    time: time + 50,
                    button: btn_code,
                    state: smithay::backend::input::ButtonState::Released,
                },
            );
            queue_redraw(state);
            serde_json::json!({ "clicked": format!("{button:?}") })
        }

        ToolCall::KeyboardType { ref text } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                let mut mapped_keys = Vec::new();
                for (index, ch) in text.chars().enumerate() {
                    let Some(mapped) = char_to_evdev_keycode(ch) else {
                        return serde_json::json!({
                            "error": format!(
                                "unsupported character at index {index}: U+{:04X}",
                                ch as u32
                            )
                        });
                    };
                    mapped_keys.push(mapped);
                }

                for (keycode, shift) in &mapped_keys {
                    let time = state.start_time.elapsed().as_millis() as u32;
                    let shift_xkb: u32 = 42 + 8;
                    if *shift {
                        keyboard.input::<(), _>(
                            state,
                            shift_xkb.into(),
                            smithay::backend::input::KeyState::Pressed,
                            SERIAL_COUNTER.next_serial(),
                            time,
                            |_, _, _| FilterResult::Forward,
                        );
                    }
                    keyboard.input::<(), _>(
                        state,
                        (*keycode).into(),
                        smithay::backend::input::KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        time + 1,
                        |_, _, _| FilterResult::Forward,
                    );
                    keyboard.input::<(), _>(
                        state,
                        (*keycode).into(),
                        smithay::backend::input::KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        time + 2,
                        |_, _, _| FilterResult::Forward,
                    );
                    if *shift {
                        keyboard.input::<(), _>(
                            state,
                            shift_xkb.into(),
                            smithay::backend::input::KeyState::Released,
                            SERIAL_COUNTER.next_serial(),
                            time + 3,
                            |_, _, _| FilterResult::Forward,
                        );
                    }
                }
                serde_json::json!({ "typed": mapped_keys.len(), "total": mapped_keys.len() })
            } else {
                serde_json::json!({ "error": "no keyboard" })
            }
        }

        ToolCall::KeyboardKey {
            ref key,
            ref modifiers,
        } => {
            let Some(keycode) = key_name_to_evdev(key) else {
                return serde_json::json!({ "error": format!("unsupported key: {key}") });
            };
            let mut mod_codes = Vec::new();
            for modifier in modifiers {
                let Some(code) = modifier_to_evdev(modifier) else {
                    return serde_json::json!({
                        "error": format!("unsupported modifier: {modifier}")
                    });
                };
                mod_codes.push(code);
            }

            if let Some(keyboard) = state.seat.get_keyboard() {
                let time = state.start_time.elapsed().as_millis() as u32;
                for &mc in &mod_codes {
                    keyboard.input::<(), _>(
                        state,
                        mc.into(),
                        smithay::backend::input::KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        time,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                keyboard.input::<(), _>(
                    state,
                    keycode.into(),
                    smithay::backend::input::KeyState::Pressed,
                    SERIAL_COUNTER.next_serial(),
                    time + 1,
                    |_, _, _| FilterResult::Forward,
                );
                keyboard.input::<(), _>(
                    state,
                    keycode.into(),
                    smithay::backend::input::KeyState::Released,
                    SERIAL_COUNTER.next_serial(),
                    time + 2,
                    |_, _, _| FilterResult::Forward,
                );
                for &mc in mod_codes.iter().rev() {
                    keyboard.input::<(), _>(
                        state,
                        mc.into(),
                        smithay::backend::input::KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        time + 3,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                serde_json::json!({ "key": key, "modifiers": modifiers })
            } else {
                serde_json::json!({ "error": "no keyboard" })
            }
        }

        ToolCall::ScreenCapture { region, scale } => match capture_screen(state, region, scale) {
            Ok((w, h, png_b64)) => {
                serde_json::json!({
                    "width": w,
                    "height": h,
                    "format": "png_base64",
                    "data": png_b64,
                })
            }
            Err(e) => {
                tracing::error!(%e, "screen capture failed");
                serde_json::json!({ "error": format!("capture failed: {e}") })
            }
        },

        _ => serde_json::json!({ "error": "unhandled tool" }),
    }
}

#[cfg(target_os = "linux")]
const NVIM_SOCKET: &str = "/tmp/nvim-mcp.sock";

#[cfg(target_os = "linux")]
fn open_in_editor(state: &mut ApcCompositor, path: &str, line: Option<u32>) {
    let wayland_display = state.wayland_display.clone();
    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));

    let nvim_running = std::path::Path::new(NVIM_SOCKET).exists();

    if nvim_running {
        let goto = line.unwrap_or(1);
        let nvim_cmd = format!(
            "nvim --server {} --remote-send '<Esc>:e {}<CR>:{}<CR>'",
            NVIM_SOCKET, path, goto
        );
        let _ = std::process::Command::new("sh")
            .args(["-c", &nvim_cmd])
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
            .spawn();
    } else {
        let goto_arg = line.map(|l| format!("+{l}")).unwrap_or_default();
        let mut args = vec![
            "-T".to_string(),
            format!("nvim - {path}"),
            "nvim".to_string(),
            "--listen".to_string(),
            NVIM_SOCKET.to_string(),
        ];
        if !goto_arg.is_empty() {
            args.push(goto_arg);
        }
        args.push(path.to_string());

        match std::process::Command::new("foot")
            .args(&args)
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
            .env("TERM", "xterm-256color")
            .spawn()
        {
            Ok(child) => {
                state.editor_pid = Some(child.id());
                state.mcp_pids.push(child.id());
            }
            Err(e) => {
                tracing::error!(%e, "failed to launch nvim");
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn char_to_evdev_keycode(ch: char) -> Option<(u32, bool)> {
    const OFF: u32 = 8;
    fn alpha(evdev: u32) -> u32 {
        evdev + 8
    }
    match ch {
        'a' => Some((alpha(30), false)),
        'b' => Some((alpha(48), false)),
        'c' => Some((alpha(46), false)),
        'd' => Some((alpha(32), false)),
        'e' => Some((alpha(18), false)),
        'f' => Some((alpha(33), false)),
        'g' => Some((alpha(34), false)),
        'h' => Some((alpha(35), false)),
        'i' => Some((alpha(23), false)),
        'j' => Some((alpha(36), false)),
        'k' => Some((alpha(37), false)),
        'l' => Some((alpha(38), false)),
        'm' => Some((alpha(50), false)),
        'n' => Some((alpha(49), false)),
        'o' => Some((alpha(24), false)),
        'p' => Some((alpha(25), false)),
        'q' => Some((alpha(16), false)),
        'r' => Some((alpha(19), false)),
        's' => Some((alpha(31), false)),
        't' => Some((alpha(20), false)),
        'u' => Some((alpha(22), false)),
        'v' => Some((alpha(47), false)),
        'w' => Some((alpha(17), false)),
        'x' => Some((alpha(45), false)),
        'y' => Some((alpha(21), false)),
        'z' => Some((alpha(44), false)),
        'A' => Some((alpha(30), true)),
        'B' => Some((alpha(48), true)),
        'C' => Some((alpha(46), true)),
        'D' => Some((alpha(32), true)),
        'E' => Some((alpha(18), true)),
        'F' => Some((alpha(33), true)),
        'G' => Some((alpha(34), true)),
        'H' => Some((alpha(35), true)),
        'I' => Some((alpha(23), true)),
        'J' => Some((alpha(36), true)),
        'K' => Some((alpha(37), true)),
        'L' => Some((alpha(38), true)),
        'M' => Some((alpha(50), true)),
        'N' => Some((alpha(49), true)),
        'O' => Some((alpha(24), true)),
        'P' => Some((alpha(25), true)),
        'Q' => Some((alpha(16), true)),
        'R' => Some((alpha(19), true)),
        'S' => Some((alpha(31), true)),
        'T' => Some((alpha(20), true)),
        'U' => Some((alpha(22), true)),
        'V' => Some((alpha(47), true)),
        'W' => Some((alpha(17), true)),
        'X' => Some((alpha(45), true)),
        'Y' => Some((alpha(21), true)),
        'Z' => Some((alpha(44), true)),
        '1' => Some((2 + OFF, false)),
        '2' => Some((3 + OFF, false)),
        '3' => Some((4 + OFF, false)),
        '4' => Some((5 + OFF, false)),
        '5' => Some((6 + OFF, false)),
        '6' => Some((7 + OFF, false)),
        '7' => Some((8 + OFF, false)),
        '8' => Some((9 + OFF, false)),
        '9' => Some((10 + OFF, false)),
        '0' => Some((11 + OFF, false)),
        '!' => Some((2 + OFF, true)),
        '@' => Some((3 + OFF, true)),
        '#' => Some((4 + OFF, true)),
        '$' => Some((5 + OFF, true)),
        '%' => Some((6 + OFF, true)),
        '^' => Some((7 + OFF, true)),
        '&' => Some((8 + OFF, true)),
        '*' => Some((9 + OFF, true)),
        '(' => Some((10 + OFF, true)),
        ')' => Some((11 + OFF, true)),
        ' ' => Some((57 + OFF, false)),
        '\n' => Some((28 + OFF, false)),
        '\t' => Some((15 + OFF, false)),
        '-' => Some((12 + OFF, false)),
        '_' => Some((12 + OFF, true)),
        '=' => Some((13 + OFF, false)),
        '+' => Some((13 + OFF, true)),
        '[' => Some((26 + OFF, false)),
        '{' => Some((26 + OFF, true)),
        ']' => Some((27 + OFF, false)),
        '}' => Some((27 + OFF, true)),
        '\\' => Some((43 + OFF, false)),
        '|' => Some((43 + OFF, true)),
        ';' => Some((39 + OFF, false)),
        ':' => Some((39 + OFF, true)),
        '\'' => Some((40 + OFF, false)),
        '"' => Some((40 + OFF, true)),
        '`' => Some((41 + OFF, false)),
        '~' => Some((41 + OFF, true)),
        ',' => Some((51 + OFF, false)),
        '<' => Some((51 + OFF, true)),
        '.' => Some((52 + OFF, false)),
        '>' => Some((52 + OFF, true)),
        '/' => Some((53 + OFF, false)),
        '?' => Some((53 + OFF, true)),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn modifier_to_evdev(name: &str) -> Option<u32> {
    const OFF: u32 = 8;
    match name.to_lowercase().as_str() {
        "shift" | "lshift" => Some(42 + OFF),
        "rshift" => Some(54 + OFF),
        "ctrl" | "control" | "lctrl" => Some(29 + OFF),
        "rctrl" => Some(97 + OFF),
        "alt" | "lalt" => Some(56 + OFF),
        "ralt" => Some(100 + OFF),
        "super" | "meta" | "win" => Some(125 + OFF),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn key_name_to_evdev(name: &str) -> Option<u32> {
    const OFF: u32 = 8;
    if name.len() == 1 {
        return char_to_evdev_keycode(name.chars().next().unwrap()).map(|(k, _)| k);
    }
    match name.to_lowercase().as_str() {
        "enter" | "return" => Some(28 + OFF),
        "escape" | "esc" => Some(1 + OFF),
        "backspace" => Some(14 + OFF),
        "tab" => Some(15 + OFF),
        "space" => Some(57 + OFF),
        "up" => Some(103 + OFF),
        "down" => Some(108 + OFF),
        "left" => Some(105 + OFF),
        "right" => Some(106 + OFF),
        "home" => Some(102 + OFF),
        "end" => Some(107 + OFF),
        "pageup" => Some(104 + OFF),
        "pagedown" => Some(109 + OFF),
        "insert" => Some(110 + OFF),
        "delete" => Some(111 + OFF),
        "f1" => Some(59 + OFF),
        "f2" => Some(60 + OFF),
        "f3" => Some(61 + OFF),
        "f4" => Some(62 + OFF),
        "f5" => Some(63 + OFF),
        "f6" => Some(64 + OFF),
        "f7" => Some(65 + OFF),
        "f8" => Some(66 + OFF),
        "f9" => Some(67 + OFF),
        "f10" => Some(68 + OFF),
        "f11" => Some(87 + OFF),
        "f12" => Some(88 + OFF),
        _ => None,
    }
}
