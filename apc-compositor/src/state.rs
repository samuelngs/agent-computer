#[cfg(target_os = "linux")]
use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        allocator::dumb::DumbAllocator,
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, DrmSurface, NodeType},
        input::{Device, DeviceCapability, InputEvent},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            element::{Id, memory::MemoryRenderBuffer},
            pixman::PixmanRenderer,
            utils::on_commit_buffer_handler,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{self, UdevBackend},
    },
    desktop::{
        PopupKeyboardGrab, PopupKind, PopupManager, PopupPointerGrab, Window,
        find_popup_root_surface, get_popup_toplevel_coords, space::Space,
    },
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::XkbConfig,
        pointer::{CursorImageStatus, Focus, GrabStartData, PointerHandle},
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopSignal, Mode, PostAction,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        input::Libinput,
        wayland_server::{
            Client, Display,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer::WlBuffer, wl_seat::WlSeat, wl_surface::WlSurface},
        },
    },
    utils::{DeviceFd, Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            self, CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface,
        },
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        output::OutputManagerState,
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            decoration::{XdgDecorationHandler, XdgDecorationState},
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
    },
};

#[cfg(target_os = "linux")]
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;

#[cfg(target_os = "linux")]
use std::{
    collections::HashSet,
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
use drm::control::{Device as ControlDevice, connector};

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "linux")]
use rustix::fs::OFlags;

#[cfg(target_os = "linux")]
use crate::input::CursorShape;

#[cfg(target_os = "linux")]
use crate::render::{
    RedrawState, ScreenSnapshot, SoftwareDrmCompositor, create_solid_buffer, queue_redraw,
    render_frame, send_frame_callbacks, taskbar_height,
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
pub(crate) struct ApcCompositor {
    pub(crate) loop_signal: LoopSignal,
    pub(crate) start_time: Instant,

    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) xdg_decoration_state: XdgDecorationState,
    pub(crate) shm_state: ShmState,
    pub(crate) dmabuf_state: DmabufState,
    pub(crate) dmabuf_global: DmabufGlobal,
    pub(crate) seat_state: SeatState<Self>,
    pub(crate) data_device_state: DataDeviceState,
    pub(crate) output_manager_state: OutputManagerState,

    pub(crate) space: Space<Window>,
    pub(crate) output: Output,

    pub(crate) seat: Seat<Self>,
    pub(crate) pointer: PointerHandle<Self>,

    pub(crate) session: LibSeatSession,
    pub(crate) renderer: PixmanRenderer,
    pub(crate) drm_compositor: SoftwareDrmCompositor,
    pub(crate) drm_device: DrmDevice,

    pub(crate) cursor_default: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_nwse: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_nesw: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_ns: crate::cursor::LoadedCursor,
    pub(crate) cursor_resize_ew: crate::cursor::LoadedCursor,
    pub(crate) cursor_shape: CursorShape,
    pub(crate) legacy_cursor: Option<crate::cursor::LegacyHardwareCursor>,
    pub(crate) redraw_state: RedrawState,
    pub(crate) last_render_at: Instant,
    pub(crate) last_screen_snapshot: Option<ScreenSnapshot>,
    pub(crate) screen_snapshot_requested: bool,

    pub(crate) taskbar_bg: MemoryRenderBuffer,
    pub(crate) taskbar_buttons: Vec<(String, bool, bool, MemoryRenderBuffer)>,
    pub(crate) taskbar_bg_id: Id,
    pub(crate) taskbar_button_ids: Vec<Id>,
    pub(crate) ssd_titlebar_ids: Vec<(Window, Id)>,
    pub(crate) ssd_titlebar_buffers: Vec<(Window, String, i32, MemoryRenderBuffer)>,

    pub(crate) popup_manager: PopupManager,
    pub(crate) minimized_windows: Vec<(Window, Point<i32, Logical>)>,
    pub(crate) window_order: Vec<Window>,

    pub(crate) scale_factor: i32,
    pub(crate) ssd_windows:
        HashSet<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,

    pub(crate) wayland_display: String,
    pub(crate) browser: crate::browser::BrowserService,
    pub(crate) mcp_tx: calloop::channel::Sender<crate::mcp::McpCommand>,
    pub(crate) editor_pid: Option<u32>,
    pub(crate) mcp_pids: Vec<u32>,
}

