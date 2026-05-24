#[cfg(target_os = "linux")]
use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
use smithay::{
    backend::{
        allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        drm::{
            compositor::{DrmCompositor, FrameFlags, PrimaryPlaneElement},
            exporter::gbm::GbmFramebufferExporter,
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, DrmSurface,
        },
        egl::{EGLContext, EGLDisplay},
        input::{
            AbsolutePositionEvent, Event, InputEvent, KeyboardKeyEvent, PointerButtonEvent,
            PointerMotionEvent as PointerMotionEventTrait,
        },
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            damage::OutputDamageTracker,
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::{GlesRenderer, GlesRenderbuffer},
            utils::on_commit_buffer_handler,
            Bind, ExportMem, Frame, ImportAll, ImportDma, ImportMem, Offscreen, Renderer,
        },
        session::{libseat::LibSeatSession, Event as SessionEvent, Session},
        udev::{self, UdevBackend},
    },
    desktop::{space::Space, space::space_render_elements, space::SpaceRenderElements, Window, WindowSurfaceType},
    input::{
        keyboard::{FilterResult, XkbConfig},
        pointer::{
            AxisFrame, ButtonEvent, CursorImageStatus, Focus, GrabStartData, MotionEvent,
            PointerGrab, PointerHandle, PointerInnerHandle, RelativeMotionEvent,
        },
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
    utils::{Buffer as BufferCoord, DeviceFd, Logical, Physical, Point, Rectangle, Scale, Serial, Size, Transform, SERIAL_COUNTER},
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
            XdgToplevelSurfaceData,
        },
        shm::{ShmHandler, ShmState},
        socket::ListeningSocketSource,
    },
};


#[cfg(target_os = "linux")]
use agentos_protocol::{ToolCall, WindowInfo};

#[cfg(target_os = "linux")]
use std::{collections::HashSet, os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd}, sync::Arc, time::{Duration, Instant}};

#[cfg(target_os = "linux")]
use drm::control::{connector, Device as ControlDevice};

#[cfg(target_os = "linux")]
use drm_fourcc::DrmFourcc;

#[cfg(target_os = "linux")]
use rustix::fs::OFlags;


#[cfg(target_os = "linux")]
struct DisplayFd(i32);

#[cfg(target_os = "linux")]
impl AsFd for DisplayFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.0) }
    }
}


#[cfg(target_os = "linux")]
smithay::backend::renderer::element::render_elements! {
    OutputRenderElements<R, E> where R: ImportAll + ImportMem;
    Space=SpaceRenderElements<R, E>,
    Cursor=MemoryRenderBufferRenderElement<R>,
}


#[cfg(target_os = "linux")]
type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmFramebufferExporter<DrmDeviceFd>,
    (),
    DrmDeviceFd,
>;


#[cfg(target_os = "linux")]
const TASKBAR_HEIGHT: i32 = 36;
#[cfg(target_os = "linux")]
const TASKBAR_BTN_WIDTH: i32 = 140;
#[cfg(target_os = "linux")]
const TASKBAR_BTN_HEIGHT: i32 = 28;
#[cfg(target_os = "linux")]
const TASKBAR_BTN_GAP: i32 = 4;
#[cfg(target_os = "linux")]
const TASKBAR_BTN_MARGIN: i32 = 4;
#[cfg(target_os = "linux")]
const RESIZE_EDGE_THRESHOLD: f64 = 8.0;


#[cfg(target_os = "linux")]
#[derive(Debug, Default)]
enum RedrawState {
    #[default]
    Idle,
    Queued,
    WaitingForVBlank {
        redraw_needed: bool,
    },
}

#[cfg(target_os = "linux")]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CursorShape {
    #[default]
    Default,
    ResizeNWSE,
    ResizeNESW,
    ResizeNS,
    ResizeEW,
}


