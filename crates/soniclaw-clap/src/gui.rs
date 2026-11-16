//! Main-thread GUI ABI. Native handles never enter the audio runtime.
use super::*;
use clap_sys::ext::gui::*;
use soniclaw_editor::{
    host::{Host, HostCallbacks, HostMainThreadCaller},
    raw_window_handle::*,
    Editor,
};
pub(super) struct GuiState {
    created: bool,
    width: u32,
    height: u32,
    scale: f64,
    pub editor: Option<Editor>,
}
impl Default for GuiState {
    fn default() -> Self {
        Self {
            created: false,
            width: 560,
            height: 300,
            scale: 1.0,
            editor: None,
        }
    }
}
#[derive(Clone, Copy)]
struct GuiHost(usize);
impl HostMainThreadCaller for GuiHost {
    fn call_main_thread(&mut self) {
        unsafe {
            let host = self.0 as *const clap_host;
            if let Some(f) = (*host).request_callback {
                f(host);
            }
        }
    }
}
impl HostCallbacks for GuiHost {
    fn request_resize(
        &mut self,
        size: soniclaw_editor::WindowSize,
    ) -> Result<(), soniclaw_editor::HandlerError> {
        unsafe {
            let host = self.0 as *const clap_host;
            if let Some(get) = (*host).get_extension {
                let ext = get(host, CLAP_EXT_GUI.as_ptr()).cast::<clap_host_gui>();
                if let Some(ext) = ext.as_ref() {
                    if ext
                        .request_resize
                        .is_some_and(|f| f(host, size.physical.width, size.physical.height))
                    {
                        return Ok(());
                    }
                }
            }
            Err(std::io::Error::other("host declined editor resize").into())
        }
    }
    fn destroyed(&mut self) {
        unsafe {
            let host = self.0 as *const clap_host;
            if let Some(get) = (*host).get_extension {
                if let Some(ext) = get(host, CLAP_EXT_GUI.as_ptr())
                    .cast::<clap_host_gui>()
                    .as_ref()
                {
                    if let Some(f) = ext.closed {
                        f(host, true);
                    }
                }
            }
        }
    }
}
fn api() -> &'static CStr {
    #[cfg(target_os = "linux")]
    {
        CLAP_WINDOW_API_X11
    }
    #[cfg(target_os = "windows")]
    {
        CLAP_WINDOW_API_WIN32
    }
    #[cfg(target_os = "macos")]
    {
        CLAP_WINDOW_API_COCOA
    }
}
unsafe extern "C" fn supported(_: *const clap_plugin, a: *const c_char, floating: bool) -> bool {
    unsafe { !floating && !a.is_null() && CStr::from_ptr(a) == api() }
}
unsafe extern "C" fn preferred(
    _: *const clap_plugin,
    a: *mut *const c_char,
    floating: *mut bool,
) -> bool {
    unsafe {
        let (Some(a), Some(f)) = (a.as_mut(), floating.as_mut()) else {
            return false;
        };
        *a = api().as_ptr();
        *f = false;
        true
    }
}
unsafe extern "C" fn create(p: *const clap_plugin, a: *const c_char, f: bool) -> bool {
    unsafe {
        if !supported(p, a, f) {
            return false;
        }
        let Some(i) = instance(p) else {
            return false;
        };
        let state = &mut *i.gui.get();
        if state.created {
            return false;
        }
        // CLAP create establishes GUI state; the native window is initialized
        // after set_parent supplies a valid host window. Size queries work now.
        state.created = true;
        true
    }
}
unsafe fn make_editor(i: &Instance, raw: RawWindowHandle, size: (u32, u32)) -> Option<Editor> {
    unsafe {
        let address = i.host as usize;
        // Lifetime: baseview destroys callbacks synchronously when Editor is dropped;
        // plugin destroy drops the GUI before releasing the instance/host association.
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let host = address as *const clap_host;
            if let Some(get) = (*host).get_extension {
                if let Some(params) = get(host, CLAP_EXT_PARAMS.as_ptr())
                    .cast::<clap_host_params>()
                    .as_ref()
                {
                    if let Some(flush) = params.request_flush {
                        flush(host);
                        return;
                    }
                }
            }
            if let Some(f) = (*host).request_process {
                f(host);
            }
        });
        let h = GuiHost(address);
        let host = Host::new().with_main_thread(h).with_callbacks(h);
        Editor::create_parented(
            CStr::from_ptr(i.definition.descriptor.raw.name)
                .to_string_lossy()
                .into_owned(),
            i.parameters.to_vec(),
            i.editor_sender.clone(),
            notify,
            host,
            size,
            raw,
        )
        .map_err(|e| {
            if std::env::var_os("SONICLAW_EDITOR_TRACE").is_some() {
                eprintln!("Soniclaw editor initialization failed: {e}");
            }
            e
        })
        .ok()
    }
}
unsafe extern "C" fn destroy(p: *const clap_plugin) {
    unsafe {
        if let Some(i) = instance(p) {
            let _ = catch_unwind(AssertUnwindSafe(|| {
                (*i.gui.get()).created = false;
                (*i.gui.get()).editor.take();
            }));
        }
    }
}
unsafe extern "C" fn scale(p: *const clap_plugin, value: f64) -> bool {
    unsafe {
        if !value.is_finite() || !(0.5..=4.0).contains(&value) {
            return false;
        }
        let Some(i) = instance(p) else {
            return false;
        };
        if !(*i.gui.get()).created {
            return false;
        }
        (*i.gui.get()).scale = value;
        (&*i.gui.get())
            .editor
            .as_ref()
            .is_none_or(|e| e.set_scale(value))
    }
}
unsafe extern "C" fn size(p: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    unsafe {
        let (Some(i), Some(w), Some(h)) = (instance(p), w.as_mut(), h.as_mut()) else {
            return false;
        };
        let state = &*i.gui.get();
        if !state.created {
            return false;
        }
        (*w, *h) = state
            .editor
            .as_ref()
            .map_or((state.width, state.height), Editor::size);
        true
    }
}
unsafe extern "C" fn resizable(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn hints(_: *const clap_plugin, out: *mut clap_gui_resize_hints) -> bool {
    unsafe {
        let Some(o) = out.as_mut() else {
            return false;
        };
        *o = clap_gui_resize_hints {
            can_resize_horizontally: true,
            can_resize_vertically: true,
            preserve_aspect_ratio: false,
            aspect_ratio_width: 0,
            aspect_ratio_height: 0,
        };
        true
    }
}
unsafe extern "C" fn adjust(_: *const clap_plugin, w: *mut u32, h: *mut u32) -> bool {
    unsafe {
        let (Some(w), Some(h)) = (w.as_mut(), h.as_mut()) else {
            return false;
        };
        *w = (*w).clamp(320, 4096);
        *h = (*h).clamp(220, 2160);
        true
    }
}
unsafe extern "C" fn resize(p: *const clap_plugin, w: u32, h: u32) -> bool {
    unsafe {
        if !(320..=4096).contains(&w) || !(220..=2160).contains(&h) {
            return false;
        }
        let Some(i) = instance(p) else {
            return false;
        };
        if !(*i.gui.get()).created {
            return false;
        }
        // Do not keep an exclusive GuiState borrow across native/host callbacks.
        (*i.gui.get()).width = w;
        (*i.gui.get()).height = h;
        (&*i.gui.get())
            .editor
            .as_ref()
            .is_none_or(|e| e.resize(w, h))
    }
}
unsafe extern "C" fn parent(p: *const clap_plugin, window: *const clap_window) -> bool {
    unsafe {
        catch_unwind(AssertUnwindSafe(|| {
            let (Some(i), Some(w)) = (instance(p), window.as_ref()) else {
                return false;
            };
            if !supported(p, w.api, false) {
                return false;
            }
            if !(*i.gui.get()).created {
                return false;
            }
            #[cfg(target_os = "linux")]
            let raw = {
                if w.specific.x11 == 0 {
                    return false;
                }
                RawWindowHandle::Xlib(XlibWindowHandle::new(w.specific.x11))
            };
            #[cfg(target_os = "windows")]
            let raw = {
                let Some(hwnd) = std::num::NonZeroIsize::new(w.specific.win32 as isize) else {
                    return false;
                };
                RawWindowHandle::Win32(Win32WindowHandle::new(hwnd))
            };
            #[cfg(target_os = "macos")]
            let raw = {
                let Some(view) = std::ptr::NonNull::new(w.specific.cocoa) else {
                    return false;
                };
                RawWindowHandle::AppKit(AppKitWindowHandle::new(view))
            };
            if let Some(e) = (&*i.gui.get()).editor.as_ref() {
                return e.set_parent(raw);
            }
            let size = ((*i.gui.get()).width, (*i.gui.get()).height);
            let Some(editor) = make_editor(i, raw, size) else {
                return false;
            };
            let _ = editor.set_scale((*i.gui.get()).scale);
            (*i.gui.get()).editor = Some(editor);
            true
        }))
        .unwrap_or(false)
    }
}
unsafe extern "C" fn transient(_: *const clap_plugin, _: *const clap_window) -> bool {
    false
}
unsafe extern "C" fn title(_: *const clap_plugin, _: *const c_char) {}
unsafe extern "C" fn show(p: *const clap_plugin) -> bool {
    unsafe {
        instance(p)
            .and_then(|i| (&*i.gui.get()).editor.as_ref())
            .is_some_and(Editor::show)
    }
}
unsafe extern "C" fn hide(p: *const clap_plugin) -> bool {
    unsafe {
        instance(p)
            .and_then(|i| (&*i.gui.get()).editor.as_ref())
            .is_some_and(Editor::hide)
    }
}
pub(super) static GUI: clap_plugin_gui = clap_plugin_gui {
    is_api_supported: Some(supported),
    get_preferred_api: Some(preferred),
    create: Some(create),
    destroy: Some(destroy),
    set_scale: Some(scale),
    get_size: Some(size),
    can_resize: Some(resizable),
    get_resize_hints: Some(hints),
    adjust_size: Some(adjust),
    set_size: Some(resize),
    set_parent: Some(parent),
    set_transient: Some(transient),
    suggest_title: Some(title),
    show: Some(show),
    hide: Some(hide),
};