#[cfg(target_os = "linux")]
pub(crate) struct CalloopData {
    pub(crate) display: Display<ApcCompositor>,
    pub(crate) state: ApcCompositor,
}

#[cfg(target_os = "linux")]
impl ApcCompositor {
    pub(crate) fn render_software_cursor(&self) -> bool {
        std::env::var_os("APC_HOST_CURSOR").is_none() && self.legacy_cursor.is_none()
    }

    pub(crate) fn sync_legacy_cursor(&mut self) {
        let Some(cursor) = self.legacy_cursor.as_mut() else {
            return;
        };

        let location = self.pointer.current_location();
        if let Err(e) = cursor.move_to(&self.drm_device, location, self.scale_factor) {
            tracing::warn!(%e, "legacy hardware cursor failed, falling back to software cursor");
            self.legacy_cursor = None;
            queue_redraw(self);
        }
    }

    pub(crate) fn finish_pointer_motion(&mut self) {
        self.sync_legacy_cursor();
        if self.render_software_cursor() {
            queue_redraw(self);
        }
    }

    fn popup_target_rect(&self, surface: &PopupSurface) -> Rectangle<i32, Logical> {
        let output_size = self
            .output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1920, 1080).into());
        let s = self.scale_factor;
        let logical_w = output_size.w / s;
        let logical_h = output_size.h / s;

        let kind = PopupKind::Xdg(surface.clone());
        let popup_offset = get_popup_toplevel_coords(&kind);

        let toplevel_loc = find_popup_root_surface(&kind)
            .ok()
            .and_then(|root| {
                self.space.elements().find_map(|w| {
                    let is_root = w
                        .toplevel()
                        .map(|t| t.wl_surface() == &root)
                        .unwrap_or(false);
                    if is_root {
                        self.space.element_location(w)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        let parent_global: Point<i32, Logical> = (
            toplevel_loc.x + popup_offset.x,
            toplevel_loc.y + popup_offset.y,
        )
            .into();

        Rectangle::new(
            (-parent_global.x, -parent_global.y).into(),
            (logical_w, logical_h).into(),
        )
    }

    pub(crate) fn is_ssd(&self, window: &Window) -> bool {
        window
            .toplevel()
            .map(|t| self.ssd_windows.contains(t.wl_surface()))
            .unwrap_or(false)
    }

    fn pick_window_position(
        &self,
        win_w: i32,
        win_h: i32,
        usable_w: i32,
        usable_h: i32,
    ) -> (i32, i32) {
        let margin = 48;
        let max_x = (usable_w - win_w - margin).max(margin);
        let max_y = (usable_h - win_h - margin).max(margin);

        let existing: Vec<Rectangle<i32, Logical>> = self
            .space
            .elements()
            .filter_map(|w| {
                let loc = self.space.element_location(w)?;
                let size = w.toplevel()?.current_state().size?;
                Some(Rectangle::from_loc_and_size(loc, size))
            })
            .collect();

        if existing.is_empty() {
            return ((usable_w - win_w) / 2, (usable_h - win_h) / 2);
        }

        let seed = self.start_time.elapsed().as_micros() as u64;
        let range_x = (max_x - margin + 1).max(1) as u64;
        let range_y = (max_y - margin + 1).max(1) as u64;

        let candidate_rect = |x: i32, y: i32| -> Rectangle<i32, Logical> {
            Rectangle::from_loc_and_size((x, y), (win_w, win_h))
        };

        let overlap = |x: i32, y: i32| -> i64 {
            let r = candidate_rect(x, y);
            existing
                .iter()
                .map(|e| {
                    e.intersection(r)
                        .map(|i| i.size.w as i64 * i.size.h as i64)
                        .unwrap_or(0)
                })
                .sum()
        };

        let mut best = ((usable_w - win_w) / 2, (usable_h - win_h) / 2);
        let mut best_score = overlap(best.0, best.1);

        for i in 0u64..16 {
            let h = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(i.wrapping_mul(1442695040888963407));
            let x = margin + ((h >> 16) % range_x) as i32;
            let y = margin + ((h >> 32) % range_y) as i32;
            let score = overlap(x, y);
            if score < best_score {
                best_score = score;
                best = (x, y);
                if score == 0 {
                    break;
                }
            }
        }

        best
    }
}

#[cfg(target_os = "linux")]
impl CompositorHandler for ApcCompositor {
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
            if let Some(window) = self.space.elements().find(|w| {
                w.toplevel()
                    .map(|t| t.wl_surface() == &root)
                    .unwrap_or(false)
            }) {
                window.on_commit();
            }
        }

        queue_redraw(self);
    }
}

#[cfg(target_os = "linux")]
impl XdgShellHandler for ApcCompositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let s = self.scale_factor;
        let output_size = self
            .output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1920, 1080).into());
        let logical_w = output_size.w as i32 / s;
        let logical_h = output_size.h as i32 / s;
        let taskbar_h = taskbar_height(1);
        let usable_w = logical_w;
        let usable_h = logical_h - taskbar_h;

        let win_w = (usable_w * 2 / 3).min(960);
        let win_h = (usable_h * 2 / 3).min(720);
        let (x, y) = self.pick_window_position(win_w, win_h, usable_w, usable_h);

        surface.with_pending_state(|s| {
            s.size = Some((win_w, win_h).into());
        });
        let focus_surface = surface.wl_surface().clone();
        surface.send_configure();
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window.clone(), (x, y), true);
        self.window_order.push(window);
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(focus_surface), SERIAL_COUNTER.next_serial());
        }
        queue_redraw(self);
        tracing::info!("new toplevel window mapped and focused");
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
            let initial_size = surface.current_state().size.unwrap_or((800, 600).into());
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

    fn new_popup(&mut self, surface: PopupSurface, positioner: PositionerState) {
        let target = self.popup_target_rect(&surface);
        let geometry = positioner.get_unconstrained_geometry(target);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
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
        if let Ok(grab) = self
            .popup_manager
            .grab_popup::<ApcCompositor>(root, kind, &seat, serial)
        {
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
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        let target = self.popup_target_rect(&surface);
        let geometry = positioner.get_unconstrained_geometry(target);
        surface.with_pending_state(|state| {
            state.geometry = geometry;
            state.positioner = positioner;
        });
        let _ = surface.send_repositioned(token);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.ssd_windows.remove(surface.wl_surface());
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
impl XdgDecorationHandler for ApcCompositor {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
        let surface = toplevel.wl_surface().clone();
        if self.ssd_windows.insert(surface.clone()) {
            let win_and_loc: Option<(Window, Point<i32, Logical>)> = self
                .space
                .elements()
                .find(|w| {
                    w.toplevel()
                        .map(|t| t.wl_surface() == &surface)
                        .unwrap_or(false)
                })
                .map(|w| {
                    let loc = self.space.element_location(w).unwrap_or_default();
                    (w.clone(), loc)
                });
            if let Some((window, loc)) = win_and_loc {
                self.space.map_element(
                    window,
                    (loc.x, loc.y + crate::render::SSD_TITLE_BAR_HEIGHT),
                    true,
                );
            }
        }
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(mode);
        });
        toplevel.send_configure();
        let surface = toplevel.wl_surface().clone();
        if mode == DecorationMode::ClientSide {
            if self.ssd_windows.remove(&surface) {
                let win_and_loc: Option<(Window, Point<i32, Logical>)> = self
                    .space
                    .elements()
                    .find(|w| {
                        w.toplevel()
                            .map(|t| t.wl_surface() == &surface)
                            .unwrap_or(false)
                    })
                    .map(|w| {
                        let loc = self.space.element_location(w).unwrap_or_default();
                        (w.clone(), loc)
                    });
                if let Some((window, loc)) = win_and_loc {
                    self.space.map_element(
                        window,
                        (loc.x, loc.y - crate::render::SSD_TITLE_BAR_HEIGHT),
                        true,
                    );
                }
            }
        } else {
            self.ssd_windows.insert(surface);
        }
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        toplevel.send_configure();
        self.ssd_windows.insert(toplevel.wl_surface().clone());
    }
}

