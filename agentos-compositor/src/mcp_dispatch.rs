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
use agentos_protocol::{ToolCall, WindowInfo};

#[cfg(target_os = "linux")]
use std::sync::mpsc;

#[cfg(target_os = "linux")]
use super::state::AgentCompositor;

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
    state: &mut AgentCompositor,
    _display: &mut Display<AgentCompositor>,
    tool: ToolCall,
    reply_tx: mpsc::SyncSender<serde_json::Value>,
) -> Option<serde_json::Value> {
    match tool {
        ToolCall::ShellExec { ref cmd } => {
            let id = state.start_time.elapsed().as_millis();
            let out_path = format!("/tmp/mcp-out-{id}");
            let wayland_display = state.wayland_display.clone();
            let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));

            let shell_cmd = format!(
                "{{ {cmd}; }} 2>&1 | tee {out_path}; echo $? > {out_path}.exit; sleep 2"
            );
            let title = if cmd.len() > 60 {
                format!("$ {}...", &cmd[..57])
            } else {
                format!("$ {cmd}")
            };

            let result = std::process::Command::new("alacritty")
                .args(["-T", &title, "-e", "sh", "-c", &shell_cmd])
                .env("WAYLAND_DISPLAY", &wayland_display)
                .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
                .env("TERM", "xterm-256color")
                .spawn();

            match result {
                Ok(mut child) => {
                    let pid = child.id();
                    state.mcp_pids.push(pid);
                    let out_path_clone = out_path.clone();
                    std::thread::spawn(move || {
                        let _ = child.wait();
                        let stdout = std::fs::read_to_string(&out_path_clone).unwrap_or_default();
                        let exit_file = format!("{out_path_clone}.exit");
                        let exit_code: i32 = std::fs::read_to_string(&exit_file)
                            .ok()
                            .and_then(|s| s.trim().parse().ok())
                            .unwrap_or(-1);
                        let _ = std::fs::remove_file(&out_path_clone);
                        let _ = std::fs::remove_file(&exit_file);
                        let _ = reply_tx.send(serde_json::json!({
                            "exit_code": exit_code,
                            "stdout": stdout,
                            "stderr": "",
                        }));
                    });
                    return None;
                }
                Err(e) => {
                    Some(serde_json::json!({ "error": format!("alacritty launch failed: {e}") }))
                }
            }
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

        ToolCall::FileWrite { ref path, ref data, offset } => {
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
            let fs_port = agentos_protocol::fs::VSOCK_FS_PORT;
            std::thread::spawn(move || {
                let result = std::process::Command::new("/usr/local/bin/agentos-fuse")
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
                            "error": format!("failed to launch agentos-fuse: {e}")
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
fn handle_sync_tool(
    state: &mut AgentCompositor,
    tool: ToolCall,
) -> serde_json::Value {
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
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            if let Some(window) = windows.get(id as usize) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.with_pending_state(|s| {
                        s.size = Some((width as i32, height as i32).into());
                    });
                    toplevel.send_configure();
                    queue_redraw(state);
                }
                serde_json::json!({ "resized": id })
            } else {
                serde_json::json!({ "error": "window not found" })
            }
        }

        ToolCall::WindowMove { id, x, y } => {
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            if let Some(window) = windows.get(id as usize) {
                state.space.map_element(window.clone(), (x, y), true);
                queue_redraw(state);
                serde_json::json!({ "moved": id })
            } else {
                serde_json::json!({ "error": "window not found" })
            }
        }

        ToolCall::WindowOpen { ref cmd } => {
            let wayland_display = state.wayland_display.clone();
            let cmd_clone = cmd.clone();
            let cmd_name = cmd.clone();
            let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
                format!("/run/user/{}", unsafe { libc::getuid() })
            });
            std::thread::spawn(move || {
                let result = std::process::Command::new("sh")
                    .args(["-c", &cmd_clone])
                    .env("WAYLAND_DISPLAY", &wayland_display)
                    .env("XDG_RUNTIME_DIR", &xdg_runtime_dir)
                    .env("TERM", "xterm-256color")
                    .spawn();
                match result {
                    Ok(_) => tracing::info!(cmd = %cmd_clone, "window_open launched"),
                    Err(e) => tracing::error!(cmd = %cmd_clone, %e, "window_open failed"),
                }
            });
            serde_json::json!({ "opened": cmd_name })
        }

        ToolCall::WindowClose { id } => {
            let windows: Vec<Window> = state.space.elements().cloned().collect();
            let visible_count = windows.len();
            if let Some(window) = windows.get(id as usize) {
                if let Some(toplevel) = window.toplevel() {
                    toplevel.send_close();
                    queue_redraw(state);
                }
                serde_json::json!({ "closed": id })
            } else {
                let min_idx = id as usize - visible_count;
                if min_idx < state.minimized_windows.len() {
                    let (window, _) = &state.minimized_windows[min_idx];
                    if let Some(toplevel) = window.toplevel() {
                        toplevel.send_close();
                    }
                    state.minimized_windows.remove(min_idx);
                    queue_redraw(state);
                    serde_json::json!({ "closed": id })
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
            let under = state
                .space
                .element_under(pos)
                .and_then(|(window, loc)| {
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
            queue_redraw(state);
            serde_json::json!({ "moved": [x, y] })
        }

        ToolCall::MouseClick { button } => {
            let pointer = state.pointer.clone();
            let serial = SERIAL_COUNTER.next_serial();
            let time = state.start_time.elapsed().as_millis() as u32;
            let btn_code = match button {
                agentos_protocol::MouseButton::Left => 0x110,
                agentos_protocol::MouseButton::Right => 0x111,
                agentos_protocol::MouseButton::Middle => 0x112,
            };
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
                let mut typed = 0u32;
                for ch in text.chars() {
                    if let Some((keycode, shift)) = char_to_evdev_keycode(ch) {
                        let time = state.start_time.elapsed().as_millis() as u32;
                        let shift_xkb: u32 = 42 + 8;
                        if shift {
                            keyboard.input::<(), _>(
                                state, shift_xkb.into(),
                                smithay::backend::input::KeyState::Pressed,
                                SERIAL_COUNTER.next_serial(),
                                time,
                                |_, _, _| FilterResult::Forward,
                            );
                        }
                        keyboard.input::<(), _>(
                            state, keycode.into(),
                            smithay::backend::input::KeyState::Pressed,
                            SERIAL_COUNTER.next_serial(),
                            time + 1,
                            |_, _, _| FilterResult::Forward,
                        );
                        keyboard.input::<(), _>(
                            state, keycode.into(),
                            smithay::backend::input::KeyState::Released,
                            SERIAL_COUNTER.next_serial(),
                            time + 2,
                            |_, _, _| FilterResult::Forward,
                        );
                        if shift {
                            keyboard.input::<(), _>(
                                state, shift_xkb.into(),
                                smithay::backend::input::KeyState::Released,
                                SERIAL_COUNTER.next_serial(),
                                time + 3,
                                |_, _, _| FilterResult::Forward,
                            );
                        }
                        typed += 1;
                    }
                }
                serde_json::json!({ "typed": typed, "total": text.len() })
            } else {
                serde_json::json!({ "error": "no keyboard" })
            }
        }

        ToolCall::KeyboardKey { ref key, ref modifiers } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                let time = state.start_time.elapsed().as_millis() as u32;
                let mod_codes: Vec<u32> = modifiers.iter().filter_map(|m| modifier_to_evdev(m)).collect();
                for &mc in &mod_codes {
                    keyboard.input::<(), _>(
                        state, mc.into(),
                        smithay::backend::input::KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        time,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                if let Some(keycode) = key_name_to_evdev(key) {
                    keyboard.input::<(), _>(
                        state, keycode.into(),
                        smithay::backend::input::KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        time + 1,
                        |_, _, _| FilterResult::Forward,
                    );
                    keyboard.input::<(), _>(
                        state, keycode.into(),
                        smithay::backend::input::KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        time + 2,
                        |_, _, _| FilterResult::Forward,
                    );
                }
                for &mc in mod_codes.iter().rev() {
                    keyboard.input::<(), _>(
                        state, mc.into(),
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

        ToolCall::ScreenCapture { region: _, scale: _ } => {
            match capture_screen(state) {
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
            }
        }

        _ => serde_json::json!({ "error": "unhandled tool" }),
    }
}

#[cfg(target_os = "linux")]
const NVIM_SOCKET: &str = "/tmp/nvim-mcp.sock";

#[cfg(target_os = "linux")]
fn open_in_editor(state: &mut AgentCompositor, path: &str, line: Option<u32>) {
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
            "-T".to_string(), format!("nvim - {path}"),
            "-e".to_string(), "nvim".to_string(),
            "--listen".to_string(), NVIM_SOCKET.to_string(),
        ];
        if !goto_arg.is_empty() {
            args.push(goto_arg);
        }
        args.push(path.to_string());

        match std::process::Command::new("alacritty")
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
    match ch {
        'a'..='z' => Some((ch as u32 - 'a' as u32 + 30 + OFF, false)),
        'A'..='Z' => Some((ch as u32 - 'A' as u32 + 30 + OFF, true)),
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
