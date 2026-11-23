//! Main-thread native editor parenting, resize, scale and owned desktop windows.
use super::{live::*, *};
use clap_sys::ext::gui::*;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
pub(crate) use x11::Window;
#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
pub(crate) use win32::Window;
#[cfg(target_os = "macos")]
mod cocoa;
#[cfg(target_os = "macos")]
pub(crate) use cocoa::Window;
#[allow(dead_code)] // Each OS emits only the native events its protocol supports.
pub(crate) enum NativeEvent {
    Close,
    Resize([u32; 2]),
    Scale(f64),
}
#[derive(Clone, Copy, Debug)]
pub enum ParentWindow {
    X11(u64),
    Win32(*mut c_void),
    Cocoa(*mut c_void),
}
#[derive(Clone, Debug, Serialize)]
pub enum EditorEvent {
    Resized { node: usize, size: [u32; 2] },
    Closed { node: usize },
    Error { node: usize, message: String },
}
impl ParentWindow {
    fn native(self) -> Result<clap_window, HostError> {
        match self {
            Self::X11(id) if cfg!(target_os = "linux") && id != 0 => Ok(clap_window {
                api: CLAP_WINDOW_API_X11.as_ptr(),
                specific: clap_window_handle { x11: id as _ },
            }),
            Self::Win32(handle) if cfg!(target_os = "windows") && !handle.is_null() => {
                Ok(clap_window {
                    api: CLAP_WINDOW_API_WIN32.as_ptr(),
                    specific: clap_window_handle { win32: handle },
                })
            }
            Self::Cocoa(handle) if cfg!(target_os = "macos") && !handle.is_null() => {
                Ok(clap_window {
                    api: CLAP_WINDOW_API_COCOA.as_ptr(),
                    specific: clap_window_handle { cocoa: handle },
                })
            }
            _ => Err("invalid/wrong-platform editor parent".into()),
        }
    }
}
impl LiveRack {
    pub fn editor_size(&self, node: usize) -> Option<[u32; 2]> {
        self.editors
            .get(node)
            .and_then(Option::as_ref)
            .map(|e| e.size)
    }