#[cfg(target_os = "linux")]
pub struct AgentCompositor {
    pub loop_signal: LoopSignal,
    pub start_time: Instant,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub seat_state: SeatState<Self>,
    pub data_device_state: DataDeviceState,
    pub output_manager_state: OutputManagerState,

    pub space: Space<Window>,
    pub output: Output,

    pub seat: Seat<Self>,
    pub pointer: PointerHandle<Self>,

    pub session: LibSeatSession,
    pub renderer: GlesRenderer,
    pub drm_compositor: GbmDrmCompositor,
    pub drm_device: DrmDevice,

    pub cursor_default: MemoryRenderBuffer,
    pub cursor_resize_nwse: MemoryRenderBuffer,
    pub cursor_resize_nesw: MemoryRenderBuffer,
    pub cursor_resize_ns: MemoryRenderBuffer,
    pub cursor_resize_ew: MemoryRenderBuffer,
    pub cursor_shape: CursorShape,
    pub redraw_state: RedrawState,

    pub taskbar_bg: MemoryRenderBuffer,
    pub taskbar_buttons: Vec<(String, bool, bool, MemoryRenderBuffer)>,

    pub minimized_windows: Vec<(Window, Point<i32, Logical>)>,

    pub wayland_display: String,
    pub mcp_tx: calloop::channel::Sender<crate::mcp::McpCommand>,
}

#[cfg(target_os = "linux")]
pub struct CalloopData {
    pub display: Display<AgentCompositor>,
    pub state: AgentCompositor,
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
impl CompositorHandler for AgentCompositor {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);

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
        let output_size = self.output.current_mode().map(|m| m.size).unwrap_or((1920, 1080).into());
        let usable: Size<i32, Logical> = (output_size.w as i32, output_size.h as i32 - TASKBAR_HEIGHT).into();
        surface.with_pending_state(|s| {
            s.size = Some(usable);
        });
        surface.send_configure();
        let window = Window::new_wayland_window(surface);
        self.space.map_element(window, (0, 0), false);
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
            let grab = MoveSurfaceGrab {
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
            let grab = ResizeSurfaceGrab {
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
        let output_size = self
            .output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1920, 1080).into());
        let usable: Size<i32, Logical> = (output_size.w as i32, output_size.h as i32 - TASKBAR_HEIGHT).into();
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
            minimize_window(self, &window);
        }
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}
    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}
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
struct MoveSurfaceGrab {
    window: Window,
    start_data: GrabStartData<AgentCompositor>,
    initial_loc: Point<i32, Logical>,
}

#[cfg(target_os = "linux")]
impl PointerGrab<AgentCompositor> for MoveSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
        let delta = event.location - self.start_data.location;
        let new_loc = self.initial_loc.to_f64() + delta;
        data.space
            .map_element(self.window.clone(), new_loc.to_i32_round(), true);
        queue_redraw(data);
    }

    fn relative_motion(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
    }

    fn start_data(&self) -> &GrabStartData<AgentCompositor> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut AgentCompositor) {}
}

#[cfg(target_os = "linux")]
struct ResizeSurfaceGrab {
    window: Window,
    start_data: GrabStartData<AgentCompositor>,
    edges: u32,
    initial_size: Size<i32, Logical>,
    initial_loc: Point<i32, Logical>,
}

#[cfg(target_os = "linux")]
impl PointerGrab<AgentCompositor> for ResizeSurfaceGrab {
    fn motion(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
        let delta = event.location - self.start_data.location;

        let mut new_w = self.initial_size.w;
        let mut new_h = self.initial_size.h;
        let mut loc = self.initial_loc;

        // edges: top=1, bottom=2, left=4, right=8
        if self.edges & 8 != 0 {
            new_w = (self.initial_size.w as f64 + delta.x) as i32;
        }
        if self.edges & 4 != 0 {
            new_w = (self.initial_size.w as f64 - delta.x) as i32;
            loc.x = (self.initial_loc.x as f64 + delta.x) as i32;
        }
        if self.edges & 2 != 0 {
            new_h = (self.initial_size.h as f64 + delta.y) as i32;
        }
        if self.edges & 1 != 0 {
            new_h = (self.initial_size.h as f64 - delta.y) as i32;
            loc.y = (self.initial_loc.y as f64 + delta.y) as i32;
        }

        new_w = new_w.max(100);
        new_h = new_h.max(100);

        if let Some(toplevel) = self.window.toplevel() {
            toplevel.with_pending_state(|s| {
                s.size = Some((new_w, new_h).into());
            });
            toplevel.send_configure();
        }
        data.space.map_element(self.window.clone(), loc, true);
    }

