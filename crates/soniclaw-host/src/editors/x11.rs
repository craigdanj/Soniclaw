//! Xlib c_ulong width varies by target; explicit casts preserve the ABI.
#![allow(clippy::unnecessary_cast)]
use super::NativeEvent as Event;
use super::*;
use x11_dl::xlib;
pub(crate) struct Window {
    api: xlib::Xlib,
    display: *mut xlib::Display,
    window: xlib::Window,
    delete: xlib::Atom,
    size: [u32; 2],
}
impl Window {
    pub(crate) fn new(title: &str) -> Result<Self, HostError> {
        let title = CString::new(title)?;
        let api = xlib::Xlib::open()?;
        unsafe {
            (api.XInitThreads)();
            let display = (api.XOpenDisplay)(ptr::null());
            if display.is_null() {
                return Err("X11 display unavailable".into());
            }
            let root = (api.XDefaultRootWindow)(display);
            let window = (api.XCreateSimpleWindow)(display, root, 20, 20, 640, 480, 0, 0, 0x20242a);
            if window == 0 {
                (api.XCloseDisplay)(display);
                return Err("X11 parent creation failed".into());
            }
            (api.XStoreName)(display, window, title.as_ptr());
            let mut delete = (api.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (api.XSetWMProtocols)(display, window, &mut delete, 1);
            (api.XSelectInput)(
                display,
                window,
                xlib::StructureNotifyMask | xlib::FocusChangeMask,
            );
            (api.XMapWindow)(display, window);
            (api.XFlush)(display);
            Ok(Self {
                api,
                display,
                window,
                delete,
                size: [640, 480],
            })
        }
    }
    pub(crate) fn parent(&self) -> ParentWindow {
        ParentWindow::X11(self.handle())
    }
    pub(crate) fn handle(&self) -> u64 {
        self.window as u64
    }
    pub(crate) fn resize(&mut self, size: [u32; 2]) {
        if self.size == size {
            return;
        }
        self.size = size;
        unsafe {
            (self.api.XResizeWindow)(self.display, self.window, size[0], size[1]);
            (self.api.XFlush)(self.display);
        }
    }
    pub(crate) fn focus(&self) {
        unsafe {
            (self.api.XSetInputFocus)(
                self.display,
                self.window,
                xlib::RevertToParent,
                xlib::CurrentTime,
            );
            (self.api.XFlush)(self.display);
        }
    }
    pub(crate) fn events(&mut self) -> Vec<Event> {
        let mut out = vec![];
        unsafe {
            for _ in 0..128 {
                if (self.api.XPending)(self.display) == 0 {
                    break;
                }
                let mut event: xlib::XEvent = std::mem::zeroed();
                (self.api.XNextEvent)(self.display, &mut event);
                match event.get_type() {
                    xlib::ClientMessage
                        if event.client_message.data.get_long(0) as xlib::Atom == self.delete =>
                    {
                        out.push(Event::Close)
                    }
                    xlib::ConfigureNotify => {
                        let size = [
                            event.configure.width.max(64) as u32,
                            event.configure.height.max(64) as u32,
                        ];
                        if size != self.size {
                            self.size = size;
                            out.push(Event::Resize(size));
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }
    pub(crate) fn capture(&self, path: &Path) -> Result<(), HostError> {
        use std::io::Write;
        unsafe {
            (self.api.XSync)(self.display, 0);
            let image = (self.api.XGetImage)(
                self.display,
                self.window,
                0,
                0,
                self.size[0],
                self.size[1],
                !0,
                xlib::ZPixmap,
            );
            if image.is_null() {
                return Err("X11 editor capture failed".into());
            }
            let mut pixels = Vec::with_capacity(self.size[0] as usize * self.size[1] as usize * 3);
            let channel = |p: u64, mask: u64| {
                if mask == 0 {
                    0
                } else {
                    (((p & mask) >> mask.trailing_zeros()) * 255 / (mask >> mask.trailing_zeros()))
                        as u8
                }
            };
            for y in 0..self.size[1] {
                for x in 0..self.size[0] {
                    let pixel = (self.api.XGetPixel)(image, x as i32, y as i32) as u64;
                    pixels.extend([
                        channel(pixel, (*image).red_mask as u64),
                        channel(pixel, (*image).green_mask as u64),
                        channel(pixel, (*image).blue_mask as u64),
                    ]);
                }
            }
            (self.api.XDestroyImage)(image);
            let mut f = std::fs::File::create(path)?;
            write!(f, "P6\n{} {}\n255\n", self.size[0], self.size[1])?;
            f.write_all(&pixels)?;
            Ok(())
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            (self.api.XDestroyWindow)(self.display, self.window);
            (self.api.XCloseDisplay)(self.display);
        }
    }
}
