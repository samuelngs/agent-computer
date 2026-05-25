#[cfg(target_os = "linux")]
use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, DrmSurface,
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{EGLContext, EGLDisplay},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            element::memory::MemoryRenderBuffer,
            gles::GlesRenderer,
            utils::on_commit_buffer_handler,
            ImportDma,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{self, UdevBackend},
    },
    desktop::{
        PopupManager, PopupKind, PopupKeyboardGrab, PopupPointerGrab,
        space::Space, Window,
    },
    input::{
        keyboard::XkbConfig,
        pointer::{CursorImageStatus, Focus, GrabStartData, PointerHandle},
        Seat, SeatHandler, SeatState,
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        input::Libinput,
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer::WlBuffer, wl_seat::WlSeat, wl_surface::WlSurface},
            Client, Display,
        },
    },
    utils::{DeviceFd, Logical, Point, Serial, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{self, get_parent, is_sync_subsurface, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        output::OutputManagerState,
        selection::{
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
            SelectionHandler,
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
    },
};

#[cfg(target_os = "linux")]
use std::{
    collections::HashSet,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use drm::control::{connector, Device as ControlDevice};

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "linux")]
use rustix::fs::OFlags;

#[cfg(target_os = "linux")]
use crate::input::CursorShape;

#[cfg(target_os = "linux")]
use crate::render::{
    GbmDrmCompositor, RedrawState,
    create_solid_buffer, queue_redraw, render_frame, taskbar_height,
};

#[cfg(target_os = "linux")]
struct DisplayFd(i32);

#[cfg(target_os = "linux")]
impl AsFd for DisplayFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

#[cfg(target_os = "linux")]
impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[cfg(target_os = "linux")]
pub(crate) struct AgentCompositor {
    pub(crate) loop_signal: LoopSignal,
    pub(crate) start_time: Instant,

    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) shm_state: ShmState,
    pub(crate) seat_state: SeatState<Self>,
    pub(crate) data_device_state: DataDeviceState,
    pub(crate) output_manager_state: OutputManagerState,

    pub(crate) space: Space<Window>,
    pub(crate) output: Output,

    pub(crate) seat: Seat<Self>,
    pub(crate) pointer: PointerHandle<Self>,

    pub(crate) session: LibSeatSession,
    pub(crate) renderer: GlesRenderer,
    pub(crate) drm_compositor: GbmDrmCompositor,
    pub(crate) drm_device: DrmDevice,

    pub(crate) cursor_default: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_nwse: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_nesw: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_ns: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_ew: crate::cursor::LoadedCursor,
    pub(crate) cursor_shape: CursorShape,
    pub(crate) redraw_state: RedrawState,

    pub(crate) taskbar_bg: MemoryRenderBuffer,
    pub(crate) taskbar_buttons: Vec<(String, bool, bool, MemoryRenderBuffer)>,

    pub(crate) popup_manager: PopupManager,
    pub(crate) minimized_windows: Vec<(Window, Point<i32, Logical>)>,
    pub(crate) window_order: Vec<Window>,

    pub(crate) scale_factor: i32,

    pub(crate) wayland_display: String,
    pub(crate) mcp_tx: calloop::channel::Sender<crate::mcp::McpCommand>,
}

#[cfg(target_os = "linux")]
pub(crate) struct CalloopData {
    pub(crate) display: Display<AgentCompositor>,
    pub(crate) state: AgentCompositor,
}

#[cfg(target_os = "linux")]
impl CompositorHandler for AgentCompositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popup_manager.commit(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self
                .space
                .elements()
                .find(|w| w.toplevel().map(|t| t.wl_surface() == &root).unwrap_or(false))
            {
                window.on_commit();
            }
        }

        queue_redraw(self);
    }
}