    fn relative_motion(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &RelativeMotionEvent,
    ) {
        handle.relative_motion(data, focus, event);
    }

    fn button(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if handle.current_pressed().is_empty() {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn axis(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        details: AxisFrame,
    ) {
        handle.axis(data, details);
    }

    fn frame(
        &mut self,
        data: &mut AgentCompositor,
        handle: &mut PointerInnerHandle<'_, AgentCompositor>,
    ) {
        handle.frame(data);
    }

    fn gesture_swipe_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &mut self,
        _data: &mut AgentCompositor,
        _handle: &mut PointerInnerHandle<'_, AgentCompositor>,
        _event: &smithay::input::pointer::GestureHoldEndEvent,
    ) {
    }

    fn start_data(&self) -> &GrabStartData<AgentCompositor> {
        &self.start_data
    }

    fn unset(&mut self, _data: &mut AgentCompositor) {}
}


#[cfg(target_os = "linux")]
fn create_cursor_buffer() -> MemoryRenderBuffer {
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
                1 => { data[i] = 0; data[i + 1] = 0; data[i + 2] = 0; data[i + 3] = 255; }
                2 => { data[i] = 255; data[i + 1] = 255; data[i + 2] = 255; data[i + 3] = 255; }
                _ => {}
            }
        }
    }
    MemoryRenderBuffer::from_slice(
        &data,
        DrmFourcc::Abgr8888,
        (16, 16),
        1,
        Transform::Normal,
        None,
    )
}

#[cfg(target_os = "linux")]
fn create_cursor_from_bitmap(bitmap: &[[u8; 16]; 16]) -> MemoryRenderBuffer {
    let data = crate::bitmap::bitmap_to_cursor_data(bitmap);
    MemoryRenderBuffer::from_slice(
        &data,
        DrmFourcc::Abgr8888,
        (16, 16),
        1,
        Transform::Normal,
        None,
    )
}

