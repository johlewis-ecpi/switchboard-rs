#[cfg(target_os = "macos")]
pub struct MagnifyMonitor(Option<objc2::rc::Retained<objc2::runtime::AnyObject>>);

#[cfg(target_os = "macos")]
impl Drop for MagnifyMonitor {
    fn drop(&mut self) {
        if let Some(monitor) = &self.0 {
            // This token came from NSEvent's local monitor registration.
            unsafe { objc2_app_kit::NSEvent::removeMonitor(monitor) };
        }
    }
}

#[cfg(target_os = "macos")]
pub fn monitor_magnify(
    window: &tao::window::Window,
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handler: impl Fn(f64, bool) + 'static,
) -> MagnifyMonitor {
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventPhase, NSWindow};
    use std::{ptr::NonNull, sync::atomic::Ordering};
    use tao::platform::macos::WindowExtMacOS;
    // tao owns the window; this function and the monitor run on the UI thread.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    let number = native.windowNumber();
    let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| {
        // AppKit guarantees the event is valid for this callback.
        let value = unsafe { event.as_ref() };
        if value.windowNumber() != number || !enabled.load(Ordering::Relaxed) {
            return event.as_ptr();
        }
        handler(
            value.magnification(),
            value
                .phase()
                .intersects(NSEventPhase::Ended | NSEventPhase::Cancelled),
        );
        // Consume the event so WebKit cannot also zoom the entire page.
        std::ptr::null_mut()
    });
    MagnifyMonitor(unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::Magnify, &block)
    })
}

#[cfg(not(target_os = "macos"))]
pub struct MagnifyMonitor;

#[cfg(not(target_os = "macos"))]
pub fn monitor_magnify(
    _: &tao::window::Window,
    _: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _: impl Fn(f64, bool) + 'static,
) -> MagnifyMonitor {
    MagnifyMonitor
}

#[cfg(target_os = "macos")]
pub fn restore_library_window_frame(window: &tao::window::Window) {
    use objc2_app_kit::NSWindow;
    use objc2_foundation::NSString;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tao::platform::macos::WindowExtMacOS;
    // AppKit requires a distinct autosave name for each live window. Creation
    // order gives the main window and additional library windows stable slots.
    static NEXT_SLOT: AtomicUsize = AtomicUsize::new(0);
    let slot = NEXT_SLOT.fetch_add(1, Ordering::Relaxed);
    let name = NSString::from_str(&format!("SwitchboardLibraryWindow-{slot}"));
    // Called on the UI thread while tao owns the native window.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    native.setFrameUsingName(&name);
    native.setFrameAutosaveName(&name);
}

#[cfg(not(target_os = "macos"))]
pub fn restore_library_window_frame(_: &tao::window::Window) {}

#[cfg(target_os = "macos")]
pub fn save_library_window_frame(window: &tao::window::Window) {
    use objc2_app_kit::NSWindow;
    use tao::platform::macos::WindowExtMacOS;
    if window.is_minimized() || window.fullscreen().is_some() {
        return;
    }
    // Called on the UI thread while tao owns the native window.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    let name = native.frameAutosaveName();
    if !name.is_empty() {
        native.saveFrameUsingName(&name);
    }
}

#[cfg(not(target_os = "macos"))]
pub fn save_library_window_frame(_: &tao::window::Window) {}

#[cfg(target_os = "macos")]
pub fn previous_app() -> Option<i32> {
    objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier())
}
#[cfg(not(target_os = "macos"))]
pub fn previous_app() -> Option<i32> {
    None
}
#[cfg(target_os = "macos")]
pub fn restore_app(pid: Option<i32>) {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    if let Some(app) = pid.and_then(NSRunningApplication::runningApplicationWithProcessIdentifier) {
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    }
}
#[cfg(not(target_os = "macos"))]
pub fn restore_app(_: Option<i32>) {}
#[cfg(target_os = "macos")]
pub fn place_launcher(window: &tao::window::Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSEvent, NSScreen, NSWindow, NSWindowCollectionBehavior};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tao::platform::macos::WindowExtMacOS;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let point = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .find(|s| {
            let r = s.frame();
            point.x >= r.origin.x
                && point.x < r.origin.x + r.size.width
                && point.y >= r.origin.y
                && point.y < r.origin.y + r.size.height
        })
        .or_else(|| screens.firstObject());
    let Some(screen) = screen else { return };
    let r = screen.visibleFrame();
    let width = 640f64.min((r.size.width - 32.).max(100.));
    let height = 460f64.min((r.size.height - 32.).max(100.));
    let x = r.origin.x + (r.size.width - width) / 2.;
    let y = r.origin.y + (r.size.height - height) * 0.65;
    // The window is owned by tao and remains valid throughout this call on the main thread.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    native.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    native.setFrame_display(
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)),
        true,
    );
}
#[cfg(not(target_os = "macos"))]
pub fn place_launcher(_: &tao::window::Window) {}