#[cfg(target_os = "linux")]
impl XdgShellHandler for AgentCompositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let s = self.scale_factor;
        let output_size = self.output.current_mode().map(|m| m.size).unwrap_or((1920, 1080).into());
        let logical_w = output_size.w as i32 / s;
        let logical_h = output_size.h as i32 / s;
        let usable: Size<i32, Logical> = (logical_w, logical_h - taskbar_height(1)).into();
        surface.with_pending_state(|s| {
            s.size = Some(usable);
        });
        surface.send_configure();
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window.clone(), (0, 0), false);
        self.window_order.push(window);
        tracing::info!("new toplevel window mapped");
    }

    fn move_request(&mut self, surface: ToplevelSurface, _seat: WlSeat, serial: Serial) {
        let pointer = self.pointer.clone();
        let window = self
            .space
            .elements()
            .find(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == surface.wl_surface())
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(window) = window {
            let initial_loc = self.space.element_location(&window).unwrap_or_default();
            let start_data = GrabStartData {
                focus: None,
                button: 0x110,
                location: pointer.current_location(),
            };
            let grab = crate::grabs::MoveSurfaceGrab {
                window,
                start_data,
                initial_loc,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        _seat: WlSeat,
        serial: Serial,
        edges: smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::ResizeEdge,
    ) {
        let pointer = self.pointer.clone();
        let window = self
            .space
            .elements()
            .find(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == surface.wl_surface())
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(window) = window {
            let initial_loc = self.space.element_location(&window).unwrap_or_default();
            let initial_size = surface
                .current_state()
                .size
                .unwrap_or((800, 600).into());
            let start_data = GrabStartData {
                focus: None,
                button: 0x110,
                location: pointer.current_location(),
            };
            let grab = crate::grabs::ResizeSurfaceGrab {
                window,
                start_data,
                edges: edges as u32,
                initial_size,
                initial_loc,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        let s = self.scale_factor;
        let output_size = self
            .output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1920, 1080).into());
        let logical_w = output_size.w as i32 / s;
        let logical_h = output_size.h as i32 / s;
        let usable: Size<i32, Logical> = (logical_w, logical_h - taskbar_height(1)).into();
        surface.with_pending_state(|s| {
            s.size = Some(usable);
            s.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
        let window = self
            .space
            .elements()
            .find(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == surface.wl_surface())
                    .unwrap_or(false)
            })
            .cloned();
        if let Some(window) = window {
            self.space.map_element(window, (0, 0), true);
        }
        queue_redraw(self);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|s| {
            s.size = None;
            s.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
        });
        surface.send_configure();
        queue_redraw(self);
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let window = self
            .space
            .elements()
            .find(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == surface.wl_surface())
                    .unwrap_or(false)
            })
            .cloned();
        if let Some(window) = window {
            crate::input::minimize_window(self, &window);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = surface.send_configure();
        let _ = self.popup_manager.track_popup(PopupKind::Xdg(surface));
    }

    fn grab(&mut self, surface: PopupSurface, _seat: WlSeat, serial: Serial) {
        let seat = self.seat.clone();
        let kind = PopupKind::Xdg(surface);
        let mut root = kind.wl_surface().clone();
        while let Some(parent) = compositor::get_parent(&root) {
            root = parent;
        }
        if let Ok(grab) = self.popup_manager.grab_popup::<AgentCompositor>(root, kind, &seat, serial) {
            let keyboard = seat.get_keyboard().unwrap();
            let pointer = seat.get_pointer().unwrap();
            let kb_grab = PopupKeyboardGrab::new(&grab);
            let ptr_grab = PopupPointerGrab::new(&grab);
            keyboard.set_grab(self, kb_grab, serial);
            pointer.set_grab(self, ptr_grab, serial, Focus::Keep);
        }
    }

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.minimized_windows.retain(|(w, _)| {
            w.toplevel()
                .map(|t| t.wl_surface() != surface.wl_surface())
                .unwrap_or(true)
        });
        self.window_order.retain(|w| {
            w.toplevel()
                .map(|t| t.wl_surface() != surface.wl_surface())
                .unwrap_or(true)
        });
    }
}