#[cfg(target_os = "linux")]
impl ShmHandler for ApcCompositor {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

#[cfg(target_os = "linux")]
impl BufferHandler for ApcCompositor {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

#[cfg(target_os = "linux")]
impl DmabufHandler for ApcCompositor {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        match self.renderer.import_dmabuf(&dmabuf, None) {
            Ok(_) => {
                let _ = notifier.successful::<ApcCompositor>();
                queue_redraw(self);
            }
            Err(e) => {
                tracing::warn!(%e, "failed to import client dmabuf");
                notifier.failed();
            }
        }
    }
}

#[cfg(target_os = "linux")]
impl SeatHandler for ApcCompositor {
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
impl OutputHandler for ApcCompositor {}

#[cfg(target_os = "linux")]
impl SelectionHandler for ApcCompositor {
    type SelectionUserData = ();
}

#[cfg(target_os = "linux")]
impl DataDeviceHandler for ApcCompositor {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

#[cfg(target_os = "linux")]
impl ClientDndGrabHandler for ApcCompositor {}

#[cfg(target_os = "linux")]
impl ServerDndGrabHandler for ApcCompositor {
    fn send(&mut self, _mime: String, _fd: OwnedFd, _seat: Seat<Self>) {}
}

#[cfg(target_os = "linux")]
smithay::delegate_compositor!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_xdg_shell!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_shm!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_dmabuf!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_seat!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_output!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_data_device!(ApcCompositor);
#[cfg(target_os = "linux")]
smithay::delegate_xdg_decoration!(ApcCompositor);

#[cfg(target_os = "linux")]
pub fn run() -> Result<()> {
    let (mut session, session_notifier) =
        LibSeatSession::new().context("failed to create libseat session")?;
    let seat_name = session.seat();
    tracing::info!(seat = %seat_name, "session opened");

    let mut event_loop: EventLoop<CalloopData> = EventLoop::try_new()?;
    let loop_handle = event_loop.handle();

    let udev_backend = UdevBackend::new(&seat_name).context("udev backend")?;
    let gpu_path = udev::primary_gpu(&seat_name)
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

    let res = drm_device.resource_handles().context("DRM resources")?;
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

    let allocator = DumbAllocator::new(device_fd.clone());
    let renderer = PixmanRenderer::new().context("Pixman renderer")?;
    let renderer_formats: HashSet<_> = renderer.dmabuf_formats().into_iter().collect();
    let dmabuf_formats = renderer_formats.clone();
    let primary_node = DrmNode::from_file(&device_fd).context("DRM node from device fd")?;
    let feedback_node = primary_node
        .node_with_type(NodeType::Render)
        .transpose()
        .context("DRM render node from primary node")?
        .unwrap_or(primary_node);
    let feedback_device = feedback_node.dev_id();
    tracing::info!(
        node = %feedback_node,
        major = feedback_node.major(),
        minor = feedback_node.minor(),
        "linux-dmabuf feedback device selected"
    );
    let exporter = device_fd.clone();

    let output = Output::new(
        "Virtual-1".to_string(),
        PhysicalProperties {
            size: Size::from((0, 0)),
            make: "APC".to_string(),
            model: "VirtIO GPU".to_string(),
            subpixel: Subpixel::Unknown,
        },
    );
    let output_mode = OutputMode {
        size: (mode_size.0 as i32, mode_size.1 as i32).into(),
        refresh: mode.vrefresh() as i32 * 1000,
    };
    let scale_factor: i32 = std::env::var("APC_SCALE")
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
        None,
    )
    .context("DRM compositor")?;

