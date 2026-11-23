//! Retained AppKit parent/container. Logical editor sizes are points; backing-scale changes are AppKit's domain.
use super::*;
use objc2::{rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
pub(crate) struct Window {
    window: Retained<NSWindow>,
    view: Retained<NSView>,
    size: [u32; 2],
    closed: bool,
}
impl Window {
    pub(crate) fn new(title: &str) -> Result<Self, HostError> {
        if title.contains('\0') || title.len() > 1024 {
            return Err("invalid editor window title".into());
        }
        let mtm = MainThreadMarker::new().ok_or("AppKit editor must open on main thread")?;
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(100., 100.), NSSize::new(640., 480.));
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // Retained owns the window through plug-in GUI destruction, including after user close.
        unsafe {
            window.setReleasedWhenClosed(false);
        }
        window.setTitle(&NSString::from_str(title));
        let view = NSView::initWithFrame(
            NSView::alloc(mtm),
            NSRect::new(NSPoint::new(0., 0.), frame.size),
        );
        window.setContentView(Some(&view));
        window.makeKeyAndOrderFront(None);
        Ok(Self {
            window,
            view,
            size: [640, 480],
            closed: false,
        })
    }
    pub(crate) fn parent(&self) -> ParentWindow {
        ParentWindow::Cocoa(Retained::as_ptr(&self.view) as *mut c_void)
    }
    pub(crate) fn resize(&mut self, size: [u32; 2]) {
        if size == self.size {
            return;
        }
        self.size = size;
        let size = NSSize::new(size[0] as f64, size[1] as f64);
        self.window.setContentSize(size);
        self.view.setFrameSize(size);
    }
    pub(crate) fn focus(&self) {
        self.window.makeKeyAndOrderFront(None);
        self.window.makeFirstResponder(Some(&self.view));
    }
    pub(crate) fn events(&mut self) -> Vec<NativeEvent> {
        let mut out = Vec::new();
        let Some(mtm) = MainThreadMarker::new() else {
            return out;
        };
        if !self.closed
            && !self.window.isVisible()
            && !self.window.isMiniaturized()
            && !NSApplication::sharedApplication(mtm).isHidden()
        {
            self.closed = true;
            out.push(NativeEvent::Close);
            return out;
        }
        let frame = self
            .window
            .contentView()
            .map(|v| v.frame())
            .unwrap_or_else(|| self.view.frame());
        let size = [
            frame.size.width.round().clamp(64., 8192.) as u32,
            frame.size.height.round().clamp(64., 8192.) as u32,
        ];
        if self.size != size {
            self.size = size;
            self.view.setFrameSize(frame.size);
            out.push(NativeEvent::Resize(size));
        }
        out
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        self.window.close();
    }
}