#[cfg(target_os = "linux")]
impl ShmHandler for AgentCompositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

#[cfg(target_os = "linux")]
impl BufferHandler for AgentCompositor {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

#[cfg(target_os = "linux")]
impl SeatHandler for AgentCompositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
}

#[cfg(target_os = "linux")]
impl OutputHandler for AgentCompositor {}

#[cfg(target_os = "linux")]
impl SelectionHandler for AgentCompositor {
    type SelectionUserData = ();
}

#[cfg(target_os = "linux")]
impl DataDeviceHandler for AgentCompositor {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

#[cfg(target_os = "linux")]
impl ClientDndGrabHandler for AgentCompositor {}

#[cfg(target_os = "linux")]
impl ServerDndGrabHandler for AgentCompositor {
    fn send(&mut self, _mime: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

#[cfg(target_os = "linux")]
smithay::delegate_compositor!(AgentCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_xdg_shell!(AgentCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_shm!(AgentCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_seat!(AgentCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_output!(AgentCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_data_device!(AgentCompositor);

#[cfg(target_os = "linux")]
pub fn run() -> Result<()> {
    let (mut session, session_notifier) =
        LibSeatSession::new().context("failed to create libseat session")?;
    tracing::info!(seat = %session.seat(), "session opened");

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    loop_handle
        .insert_source(session_notifier, |event, _, data| match event {
            SessionEvent::ActivateSession => {
                tracing::info!("session activated");
                let _ = data.state.drm_device.activate(false);
            }
            SessionEvent::PauseSession => {
                tracing::info!("session paused");
                data.state.drm_device.pause();
            }
        })
        .map_err(|e| anyhow::anyhow!("session source: {}", e.error))?;

    let udev_backend = UdevBackend::new(&session.seat()).context("udev backend")?;
    let gpu_path = udev::primary_gpu(&session.seat())
        .ok()
        .flatten()
        .or_else(|| {
            udev_backend
                .device_list()
                .next()
                .map(|(_, path)| path.to_path_buf())
        })
        .context("no GPU found")?;
    tracing::info!(?gpu_path, "using GPU");

    let fd = session
        .open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .context("failed to open DRM device")?;
    let device_fd = DrmDeviceFd::new(DeviceFd::from(fd));

    let (mut drm_device, drm_notifier) =
        DrmDevice::new(device_fd.clone(), true).context("DRM device init")?;

    let res = drm_device
        .resource_handles()
        .context("DRM resources")?;
    let (connector_handle, mode) = res
        .connectors()
        .iter()
        .find_map(|c| {
            let info = drm_device.get_connector(*c, true).ok()?;
            if info.state() == connector::State::Connected {
                let mode = info.modes().first().copied()?;
                Some((*c, mode))
            } else {
                None
            }
        })
        .context("no connected display")?;

    let crtc = res.crtcs().first().copied().context("no CRTC available")?;

    let mode_size = mode.size();
    tracing::info!(
        connector = ?connector_handle,
        crtc = ?crtc,
        mode = %format!("{}x{}@{}Hz", mode_size.0, mode_size.1, mode.vrefresh()),
        "DRM output configured"
    );

    let surface: DrmSurface = drm_device.create_surface(crtc, mode, &[connector_handle])?;

    let gbm_device = GbmDevice::new(device_fd.clone()).context("GBM device")?;
    let allocator = GbmAllocator::new(
        gbm_device.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    let egl_display =
        unsafe { EGLDisplay::new(gbm_device.clone()) }.context("EGL display")?;
    let egl_context = EGLContext::new(&egl_display).context("EGL context")?;
    let renderer = unsafe { GlesRenderer::new(egl_context) }.context("GLES renderer")?;

    let renderer_formats: HashSet<_> = renderer.dmabuf_formats().into_iter().collect();

    let drm_node = DrmNode::from_file(device_fd.clone()).ok();
    let exporter = GbmFramebufferExporter::new(gbm_device.clone(), drm_node);

    let output = Output::new(
        "Virtual-1".to_string(),
        PhysicalProperties {
            size: Size::from((0, 0)),
            make: "AgentOS".to_string(),
            model: "VirtIO GPU".to_string(),
            subpixel: Subpixel::Unknown,
        },
    );
    let output_mode = OutputMode {
        size: (mode_size.0 as i32, mode_size.1 as i32).into(),
        refresh: mode.vrefresh() as i32 * 1000,
    };
    let scale_factor: i32 = std::env::var("AGENTOS_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);
    tracing::info!(scale_factor, "output scale factor");

    output.change_current_state(
        Some(output_mode),
        Some(Transform::Normal),
        Some(smithay::output::Scale::Integer(scale_factor)),
        Some((0, 0).into()),
    );
    output.set_preferred(output_mode);

    let color_formats = [DrmFourcc::Argb8888, DrmFourcc::Xrgb8888];
    let drm_compositor = smithay::backend::drm::compositor::DrmCompositor::new(
        &output,
        surface,
        None,
        allocator,
        exporter,
        color_formats,
        renderer_formats,
        drm_device.cursor_size(),
        Some(gbm_device.clone()),
    )
    .context("DRM compositor")?;

    loop_handle
        .insert_source(drm_notifier, |event, _, data| match event {
            DrmEvent::VBlank(_crtc) => {
                if let Err(e) = data.state.drm_compositor.frame_submitted() {
                    tracing::error!("frame_submitted failed: {e}");
                }
                let redraw_needed = match data.state.redraw_state {
                    RedrawState::WaitingForVBlank { redraw_needed } => redraw_needed,
                    _ => false,
                };
                data.state.redraw_state = RedrawState::Idle;
                if redraw_needed {
                    queue_redraw(&mut data.state);
                }
                let time = data.state.start_time.elapsed();
                let output = &data.state.output;
                data.state.space.elements().for_each(|window| {
                    window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                });
            }
            DrmEvent::Error(e) => tracing::error!("DRM error: {e}"),
        })
        .map_err(|e| anyhow::anyhow!("drm source: {}", e.error))?;

    let mut libinput_context =
        Libinput::new_from_path(LibinputSessionInterface::from(session.clone()));
    for entry in std::fs::read_dir("/dev/input").into_iter().flatten() {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.file_name().map(|n| n.to_string_lossy().starts_with("event")).unwrap_or(false) {
                if let Some(path_str) = path.to_str() {
                    let _ = libinput_context.path_add_device(path_str);
                    tracing::info!("added input device: {}", path_str);
                }
            }
        }
    }
    let libinput_backend = LibinputInputBackend::new(libinput_context);

    loop_handle
        .insert_source(libinput_backend, |event, _, data| {
            crate::input::handle_input(&mut data.state, event);
        })
        .map_err(|e| anyhow::anyhow!("libinput source: {}", e.error))?;

    let mut display = Display::<AgentCompositor>::new()?;
    let dh = display.handle();

    let compositor_state = CompositorState::new::<AgentCompositor>(&dh);
    let xdg_shell_state = XdgShellState::new::<AgentCompositor>(&dh);
    let shm_state = ShmState::new::<AgentCompositor>(&dh, vec![]);
    let mut seat_state = SeatState::<AgentCompositor>::new();
    let data_device_state = DataDeviceState::new::<AgentCompositor>(&dh);
    let output_manager_state =
        OutputManagerState::new_with_xdg_output::<AgentCompositor>(&dh);

    let _output_global = output.create_global::<AgentCompositor>(&dh);

    let mut seat = seat_state.new_wl_seat(&dh, "seat-0");
    let _keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    let pointer = seat.add_pointer();

    let socket = ListeningSocketSource::new_auto().context("wayland socket")?;
    let socket_name = socket
        .socket_name()
        .to_str()
        .unwrap_or("wayland-0")
        .to_string();
    tracing::info!(%socket_name, "wayland socket bound");

    loop_handle
        .insert_source(socket, move |client_stream, _, data| {
            data.display
                .handle()
                .insert_client(client_stream, Arc::new(ClientState::default()))
                .ok();
        })
        .map_err(|e| anyhow::anyhow!("socket source: {}", e.error))?;

    let display_fd = display.backend().poll_fd().as_raw_fd();
    loop_handle
        .insert_source(
            Generic::new(DisplayFd(display_fd), Interest::READ, Mode::Level),
            |_, _, data: &mut CalloopData| {
                data.display.dispatch_clients(&mut data.state).unwrap();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("display source: {}", e.error))?;

    let loop_signal = event_loop.get_signal();

    let mut space = Space::default();
    space.map_output(&output, (0, 0));

    let cursor_theme = crate::cursor::CursorTheme::load(scale_factor);
    let cursor_default = cursor_theme.load_cursor("left_ptr");
    let cursor_resize_nwse = cursor_theme.load_cursor("top_left_corner");
    let cursor_resize_nesw = cursor_theme.load_cursor("top_right_corner");
    let cursor_resize_ns = cursor_theme.load_cursor("sb_v_double_arrow");
    let cursor_resize_ew = cursor_theme.load_cursor("sb_h_double_arrow");
    let output_w = mode_size.0 as i32;
    let taskbar_bg = create_solid_buffer(output_w, crate::render::taskbar_height(scale_factor), 30, 30, 30, 255, scale_factor);

    let (_mcp, mcp_tx) = crate::mcp::start(loop_handle.clone())?;

    let state = AgentCompositor {
        loop_signal,
        start_time: Instant::now(),
        compositor_state,
        xdg_shell_state,
        shm_state,
        seat_state,
        data_device_state,
        output_manager_state,
        space,
        output,
        seat,
        pointer,
        session,
        renderer,
        drm_compositor,
        drm_device,
        cursor_default,
        cursor_resize_nwse,
        cursor_resize_nesw,
        cursor_resize_ns,
        cursor_resize_ew,
        cursor_shape: CursorShape::Default,
        redraw_state: RedrawState::Idle,
        taskbar_bg,
        taskbar_buttons: Vec::new(),
        popup_manager: PopupManager::default(),
        minimized_windows: Vec::new(),
        window_order: Vec::new(),
        scale_factor,
        wayland_display: socket_name.clone(),
        mcp_tx,
    };

    let wayland_display = socket_name.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
            format!("/run/user/{}", unsafe { libc::getuid() })
        });
        let env_vars: Vec<(&str, &str)> = vec![
            ("WAYLAND_DISPLAY", wayland_display.as_str()),
            ("XDG_RUNTIME_DIR", xdg_runtime_dir.as_str()),
            ("TERM", "xterm-256color"),
        ];
        for cmd in &["foot"] {
            tracing::info!(cmd, "attempting to launch terminal");
            let result = std::process::Command::new(cmd)
                .envs(env_vars.iter().cloned())
                .spawn();
            match result {
                Ok(_) => {
                    tracing::info!(cmd, "terminal launched");
                    break;
                }
                Err(e) => tracing::warn!(cmd, %e, "terminal not available"),
            }
        }
        tracing::info!("attempting to launch chromium");
        match std::process::Command::new("chromium")
            .envs(env_vars.iter().cloned())
            .spawn()
        {
            Ok(_) => tracing::info!("chromium launched"),
            Err(e) => tracing::warn!(%e, "chromium not available"),
        }
        tracing::error!("no terminal emulator found");
    });

    let mut calloop_data = CalloopData { display, state };

    render_frame(&mut calloop_data.state);

    tracing::info!("compositor initialized, entering event loop");
    event_loop.run(None, &mut calloop_data, |data| {
        data.state.space.refresh();
        if matches!(data.state.redraw_state, RedrawState::Queued) {
            render_frame(&mut data.state);
        }
        data.display.flush_clients().unwrap();
    })?;

    Ok(())
}