#[cfg(target_os = "linux")]
fn create_solid_buffer(w: i32, h: i32, r: u8, g: u8, b: u8, a: u8) -> MemoryRenderBuffer {
    let data = vec![[r, g, b, a]; (w * h) as usize].into_iter().flatten().collect::<Vec<u8>>();
    MemoryRenderBuffer::from_slice(
        &data,
        DrmFourcc::Abgr8888,
        (w, h),
        1,
        Transform::Normal,
        None,
    )
}


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
    output.change_current_state(
        Some(output_mode),
        Some(Transform::Normal),
        None,
        Some((0, 0).into()),
    );
    output.set_preferred(output_mode);

    let color_formats = [DrmFourcc::Argb8888, DrmFourcc::Xrgb8888];
    let drm_compositor = DrmCompositor::new(
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
            handle_input(&mut data.state, event);
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

    let cursor_default = create_cursor_buffer();
    let cursor_resize_nwse = create_cursor_from_bitmap(&crate::bitmap::resize_nwse_bitmap());
    let cursor_resize_nesw = create_cursor_from_bitmap(&crate::bitmap::resize_nesw_bitmap());
    let cursor_resize_ns = create_cursor_from_bitmap(&crate::bitmap::resize_ns_bitmap());
    let cursor_resize_ew = create_cursor_from_bitmap(&crate::bitmap::resize_ew_bitmap());
    let output_w = mode_size.0 as i32;
    let taskbar_bg = create_solid_buffer(output_w, TASKBAR_HEIGHT, 30, 30, 30, 255);

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
        minimized_windows: Vec::new(),
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

#[cfg(target_os = "linux")]
fn minimize_window(state: &mut AgentCompositor, window: &Window) {
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
fn unminimize_window(state: &mut AgentCompositor, idx: usize) {
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
fn queue_redraw(state: &mut AgentCompositor) {
    match &state.redraw_state {
        RedrawState::Idle => {
            state.redraw_state = RedrawState::Queued;
        }
        RedrawState::Queued => {}
        RedrawState::WaitingForVBlank { .. } => {
            state.redraw_state = RedrawState::WaitingForVBlank {
                redraw_needed: true,
            };
        }
    }
}

#[cfg(target_os = "linux")]
fn get_window_title(window: &Window) -> String {
    window.toplevel().and_then(|t| {
        compositor::with_states(t.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| {
                    let attrs = d.lock().ok()?;
                    attrs.title.clone().or_else(|| attrs.app_id.clone())
                })
        })
    }).unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn edges_to_cursor_shape(edges: u32) -> CursorShape {
    let top = edges & 1 != 0;
    let bottom = edges & 2 != 0;
    let left = edges & 4 != 0;
    let right = edges & 8 != 0;
    match (top, bottom, left, right) {
        (true, false, true, false) => CursorShape::ResizeNWSE,   // top-left
        (false, true, false, true) => CursorShape::ResizeNWSE,   // bottom-right
        (true, false, false, true) => CursorShape::ResizeNESW,   // top-right
        (false, true, true, false) => CursorShape::ResizeNESW,   // bottom-left
        (true, false, false, false) | (false, true, false, false) => CursorShape::ResizeNS,
        (false, false, true, false) | (false, false, false, true) => CursorShape::ResizeEW,
        _ => CursorShape::Default,
    }
}

#[cfg(target_os = "linux")]
fn render_frame(state: &mut AgentCompositor) {
    let space_elements: Vec<SpaceRenderElements<GlesRenderer, _>> = space_render_elements(
        &mut state.renderer,
        [&state.space],
        &state.output,
        1.0,
    )
    .unwrap_or_default();

    let pointer_loc = state.pointer.current_location();
    // Element order is front-to-back: first = topmost
    let mut elements: Vec<OutputRenderElements<GlesRenderer, _>> = Vec::new();

    let cursor_buf = match state.cursor_shape {
        CursorShape::Default => &state.cursor_default,
        CursorShape::ResizeNWSE => &state.cursor_resize_nwse,
        CursorShape::ResizeNESW => &state.cursor_resize_nesw,
        CursorShape::ResizeNS => &state.cursor_resize_ns,
        CursorShape::ResizeEW => &state.cursor_resize_ew,
    };
    if let Ok(cursor_elem) = MemoryRenderBufferRenderElement::from_buffer(
        &mut state.renderer,
        (pointer_loc.x, pointer_loc.y),
        cursor_buf,
        None,
        None,
        None,
        Kind::Cursor,
    ) {
        elements.push(OutputRenderElements::Cursor(cursor_elem));
    }

    let output_h = state.output.current_mode().map(|m| m.size.h).unwrap_or(1080);
    let taskbar_y = (output_h - TASKBAR_HEIGHT) as f64;

    let focused_surface = state.seat.get_keyboard().and_then(|kb| kb.current_focus());
    let mut new_buttons: Vec<(String, bool, bool, MemoryRenderBuffer)> = Vec::new();
    for window in state.space.elements() {
        let title = get_window_title(window);
        let is_focused = window
            .toplevel()
            .map(|t| focused_surface.as_ref() == Some(t.wl_surface()))
            .unwrap_or(false);
        let label = if title.is_empty() { "Window".to_string() } else { title };
        let (r, g, b) = if is_focused { (80, 80, 120) } else { (50, 50, 50) };
        let btn_buf = create_solid_buffer(TASKBAR_BTN_WIDTH, TASKBAR_BTN_HEIGHT, r, g, b, 255);
        new_buttons.push((label, is_focused, false, btn_buf));
    }
    for (window, _) in &state.minimized_windows {
        let title = get_window_title(window);
        let label = if title.is_empty() { "Window".to_string() } else { title };
        let btn_buf = create_solid_buffer(TASKBAR_BTN_WIDTH, TASKBAR_BTN_HEIGHT, 35, 35, 35, 255);
        new_buttons.push((label, false, true, btn_buf));
    }
    state.taskbar_buttons = new_buttons;

    for (i, (_, _, _, btn_buf)) in state.taskbar_buttons.iter().enumerate() {
        let x = (TASKBAR_BTN_MARGIN + i as i32 * (TASKBAR_BTN_WIDTH + TASKBAR_BTN_GAP)) as f64;
        let y = taskbar_y + ((TASKBAR_HEIGHT - TASKBAR_BTN_HEIGHT) / 2) as f64;
        if let Ok(btn) = MemoryRenderBufferRenderElement::from_buffer(
            &mut state.renderer, (x, y), btn_buf, None, None, None, Kind::Unspecified,
        ) {
            elements.push(OutputRenderElements::Cursor(btn));
        }
    }

    if let Ok(bg) = MemoryRenderBufferRenderElement::from_buffer(
        &mut state.renderer,
        (0.0, taskbar_y),
        &state.taskbar_bg,
        None,
        None,
        None,
        Kind::Unspecified,
    ) {
        elements.push(OutputRenderElements::Cursor(bg));
    }

    elements.extend(space_elements.into_iter().map(OutputRenderElements::Space));

    // Toggle clear color epsilon to force full damage every frame.
    // Smithay's damage tracker damages the entire output when clear_color changes,
    // bypassing buffer-age-based partial damage that breaks on virtio-gpu.
    static FRAME_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let frame = FRAME_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let epsilon = if frame % 2 == 0 { 0.0f32 } else { 1.0 / 255.0 };
    let clear_color = [0.1, 0.1, 0.3 + epsilon, 1.0];

    match state
        .drm_compositor
        .render_frame(&mut state.renderer, &elements, clear_color, FrameFlags::empty())
    {
        Ok(result) => {
            if result.needs_sync() {
                if let PrimaryPlaneElement::Swapchain(ref element) = result.primary_element {
                    let _ = element.sync.wait();
                }
            }
            if !result.is_empty {
                match state.drm_compositor.queue_frame(()) {
                    Ok(()) => {
                        state.redraw_state = RedrawState::WaitingForVBlank {
                            redraw_needed: false,
                        };
                    }
                    Err(e) => {
                        tracing::error!("queue_frame failed: {e}");
                        state.redraw_state = RedrawState::Idle;
                    }
                }
            } else {
                state.redraw_state = RedrawState::Idle;
                let time = state.start_time.elapsed();
                let output = &state.output;
                state.space.elements().for_each(|window| {
                    window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                });
            }
        }
        Err(e) => {
            tracing::error!("render failed: {e}");
            state.redraw_state = RedrawState::Idle;
        }
    }
}

#[cfg(target_os = "linux")]
pub fn handle_mcp_tool(
    state: &mut AgentCompositor,
    _display: &mut Display<AgentCompositor>,
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

        ToolCall::WindowResize {
            id,
            width,
            height,
        } => {
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
            // Press
            pointer.button(
                state,
                &ButtonEvent {
                    serial,
                    time,
                    button: btn_code,
                    state: smithay::backend::input::ButtonState::Pressed,
                },
            );
            // Release
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

        ToolCall::KeyboardKey {
            ref key,
            ref modifiers,
        } => {
            if let Some(keyboard) = state.seat.get_keyboard() {
                let time = state.start_time.elapsed().as_millis() as u32;
                // Press modifiers
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

        // These should never arrive here (handled in mcp thread)
        ToolCall::ShellExec { .. } | ToolCall::FileRead { .. } | ToolCall::FileWrite { .. } => {
            serde_json::json!({ "error": "routed to wrong handler" })
        }
    }
}

#[cfg(target_os = "linux")]
fn char_to_evdev_keycode(ch: char) -> Option<(u32, bool)> {
    // Returns (xkb_keycode, needs_shift). XKB keycodes = evdev + 8.
    // smithay keyboard.input expects XKB keycodes (libinput backend adds +8).
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

#[cfg(target_os = "linux")]
fn capture_screen(state: &mut AgentCompositor) -> Result<(u32, u32, String)> {
    use base64::Engine;

    let output_mode = state.output.current_mode().unwrap_or(OutputMode {
        size: (1920, 1080).into(),
        refresh: 60000,
    });
    let w = output_mode.size.w;
    let h = output_mode.size.h;
    let buf_size: Size<i32, BufferCoord> = (w, h).into();

    let mut rbo: GlesRenderbuffer = state
        .renderer
        .create_buffer(drm_fourcc::DrmFourcc::Abgr8888, buf_size)
        .map_err(|e| anyhow::anyhow!("create offscreen buffer: {e}"))?;

    let phys_size: Size<i32, Physical> = (w, h).into();

    let space_elements: Vec<SpaceRenderElements<GlesRenderer, _>> = space_render_elements(
        &mut state.renderer,
        [&state.space],
        &state.output,
        1.0,
    )
    .unwrap_or_default();

    let pointer_loc = state.pointer.current_location();
    let mut elements: Vec<OutputRenderElements<GlesRenderer, _>> = Vec::new();

    if let Ok(cursor_elem) = MemoryRenderBufferRenderElement::from_buffer(
        &mut state.renderer,
        (pointer_loc.x, pointer_loc.y),
        &state.cursor_default,
        None,
        None,
        None,
        Kind::Cursor,
    ) {
        elements.push(OutputRenderElements::Cursor(cursor_elem));
    }

    let output_h = h;
    let taskbar_y = (output_h - TASKBAR_HEIGHT) as f64;
    if let Ok(bg) = MemoryRenderBufferRenderElement::from_buffer(
        &mut state.renderer,
        (0.0, taskbar_y),
        &state.taskbar_bg,
        None,
        None,
        None,
        Kind::Unspecified,
    ) {
        elements.push(OutputRenderElements::Cursor(bg));
    }

    for (i, (_, _, _, btn_buf)) in state.taskbar_buttons.iter().enumerate() {
        let x = (TASKBAR_BTN_MARGIN + i as i32 * (TASKBAR_BTN_WIDTH + TASKBAR_BTN_GAP)) as f64;
        let y = taskbar_y + ((TASKBAR_HEIGHT - TASKBAR_BTN_HEIGHT) / 2) as f64;
        if let Ok(btn) = MemoryRenderBufferRenderElement::from_buffer(
            &mut state.renderer, (x, y), btn_buf, None, None, None, Kind::Unspecified,
        ) {
            elements.push(OutputRenderElements::Cursor(btn));
        }
    }
    elements.extend(space_elements.into_iter().map(OutputRenderElements::Space));

    // Use damage tracker for offscreen render
    let mut damage_tracker = OutputDamageTracker::new(phys_size, Scale::from(1.0), Transform::Normal);
    {
        let mut target = state
            .renderer
            .bind(&mut rbo)
            .map_err(|e| anyhow::anyhow!("bind offscreen: {e}"))?;
        let _ = damage_tracker.render_output(
            &mut state.renderer,
            &mut target,
            0,
            &elements,
            [0.1, 0.1, 0.3, 1.0],
        ).map_err(|e| anyhow::anyhow!("offscreen render: {e}"))?;
    }

    // Read back pixels — rebind to get fresh target
    let region: Rectangle<i32, BufferCoord> = Rectangle::from_size(buf_size);
    let target = state
        .renderer
        .bind(&mut rbo)
        .map_err(|e| anyhow::anyhow!("rebind for readback: {e}"))?;
    let mapping = state
        .renderer
        .copy_framebuffer(&target, region, drm_fourcc::DrmFourcc::Abgr8888)
        .map_err(|e| anyhow::anyhow!("copy_framebuffer: {e}"))?;
    let pixels = state
        .renderer
        .map_texture(&mapping)
        .map_err(|e| anyhow::anyhow!("map_texture: {e}"))?;

    // Encode as PNG
    let mut png_buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_buf, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| anyhow::anyhow!("png header: {e}"))?;
        writer.write_image_data(pixels).map_err(|e| anyhow::anyhow!("png write: {e}"))?;
    }

    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_buf);
    Ok((w as u32, h as u32, b64))
}

#[cfg(target_os = "linux")]
fn detect_resize_edge(
    state: &AgentCompositor,
    location: Point<f64, Logical>,
) -> Option<(Window, u32)> {
    let windows: Vec<_> = state.space.elements().cloned().collect();
    for window in windows.iter().rev() {
        let bbox = state.space.element_bbox(window)?;
        let x = location.x;
        let y = location.y;
        let bx = bbox.loc.x as f64;
        let by = bbox.loc.y as f64;
        let bw = bbox.size.w as f64;
        let bh = bbox.size.h as f64;

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
            return Some((window.clone(), edges));
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
fn handle_input(state: &mut AgentCompositor, event: InputEvent<LibinputInputBackend>) {
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
                // Update cursor shape based on edge proximity
                let new_shape = detect_resize_edge(state, pos.to_f64())
                    .map(|(_, edges)| edges_to_cursor_shape(edges))
                    .unwrap_or(CursorShape::Default);
                if new_shape != state.cursor_shape {
                    state.cursor_shape = new_shape;
                }
                queue_redraw(state);
            }
        }
        InputEvent::PointerButton { event, .. } => {
            let serial = SERIAL_COUNTER.next_serial();
            let pointer = state.pointer.clone();

            if event.state() == smithay::backend::input::ButtonState::Pressed {
                let location = pointer.current_location();
                let output_h = state.output.current_mode().map(|m| m.size.h).unwrap_or(1080) as f64;
                let taskbar_y = output_h - TASKBAR_HEIGHT as f64;

                if location.y >= taskbar_y {
                    let btn_x = location.x - TASKBAR_BTN_MARGIN as f64;
                    let idx = (btn_x / (TASKBAR_BTN_WIDTH + TASKBAR_BTN_GAP) as f64) as usize;
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
                } else if let Some((window, edges)) = detect_resize_edge(state, location) {
                    // Edge resize
                    let initial_loc = state.space.element_location(&window).unwrap_or_default();
                    let initial_size = window
                        .toplevel()
                        .and_then(|t| t.current_state().size)
                        .unwrap_or((800, 600).into());
                    let pointer = state.pointer.clone();
                    let start_data = GrabStartData {
                        focus: None,
                        button: event.button_code(),
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
                } else {
                    // Normal click-to-focus
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

                    pointer.button(
                        state,
                        &ButtonEvent {
                            serial,
                            time: event.time_msec(),
                            button: event.button_code(),
                            state: event.state(),
                        },
                    );
                }
            } else {
                pointer.button(
                    state,
                    &ButtonEvent {
                        serial,
                        time: event.time_msec(),
                        button: event.button_code(),
                        state: event.state(),
                    },
                );
            }
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
                let new_shape = detect_resize_edge(state, pos)
                    .map(|(_, edges)| edges_to_cursor_shape(edges))
                    .unwrap_or(CursorShape::Default);
                if new_shape != state.cursor_shape {
                    state.cursor_shape = new_shape;
                }
                queue_redraw(state);
            }
        }
        _ => {}
    }
}
