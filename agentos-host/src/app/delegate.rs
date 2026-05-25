use objc2::{define_class, msg_send, rc::Retained, runtime::NSObject, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationDelegate, NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use std::cell::OnceCell;

use crate::vm::{self, VmConfig};

use super::view::FramebufferView;
use super::{DISPLAY_SCALE, NSEC_PER_MSEC};

unsafe extern "C" {
    fn dispatch_source_create(
        source_type: *const std::ffi::c_void,
        handle: usize,
        mask: usize,
        queue: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_void;

    fn dispatch_source_set_timer(
        source: *mut std::ffi::c_void,
        start: u64,
        interval: u64,
        leeway: u64,
    );

    fn dispatch_source_set_event_handler_f(
        source: *mut std::ffi::c_void,
        handler: unsafe extern "C" fn(*mut std::ffi::c_void),
    );

    fn dispatch_set_context(
        object: *mut std::ffi::c_void,
        context: *mut std::ffi::c_void,
    );

    fn dispatch_resume(object: *mut std::ffi::c_void);
    fn dispatch_walltime(when: *const std::ffi::c_void, delta: i64) -> u64;

    #[link_name = "_dispatch_main_q"]
    static DISPATCH_MAIN_Q: std::ffi::c_void;

    #[link_name = "_dispatch_source_type_timer"]
    static DISPATCH_SOURCE_TYPE_TIMER: std::ffi::c_void;
}

pub struct AppDelegateIvars {
    config: OnceCell<VmConfig>,
    window: OnceCell<Retained<NSWindow>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AgentOSAppDelegate"]
    #[ivars = AppDelegateIvars]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            self.setup_vm_and_viewer();
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(
            &self,
            _sender: &NSApplication,
        ) -> bool {
            true
        }
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker, config: VmConfig) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(AppDelegateIvars {
            config: OnceCell::from(config),
            window: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn setup_vm_and_viewer(&self) {
        let config = self.ivars().config.get().expect("config not set");
        let mtm = MainThreadMarker::new().expect("not on main thread");

        let mcp_socket_path = format!("/tmp/agentos-mcp-{}.sock", std::process::id());

        let (ctx, slirp_fd) = match vm::krun::configure_vm(config, &mcp_socket_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("failed to configure VM: {e}");
                std::process::exit(1);
            }
        };

        if let Err(e) = crate::slirp::start(slirp_fd) {
            tracing::error!("failed to start slirp: {e}");
            std::process::exit(1);
        }

        self.create_viewer_window(mtm, config);
        vm::krun::start_vm(ctx);
        self.start_display_timer();

        if config.mcp_test {
            crate::mcp::run_mcp_test(&mcp_socket_path);
        }
    }

    fn create_viewer_window(&self, mtm: MainThreadMarker, config: &VmConfig) {
        DISPLAY_SCALE.store(config.display_scale, std::sync::atomic::Ordering::Relaxed);
        let w = config.display_width as f64;
        let h = config.display_height as f64;

        let frame = NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(w, h));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                mtm.alloc::<NSWindow>(),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };

        let view = FramebufferView::new(mtm, frame);
        window.setContentView(Some(&view));
        window.setTitle(&NSString::from_str("AgentOS"));
        window.setAcceptsMouseMovedEvents(true);
        window.center();
        window.makeKeyAndOrderFront(None);
        window.makeFirstResponder(Some(&view));

        let _ = self.ivars().window.set(window);
    }

    fn start_display_timer(&self) {
        let window_ptr = self.ivars().window.get().unwrap() as *const Retained<NSWindow> as usize;

        unsafe extern "C" fn timer_handler(ctx: *mut std::ffi::c_void) {
            let window_ptr = ctx as usize;
            unsafe {
                let window = &*(window_ptr as *const Retained<NSWindow>);
                if let Some(view) = window.contentView() {
                    let fb_view: &FramebufferView =
                        &*((&*view) as *const objc2_app_kit::NSView as *const FramebufferView);
                    fb_view.update_framebuffer();
                }
            }
        }

        unsafe {
            let main_q = &raw const DISPATCH_MAIN_Q as *mut std::ffi::c_void;
            let timer_type = &raw const DISPATCH_SOURCE_TYPE_TIMER as *const std::ffi::c_void;
            let source = dispatch_source_create(timer_type, 0, 0, main_q);
            let interval_ns = 8 * NSEC_PER_MSEC;
            let leeway_ns = 1 * NSEC_PER_MSEC;
            let start = dispatch_walltime(std::ptr::null(), 0);
            dispatch_source_set_timer(source, start, interval_ns, leeway_ns);
            dispatch_set_context(source, window_ptr as *mut std::ffi::c_void);
            dispatch_source_set_event_handler_f(source, timer_handler);
            dispatch_resume(source);
        }
    }
}
