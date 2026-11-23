//! Owned Win32 parent. WndProc only records close/resize/DPI; plug-in calls run after dispatch.
use super::*;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};
use winapi::{
    shared::{
        minwindef::{LPARAM, LRESULT, UINT, WPARAM},
        windef::{HWND, RECT},
    },
    um::{libloaderapi::GetModuleHandleW, winuser::*},
};
#[derive(Default)]
struct Pending {
    close: bool,
    resize: Option<[u32; 2]>,
    scale: Option<f64>,
}
fn pending() -> &'static Mutex<HashMap<usize, Pending>> {
    static MAP: OnceLock<Mutex<HashMap<usize, Pending>>> = OnceLock::new();
    MAP.get_or_init(Mutex::default)
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_CLOSE {
            if let Ok(mut map) = pending().lock() {
                if let Some(p) = map.get_mut(&(hwnd as usize)) {
                    p.close = true;
                }
            }
            return 0;
        }
        if msg == WM_DPICHANGED {
            if let Some(rect) = (l as *const RECT).as_ref() {
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            let dpi = w & 0xffff;
            if dpi > 0 {
                if let Ok(mut map) = pending().lock() {
                    if let Some(p) = map.get_mut(&(hwnd as usize)) {
                        p.scale = Some(dpi as f64 / 96.);
                    }
                }
            }
            return 0;
        }
        if msg == WM_SIZE && w != SIZE_MINIMIZED {
            let size = [
                (l as usize & 0xffff) as u32,
                ((l as usize >> 16) & 0xffff) as u32,
            ];
            if valid_size(size) {
                if let Ok(mut map) = pending().lock() {
                    if let Some(p) = map.get_mut(&(hwnd as usize)) {
                        p.resize = Some(size);
                    }
                }
            }
        }
        DefWindowProcW(hwnd, msg, w, l)
    }
}
pub(crate) struct Window {
    hwnd: HWND,
    size: [u32; 2],
}
impl Window {
    pub(crate) fn new(title: &str) -> Result<Self, HostError> {
        if title.contains('\0') || title.len() > 1024 {
            return Err("invalid editor window title".into());
        }
        let class: Vec<u16> = "SoniclawClapEditorParent\0".encode_utf16().collect();
        let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        unsafe {
            static REGISTERED: OnceLock<bool> = OnceLock::new();
            let registered = *REGISTERED.get_or_init(|| {
                let mut c: WNDCLASSW = std::mem::zeroed();
                c.lpfnWndProc = Some(procedure);
                c.hInstance = GetModuleHandleW(ptr::null());
                c.lpszClassName = class.as_ptr();
                c.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
                RegisterClassW(&c) != 0
            });
            if !registered {
                return Err("could not register native editor class".into());
            }
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                640,
                480,
                ptr::null_mut(),
                ptr::null_mut(),
                GetModuleHandleW(ptr::null()),
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err("could not create native editor parent".into());
            }
            let dpi = GetDpiForWindow(hwnd);
            let mut map = pending().lock().map_err(|_| "editor event map poisoned")?;
            map.insert(
                hwnd as usize,
                Pending {
                    scale: (dpi > 0).then_some(dpi as f64 / 96.),
                    ..Default::default()
                },
            );
            drop(map);
            ShowWindow(hwnd, SW_SHOW);
            Ok(Self { hwnd, size: [0, 0] })
        }
    }
    pub(crate) fn parent(&self) -> ParentWindow {
        ParentWindow::Win32(self.hwnd.cast())
    }
    pub(crate) fn resize(&mut self, size: [u32; 2]) {
        if self.size == size {
            return;
        }
        self.size = size;
        unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: size[0] as i32,
                bottom: size[1] as i32,
            };
            AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, 0, 0);
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                0,
                0,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    pub(crate) fn focus(&self) {
        unsafe {
            SetForegroundWindow(self.hwnd);
            let child = GetWindow(self.hwnd, GW_CHILD);
            SetFocus(if child.is_null() { self.hwnd } else { child });
        }
    }
    pub(crate) fn events(&mut self) -> Vec<NativeEvent> {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            for _ in 0..128 {
                if PeekMessageW(&mut msg, self.hwnd, 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        let mut events = Vec::new();
        if let Ok(mut map) = pending().lock() {
            if let Some(p) = map.get_mut(&(self.hwnd as usize)) {
                if p.close {
                    p.close = false;
                    events.push(NativeEvent::Close);
                }
                if let Some(scale) = p.scale.take() {
                    events.push(NativeEvent::Scale(scale));
                    // The scale negotiation will select a new client size; discard stale WM_SIZE.
                    p.resize = None;
                }
                if let Some(size) = p.resize.take() {
                    if self.size != size {
                        self.size = size;
                        events.push(NativeEvent::Resize(size));
                    }
                }
            }
        }
        events
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
        if let Ok(mut map) = pending().lock() {
            map.remove(&(self.hwnd as usize));
        }
    }
}