    loop_handle
        .insert_source(drm_notifier, |event, _, data| match event {
            DrmEvent::VBlank(_crtc) => {
                if let RedrawState::WaitingForVBlank { redraw_needed, .. } =
                    std::mem::replace(&mut data.state.redraw_state, RedrawState::Idle)
                {
                    if let Err(e) = data.state.drm_compositor.frame_submitted() {
                        tracing::error!("frame_submitted failed: {e}");
                    }
                    send_frame_callbacks(&mut data.state);
                    if redraw_needed {
                        queue_redraw(&mut data.state);
                    }
                }
            }
            DrmEvent::Error(e) => tracing::error!("DRM error: {e}"),
        })
        .map_err(|e| anyhow::anyhow!("drm source: {}", e.error))?;

    loop_handle
        .insert_source(
            Timer::from_duration(Duration::from_millis(16)),
            |_, _, data: &mut CalloopData| {
                let timed_out = match data.state.redraw_state {
                    RedrawState::WaitingForVBlank { submitted_at, .. } => {
                        submitted_at.elapsed() >= Duration::from_millis(33)
                    }
                    _ => false,
                };

                if timed_out {
                    if let RedrawState::WaitingForVBlank { redraw_needed, .. } =
                        std::mem::replace(&mut data.state.redraw_state, RedrawState::Idle)
                    {
                        if let Err(e) = data.state.drm_compositor.frame_submitted() {
                            tracing::warn!("frame_submitted watchdog failed: {e}");
                        }
                        send_frame_callbacks(&mut data.state);
                        if redraw_needed {
                            queue_redraw(&mut data.state);
                        }
                    }
                }

                TimeoutAction::ToDuration(Duration::from_millis(16))
            },
        )
        .map_err(|e| anyhow::anyhow!("redraw watchdog source: {}", e.error))?;