/// Give previews their own frame, independent of the saved library window size.
#[cfg(target_os = "macos")]
pub fn place_preview(window: &tao::window::Window) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWindow};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tao::platform::macos::WindowExtMacOS;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // The first screen is the primary display; mainScreen follows keyboard focus.
    let Some(screen) = NSScreen::screens(mtm).firstObject() else {
        return;
    };
    let screen = screen.frame();
    let width = screen.size.width * 0.75;
    let height = screen.size.height * 0.75;
    let frame = NSRect::new(
        NSPoint::new(
            screen.origin.x + (screen.size.width - width) / 2.,
            screen.origin.y + (screen.size.height - height) / 2.,
        ),
        NSSize::new(width, height),
    );
    // AppKit sizes the entire window, including its title bar, in screen points.
    let native = unsafe { &*(window.ns_window() as *const NSWindow) };
    native.setFrame_display(frame, false);
}

#[cfg(not(target_os = "macos"))]
pub fn place_preview(window: &tao::window::Window) {
    use tao::dpi::{PhysicalPosition, PhysicalSize};
    let Some(screen) = window.primary_monitor() else {
        return;
    };
    let size = screen.size();
    let width = size.width * 3 / 4;
    let height = size.height * 3 / 4;
    window.set_inner_size(PhysicalSize::new(width, height));
    let outer = window.outer_size();
    let origin = screen.position();
    window.set_outer_position(PhysicalPosition::new(
        origin.x + (size.width as i32 - outer.width as i32) / 2,
        origin.y + (size.height as i32 - outer.height as i32) / 2,
    ));
}

/// Position a passive tooltip using the display under its anchor, not its owner bounds.
#[cfg(target_os = "macos")]
pub fn place_hover(
    owner: &tao::window::Window,
    popup: &tao::window::Window,
    anchor: [f64; 2],
    height: f64,
) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSScreen, NSWindow};
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use tao::platform::macos::WindowExtMacOS;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let owner = unsafe { &*(owner.ns_window() as *const NSWindow) };
    let Some(content) = owner.contentView() else {
        return;
    };
    let point = owner.convertPointToScreen(NSPoint::new(
        anchor[0],
        content.bounds().size.height - anchor[1],
    ));
    let screens = NSScreen::screens(mtm);
    let Some(screen) = screens
        .iter()
        .find(|screen| {
            let r = screen.frame();
            point.x >= r.origin.x
                && point.x < r.origin.x + r.size.width
                && point.y >= r.origin.y
                && point.y < r.origin.y + r.size.height
        })
        .or_else(|| screens.firstObject())
    else {
        return;
    };
    let r = screen.visibleFrame();
    let [x, y, width, height] = crate::hover_popup::popup_frame(
        [
            r.origin.x,
            -r.origin.y - r.size.height,
            r.size.width,
            r.size.height,
        ],
        [point.x, -point.y],
        [400., height],
    );
    let native = unsafe { &*(popup.ns_window() as *const NSWindow) };
    native.setHasShadow(true);
    native.setFrame_display(
        NSRect::new(NSPoint::new(x, -y - height), NSSize::new(width, height)),
        true,
    );
    // tao's set_visible(true) makes a window key on macOS. A hover must not do so.
    native.orderFront(None);
}

#[cfg(not(target_os = "macos"))]
pub fn place_hover(
    owner: &tao::window::Window,
    popup: &tao::window::Window,
    anchor: [f64; 2],
    height: f64,
) {
    use tao::dpi::{PhysicalPosition, PhysicalSize};
    let Ok(origin) = owner.inner_position() else {
        return;
    };
    let scale = owner.scale_factor();
    let point = [
        origin.x as f64 + anchor[0] * scale,
        origin.y as f64 + anchor[1] * scale,
    ];
    let Some(screen) = owner
        .available_monitors()
        .find(|screen| {
            let origin = screen.position();
            let size = screen.size();
            point[0] >= origin.x as f64
                && point[0] < origin.x as f64 + size.width as f64
                && point[1] >= origin.y as f64
                && point[1] < origin.y as f64 + size.height as f64
        })
        .or_else(|| owner.primary_monitor())
    else {
        return;
    };
    let origin = screen.position();
    let size = screen.size();
    let [x, y, width, height] = crate::hover_popup::popup_frame(
        [
            origin.x as f64,
            origin.y as f64,
            size.width as f64,
            size.height as f64,
        ],
        point,
        [400. * screen.scale_factor(), height * screen.scale_factor()],
    );
    popup.set_inner_size(PhysicalSize::new(width as u32, height as u32));
    popup.set_outer_position(PhysicalPosition::new(x as i32, y as i32));
    popup.set_visible(true);
}
