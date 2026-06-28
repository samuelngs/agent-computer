#[cfg(target_os = "linux")]
use smithay::{
    desktop::Window,
    wayland::{compositor, shell::xdg::XdgToplevelSurfaceData},
};

#[cfg(target_os = "linux")]
pub(crate) fn get_window_title(window: &Window) -> String {
    window
        .toplevel()
        .and_then(|t| {
            compositor::with_states(t.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| {
                        let attrs = d.lock().ok()?;
                        attrs.title.clone().or_else(|| attrs.app_id.clone())
                    })
            })
        })
        .unwrap_or_default()
}
