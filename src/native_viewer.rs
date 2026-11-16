//! UI-thread-owned Quick Look view, placed over the document host in WKWebView.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub struct NativeViewer(
    Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
    Option<objc2::rc::Retained<objc2::runtime::AnyObject>>,
);

#[cfg(target_os = "macos")]
impl NativeViewer {
    pub fn close(&mut self) {
        if let Some(monitor) = self.1.take() {
            unsafe { objc2_app_kit::NSEvent::removeMonitor(&monitor) };
        }
        if let Some(view) = self.0.take() {
            unsafe {
                let _: () = objc2::msg_send![&*view, close];
                let _: () = objc2::msg_send![&*view, removeFromSuperview];
            }
        }
    }

    pub fn show(
        &mut self,
        window: &tao::window::Window,
        path: &std::path::Path,
        rect: [f64; 4],
        on_close: impl Fn() + 'static,
    ) -> Result<(), String> {
        use objc2::{
            msg_send,
            rc::Retained,
            runtime::{AnyClass, AnyObject},
        };
        use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, NSURL};
        use tao::platform::macos::WindowExtMacOS;
        #[link(name = "Quartz", kind = "framework")]
        unsafe extern "C" {}
        if rect.iter().any(|n| !n.is_finite()) || rect[2] < 1. || rect[3] < 1. {
            return Err("Invalid viewer bounds".into());
        }
        unsafe {
            let native = &*(window.ns_window() as *const AnyObject);
            let parent: Retained<AnyObject> = msg_send![native, contentView];
            let bounds: NSRect = msg_send![&*parent, bounds];
            let frame = NSRect::new(
                NSPoint::new(rect[0], bounds.size.height - rect[1] - rect[3]),
                NSSize::new(rect[2], rect[3]),
            );
            if self.0.is_none() {
                let class =
                    AnyClass::get(c"QLPreviewView").ok_or("Quick Look viewer unavailable")?;
                let allocated: objc2::rc::Allocated<AnyObject> = msg_send![class, alloc];
                let view: Retained<AnyObject> =
                    msg_send![allocated, initWithFrame:frame, style:0isize];
                let _: () = msg_send![&*view, setShouldCloseWithWindow:true];
                let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                let _: () = msg_send![&*view, setPreviewItem:&*url];
                let _: () = msg_send![&*parent, addSubview:&*view];
                // Quick Look receives native key events rather than DOM events.
                // Only intercept plain Space inside this preview's view subtree.
                use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags};
                use std::ptr::NonNull;
                let number: isize = msg_send![native, windowNumber];
                let preview = view.clone();
                let native_window =
                    objc2::rc::Retained::retain(window.ns_window() as *mut AnyObject)
                        .ok_or("Window unavailable")?;
                let block = block2::RcBlock::new(move |event: NonNull<NSEvent>| {
                    let event_value = event.as_ref();
                    let modifiers = NSEventModifierFlags::Command
                        | NSEventModifierFlags::Control
                        | NSEventModifierFlags::Option
                        | NSEventModifierFlags::Shift;
                    if event_value.windowNumber() != number
                        || event_value.keyCode() != 49
                        || event_value.modifierFlags().intersects(modifiers)
                    {
                        return event.as_ptr();
                    }
                    let responder: Option<Retained<AnyObject>> =
                        msg_send![&*native_window, firstResponder];
                    let Some(responder) = responder else {
                        return event.as_ptr();
                    };
                    for name in [c"NSTextView", c"NSTextField", c"NSButton"] {
                        if let Some(class) = AnyClass::get(name) {
                            let matches: bool = msg_send![&*responder, isKindOfClass:class];
                            if matches {
                                if name == c"NSButton" {
                                    return event.as_ptr();
                                }
                                let editable: bool = msg_send![&*responder, isEditable];
                                if editable {
                                    return event.as_ptr();
                                }
                            }
                        }
                    }
                    let is_view: bool =
                        msg_send![&*responder, respondsToSelector:objc2::sel!(isDescendantOf:)];
                    if !is_view {
                        return event.as_ptr();
                    }
                    let inside: bool = msg_send![&*responder, isDescendantOf:&*preview];
                    if !inside {
                        return event.as_ptr();
                    }
                    if !event_value.isARepeat() {
                        on_close();
                    }
                    std::ptr::null_mut()
                });
                self.1 = NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown,
                    &block,
                );
                self.0 = Some(view);
            } else if let Some(view) = &self.0 {
                let _: () = msg_send![&**view, setFrame:frame];
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
impl Drop for NativeViewer {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(not(target_os = "macos"))]
#[derive(Default)]
pub struct NativeViewer;
#[cfg(not(target_os = "macos"))]
impl NativeViewer {
    pub fn close(&mut self) {}
    pub fn show(
        &mut self,
        _: &tao::window::Window,
        _: &std::path::Path,
        _: [f64; 4],
        _: impl Fn() + 'static,
    ) -> Result<(), String> {
        Err("Office viewing requires macOS".into())
    }
}