    let mut display = Display::<ApcCompositor>::new()?;
    let dh = display.handle();

    let compositor_state = CompositorState::new::<ApcCompositor>(&dh);
    let xdg_shell_state = XdgShellState::new::<ApcCompositor>(&dh);
    let xdg_decoration_state = XdgDecorationState::new::<ApcCompositor>(&dh);
    let shm_state = ShmState::new::<ApcCompositor>(&dh, vec![]);
    let mut dmabuf_state = DmabufState::new();
    let dmabuf_format_count = dmabuf_formats.len();
    let dmabuf_feedback = DmabufFeedbackBuilder::new(feedback_device, dmabuf_formats)
        .build()
        .context("linux-dmabuf feedback")?;
    let dmabuf_global =
        dmabuf_state.create_global_with_default_feedback::<ApcCompositor>(&dh, &dmabuf_feedback);
    tracing::info!(
        formats = dmabuf_format_count,
        "linux-dmabuf global created with default feedback"
    );
    let mut seat_state = SeatState::<ApcCompositor>::new();
    let data_device_state = DataDeviceState::new::<ApcCompositor>(&dh);
    let output_manager_state = OutputManagerState::new_with_xdg_output::<ApcCompositor>(&dh);

    let _output_global = output.create_global::<ApcCompositor>(&dh);

    let mut seat = seat_state.new_wl_seat(&dh, &seat_name);
    let _keyboard = seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    let pointer = seat.add_pointer();

    let mut libinput_context =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput_context
        .udev_assign_seat(&seat_name)
        .map_err(|_| anyhow::anyhow!("failed to assign libinput seat {seat_name}"))?;
    if !session.is_active() {
        tracing::info!("session inactive at startup; suspending libinput until activation");
        libinput_context.suspend();
    }
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    loop_handle
        .insert_source(libinput_backend, |event, _, data| {
            log_libinput_device_event(&event);
            crate::input::handle_input(&mut data.state, event);
        })
        .map_err(|e| anyhow::anyhow!("libinput source: {}", e.error))?;
    tracing::info!(%seat_name, "libinput backend initialized");

