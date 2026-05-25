#[cfg(target_os = "macos")]
mod delegate;
#[cfg(target_os = "macos")]
pub mod view;

#[cfg(target_os = "macos")]
use crate::vm::VmConfig;
#[cfg(target_os = "macos")]
use anyhow::Result;
#[cfg(target_os = "macos")]
use objc2::{msg_send, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
#[cfg(target_os = "macos")]
use std::sync::Mutex;

#[cfg(target_os = "macos")]
static DISPLAY_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
#[cfg(target_os = "macos")]
static LAST_DISPLAYED_SURFACE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "macos")]
static PRESSED_KEYS: Mutex<Option<std::collections::HashSet<u16>>> = Mutex::new(None);

#[cfg(target_os = "macos")]
const NSEC_PER_MSEC: u64 = 1_000_000;

#[cfg(target_os = "macos")]
fn ca_transaction_begin() {
    unsafe {
        let cls: &objc2::runtime::AnyClass = objc2::runtime::AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![cls, begin];
    }
}

#[cfg(target_os = "macos")]
fn ca_transaction_set_disable_actions(disable: bool) {
    unsafe {
        let cls: &objc2::runtime::AnyClass = objc2::runtime::AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![cls, setDisableActions: disable];
    }
}

#[cfg(target_os = "macos")]
fn ca_transaction_commit() {
    unsafe {
        let cls: &objc2::runtime::AnyClass = objc2::runtime::AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![cls, commit];
    }
}

#[cfg(target_os = "macos")]
pub fn run(config: VmConfig) -> Result<()> {
    let mtm = MainThreadMarker::new().expect("must run on main thread");

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    setup_menu(mtm, &app);

    let delegate = delegate::AppDelegate::new(mtm, config);
    app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));

    app.run();

    Ok(())
}

#[cfg(target_os = "macos")]
fn setup_menu(mtm: MainThreadMarker, app: &NSApplication) {
    use objc2_app_kit::{NSMenu, NSMenuItem};

    unsafe {
        let menubar = NSMenu::new(mtm);
        let app_menu_item = NSMenuItem::new(mtm);
        menubar.addItem(&app_menu_item);
        app.setMainMenu(Some(&menubar));

        let app_menu = NSMenu::new(mtm);
        let quit_item = NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc::<NSMenuItem>(),
            &NSString::from_str("Quit AgentOS"),
            Some(objc2::sel!(terminate:)),
            &NSString::from_str("q"),
        );
        app_menu.addItem(&quit_item);
        app_menu_item.setSubmenu(Some(&app_menu));
    }
}