    /// # Safety
    /// Parent must be a valid native view/window on this GUI thread and remain alive
    /// until close_editor/owner destruction completes. Handle queued resize events for external parents.
    pub unsafe fn attach_editor(
        &mut self,
        node: usize,
        parent: ParentWindow,
    ) -> Result<[u32; 2], HostError> {
        let window = parent.native()?;
        self.close_editor(node);
        let instance = self.instances.get(node).ok_or("unknown editor node")?;
        unsafe {
            let ext = (*instance.plugin).get_extension.expect("checked")(
                instance.plugin,
                CLAP_EXT_GUI.as_ptr(),
            )
            .cast::<clap_plugin_gui>();
            if ext.is_null() {
                return Err("plug-in has no GUI extension".into());
            }
            let e = &*ext;
            if !e.is_api_supported.ok_or("missing GUI support query")?(
                instance.plugin,
                window.api,
                false,
            ) {
                return Err("plug-in does not support embedded editor API".into());
            }
            let destroy = e.destroy.ok_or("missing GUI destroy")?;
            let parent_fn = e.set_parent.ok_or("missing GUI parent setter")?;
            let size = e.get_size.ok_or("missing GUI size getter")?;
            let show = e.show.ok_or("missing GUI show")?;
            let create = e.create.ok_or("missing GUI create")?;
            instance.context.gui_embedded.store(true, Ordering::Release);
            if !create(instance.plugin, window.api, false) {
                instance
                    .context
                    .gui_embedded
                    .store(false, Ordering::Release);
                return Err("embedded editor creation failed".into());
            }
            let desired_scale = f64::from_bits(instance.context.gui_scale.load(Ordering::Acquire));
            let applied_scale = if let Some(set_scale) = e.set_scale {
                if set_scale(instance.plugin, desired_scale) {
                    desired_scale
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let (mut w, mut h) = (0, 0);
            if !size(instance.plugin, &mut w, &mut h)
                || !valid_size([w, h])
                || !parent_fn(instance.plugin, &window)
                || !show(instance.plugin)
            {
                destroy(instance.plugin);
                instance
                    .context
                    .gui_embedded
                    .store(false, Ordering::Release);
                return Err("embedded editor attach/size/show failed".into());
            }
            instance
                .context
                .gui_last_size
                .store(((w as u64) << 32) | h as u64, Ordering::Release);
            instance
                .context
                .gui_last_embedded
                .store(true, Ordering::Release);
            self.editors[node] = Some(Editor {
                plugin: instance.plugin,
                extension: ext,
                scale: applied_scale,
                embedded: true,
                size: [w, h],
                context: &*instance.context,
                #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
                owned: None,
            });
            Ok([w, h])
        }
    }
    /// Own a native desktop parent through native GUI destruction. Call/poll on the main GUI thread.
    pub fn show_embedded_editor(&mut self, node: usize, title: &str) -> Result<(), HostError> {
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        {
            let mut window = Window::new(title)?;
            let size = unsafe { self.attach_editor(node, window.parent()) }?;
            window.resize(size);
            self.editors[node].as_mut().expect("attached").owned = Some(window);
            Ok(())
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = (node, title);
            Err("owned editor window unavailable on this platform; provide a supported native parent".into())
        }
    }
    pub fn resize_editor(&mut self, node: usize, size: [u32; 2]) -> Result<(), HostError> {
        self.resize_editor_inner(node, size, false)
    }
    fn resize_editor_inner(
        &mut self,
        node: usize,
        mut size: [u32; 2],
        plugin_request: bool,
    ) -> Result<(), HostError> {
        if !valid_size(size) {
            return Err("editor dimensions must be 64..8192".into());
        }
        let e = self
            .editors
            .get_mut(node)
            .and_then(Option::as_mut)
            .ok_or("editor not open")?;
        if e.size == size {
            return Ok(());
        }
        unsafe {
            if !plugin_request && !(*e.extension).can_resize.is_some_and(|f| f(e.plugin)) {
                return Err("editor cannot be resized by host".into());
            }
            if let Some(adjust) = (*e.extension).adjust_size {
                if !adjust(e.plugin, &mut size[0], &mut size[1]) {
                    return Err("editor resize adjustment refused".into());
                }
            }
            if !valid_size(size)
                || !(*e.extension).set_size.ok_or("missing editor set_size")?(
                    e.plugin, size[0], size[1],
                )
            {
                return Err("editor resize refused".into());
            }
        }
        e.size = size;
        unsafe {
            (*e.context)
                .gui_last_size
                .store(((size[0] as u64) << 32) | size[1] as u64, Ordering::Release);
        }
        #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
        if let Some(window) = &mut e.owned {
            window.resize(size);
        }
        self.push_editor_event(EditorEvent::Resized { node, size });
        Ok(())
    }
    pub fn scale_editor(&mut self, node: usize, scale: f64) -> Result<(), HostError> {
        if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return Err("editor scale must be 0.5..4".into());
        }
        let e = self
            .editors
            .get_mut(node)
            .and_then(Option::as_mut)
            .ok_or("editor not open")?;
        unsafe {
            if !(*e.extension)
                .set_scale
                .ok_or("editor scaling unavailable")?(e.plugin, scale)
            {
                return Err("editor refused scale".into());
            }
            e.scale = scale;
            (*e.context)
                .gui_scale
                .store(scale.to_bits(), Ordering::Release);
            let (mut w, mut h) = (0, 0);
            if (*e.extension)
                .get_size
                .is_some_and(|f| f(e.plugin, &mut w, &mut h))
                && valid_size([w, h])
            {
                e.size = [w, h];
                (*e.context)
                    .gui_last_size
                    .store(((w as u64) << 32) | h as u64, Ordering::Release);
                #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
                if let Some(window) = &mut e.owned {
                    window.resize([w, h]);
                }
            }
        }
        Ok(())
    }
    pub fn drain_editor_events(&mut self) -> Vec<EditorEvent> {
        std::mem::replace(&mut self.editor_events, Vec::with_capacity(128))
    }
    pub(crate) fn push_editor_event(&mut self, event: EditorEvent) {
        if self.editor_events.len() < 128 {
            self.editor_events.push(event);
        } else {
            self.shared
                .stats
                .event_drops
                .fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn poll_editors(&mut self) {
        for node in 0..self.instances.len() {
            let value = self.instances[node]
                .context
                .gui_resize
                .swap(0, Ordering::AcqRel);
            if value != 0 {
                if let Err(e) =
                    self.resize_editor_inner(node, [(value >> 32) as u32, value as u32], true)
                {
                    self.push_editor_event(EditorEvent::Error {
                        node,
                        message: e.to_string(),
                    });
                }
            }
            #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
            {
                let events = self.editors[node]
                    .as_mut()
                    .and_then(|e| e.owned.as_mut())
                    .map(Window::events)
                    .unwrap_or_default();
                for event in events {
                    match event {
                        NativeEvent::Close => {
                            self.close_editor(node);
                            self.push_editor_event(EditorEvent::Closed { node });
                        }
                        NativeEvent::Scale(scale) => {
                            if let Err(error) = self.scale_editor(node, scale) {
                                if let Some(editor) = &mut self.editors[node] {
                                    if let Some(window) = &mut editor.owned {
                                        window.resize(editor.size);
                                    }
                                }
                                self.push_editor_event(EditorEvent::Error {
                                    node,
                                    message: error.to_string(),
                                });
                            }
                        }
                        NativeEvent::Resize(size) => {
                            if let Err(e) = self.resize_editor(node, size) {
                                if let Some(e) = &mut self.editors[node] {
                                    if let Some(w) = &mut e.owned {
                                        w.resize(e.size);
                                    }
                                }
                                self.push_editor_event(EditorEvent::Error {
                                    node,
                                    message: e.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    pub fn capture_editor_ppm(&self, node: usize, path: &Path) -> Result<(), HostError> {
        self.editors
            .get(node)
            .and_then(Option::as_ref)
            .and_then(|e| e.owned.as_ref())
            .ok_or("owned embedded editor required")?
            .capture(path)
    }
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    pub fn focus_editor(&self, node: usize) -> Result<(), HostError> {
        let w = self
            .editors
            .get(node)
            .and_then(Option::as_ref)
            .and_then(|e| e.owned.as_ref())
            .ok_or("owned embedded editor required")?;
        w.focus();
        Ok(())
    }
}
fn valid_size(size: [u32; 2]) -> bool {
    size.iter().all(|n| (64..=8192).contains(n))
}