    loop_handle
        .insert_source(session_notifier, move |event, _, data| match event {
            SessionEvent::ActivateSession => {
                tracing::info!("session activated");
                if let Err(e) = libinput_context.resume() {
                    tracing::warn!(?e, "failed to resume libinput");
                }
                let _ = data.state.drm_device.activate(false);
            }
            SessionEvent::PauseSession => {
                tracing::info!("session paused");
                libinput_context.suspend();
                data.state.drm_device.pause();
            }
        })
        .map_err(|e| anyhow::anyhow!("session source: {}", e.error))?;

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
    let legacy_cursor = if std::env::var_os("APC_USE_LEGACY_HARDWARE_CURSOR").is_some() {
        crate::cursor::LegacyHardwareCursor::new(&drm_device, crtc, scale_factor)
    } else {
        None
    };
    let output_w = mode_size.0 as i32;
    let taskbar_bg = create_solid_buffer(
        output_w,
        crate::render::taskbar_height(scale_factor),
        30,
        30,
        30,
        255,
        scale_factor,
    );

    let (_mcp, mcp_tx) = crate::mcp::start(loop_handle.clone())?;

    let state = ApcCompositor {
        loop_signal,
        start_time: Instant::now(),
        compositor_state,
        xdg_shell_state,
        xdg_decoration_state,
        shm_state,
        dmabuf_state,
        dmabuf_global,
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
        legacy_cursor,
        redraw_state: RedrawState::Idle,
        last_render_at: Instant::now() - Duration::from_millis(16),
        last_screen_snapshot: None,
        screen_snapshot_requested: false,
        taskbar_bg,
        taskbar_buttons: Vec::new(),
        taskbar_bg_id: Id::new(),
        taskbar_button_ids: Vec::new(),
        ssd_titlebar_ids: Vec::new(),
        ssd_titlebar_buffers: Vec::new(),
        popup_manager: PopupManager::default(),
        minimized_windows: Vec::new(),
        window_order: Vec::new(),
        scale_factor,
        ssd_windows: HashSet::new(),
        wayland_display: socket_name.clone(),
        browser: crate::browser::BrowserService::new(socket_name.clone()),
        mcp_tx,
        editor_pid: None,
        mcp_pids: Vec::new(),
    };

    let wayland_display = socket_name.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| format!("/run/user/{}", unsafe { libc::getuid() }));
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
        tracing::info!("startup apps launched");
    });

    let mut calloop_data = CalloopData { display, state };

    render_frame(&mut calloop_data.state);

    tracing::info!("compositor initialized, entering event loop");
    event_loop.run(None, &mut calloop_data, |data| {
        data.state.space.refresh();
        if matches!(data.state.redraw_state, RedrawState::Queued)
            && data.state.last_render_at.elapsed() >= Duration::from_millis(16)
        {
            render_frame(&mut data.state);
        }
        data.display.flush_clients().unwrap();
    })?;

    Ok(())
}

#[cfg(target_os = "linux")]
fn log_libinput_device_event(event: &InputEvent<LibinputInputBackend>) {
    match event {
        InputEvent::DeviceAdded { device } => {
            tracing::info!(
                id = %device.id(),
                name = %device.name(),
                keyboard = device.has_capability(DeviceCapability::Keyboard.into()),
                pointer = device.has_capability(DeviceCapability::Pointer.into()),
                touch = device.has_capability(DeviceCapability::Touch.into()),
                tablet_tool = device.has_capability(DeviceCapability::TabletTool.into()),
                tablet_pad = device.has_capability(DeviceCapability::TabletPad.into()),
                "libinput device added"
            );
        }
        InputEvent::DeviceRemoved { device } => {
            tracing::info!(id = %device.id(), name = %device.name(), "libinput device removed");
        }
        _ => {}
    }
}
