//! A host-parented egui editor. All windows are owned by the host's main thread;
//! parameter edits cross a bounded SPSC queue. The audio consumer never locks.
use baseview::dpi::LogicalSize;
pub use baseview::{dpi, host, HandlerError, ParentWindowHandle, WindowSize};
use egui_baseview::{EguiWindow, EguiWindowSettings};
pub use raw_window_handle;
use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
use soniclaw_core::Parameter;
use std::sync::{
    atomic::{AtomicU32, AtomicU64, Ordering},
    Arc, Mutex,
};
#[derive(Clone, Copy, Debug)]
pub struct Edit {
    pub index: usize,
    pub value: f32,
    pub sequence: u64,
}
#[derive(Clone)]
pub struct EditSender {
    producer: Arc<Mutex<rtrb::Producer<Edit>>>,
    next: Arc<AtomicU64>,
    ack: Arc<[AtomicU64]>,
}
pub struct EditReceiver {
    consumer: rtrb::Consumer<Edit>,
    ack: Arc<[AtomicU64]>,
}
pub fn edit_channel(capacity: usize) -> (EditSender, EditReceiver) {
    let (p, c) = rtrb::RingBuffer::new(capacity.max(1));
    let ack: Arc<[AtomicU64]> = (0..256).map(|_| AtomicU64::new(0)).collect();
    (
        EditSender {
            producer: Arc::new(Mutex::new(p)),
            next: Arc::new(AtomicU64::new(1)),
            ack: ack.clone(),
        },
        EditReceiver { consumer: c, ack },
    )
}
impl EditSender {
    /// None means invalid input/backpressure; the UI retains its previous value.
    pub fn submit(&self, index: usize, value: f32) -> Option<u64> {
        if index >= self.ack.len() || !value.is_finite() {
            return None;
        }
        let mut producer = self.producer.lock().ok()?;
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        producer
            .push(Edit {
                index,
                value,
                sequence,
            })
            .ok()?;
        Some(sequence)
    }
    pub fn acknowledged(&self, index: usize) -> u64 {
        self.ack.get(index).map_or(0, |a| a.load(Ordering::Acquire))
    }
}
impl EditReceiver {
    pub fn pop(&mut self) -> Result<Edit, rtrb::PopError> {
        self.consumer.pop()
    }
    /// Call after the command has actually reached the DSP parameter.
    pub fn acknowledge(&self, edit: Edit) {
        if let Some(ack) = self.ack.get(edit.index) {
            ack.fetch_max(edit.sequence, Ordering::Release);
        }
    }
}
struct ControlState {
    value: f32,
    pending: u64,
}
struct Panel {
    trace: bool,
    traced_frame: bool,
    scale: Arc<AtomicU32>,
    title: String,
    parameters: Vec<Arc<Parameter>>,
    controls: Vec<ControlState>,
    sender: EditSender,
    notify: Arc<dyn Fn() + Send + Sync>,
}
impl egui_baseview::App for Panel {
    fn build(
        &mut self,
        ctx: egui::Context,
        _: &mut egui_baseview::Frame,
    ) -> Result<(), baseview::HandlerError> {
        if self.trace {
            eprintln!("Soniclaw editor: graphics/context initialized");
        }
        soniclaw_ui::apply_theme(&ctx);
        Ok(())
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut egui_baseview::Frame) {
        if self.trace && !self.traced_frame {
            eprintln!(
                "Soniclaw editor: first UI frame, available {:?}",
                ui.available_size()
            );
            self.traced_frame = true;
        }
        let requested = f32::from_bits(self.scale.load(Ordering::Relaxed));
        let native = frame.baseview_window().size().scale_factor as f32;
        let zoom = (requested / native.max(0.1)).clamp(0.25, 8.0);
        if (ui.ctx().zoom_factor() - zoom).abs() > 0.001 {
            ui.ctx().set_zoom_factor(zoom);
        }
        let focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
        frame.set_key_capture(if focused && ui.ctx().egui_wants_keyboard_input() {
            egui_baseview::KeyCapture::CaptureAll
        } else {
            egui_baseview::KeyCapture::IgnoreAll
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(12.0);
            ui.heading(&self.title);
            ui.label("SONICLAW  /  AUDIO TOOLKIT");
            ui.add_space(12.0);
            ui.separator();
            ui.add_space(16.0);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (index, p) in self.parameters.iter().enumerate() {
                        // Detached UI parameter avoids mutating the audio atomics before a command
                        // has been accepted. Host changes are read back on every repaint.
                        let state = &mut self.controls[index];
                        if state.pending == 0 || self.sender.acknowledged(index) >= state.pending {
                            state.value = p.get();
                            state.pending = 0;
                        }
                        let info = p.info();
                        let local = p.detached().expect("validated parameter");
                        let _ = local.set(state.value);
                        let logarithmic = info.min > 0.0 && info.max / info.min >= 100.0;
                        let response = if local.step_count() > 0 {
                            soniclaw_ui::parameter_control(ui, &local)
                        } else if local.has_codec() {
                            soniclaw_ui::audio_knob(ui, &local, Default::default())
                        } else {
                            soniclaw_ui::parameter_knob(ui, &local, logarithmic)
                        };
                        if response.changed() {
                            if let Some(sequence) = self.sender.submit(index, local.get()) {
                                state.value = local.get();
                                state.pending = sequence;
                                (self.notify)();
                            }
                        }
                        ui.add_space(20.0);
                    }
                });
            });
            ui.add_space(20.0);
            ui.separator();
            ui.small("Drag a knob · double-click to reset · arrow keys to adjust");
        });
    }
}
struct Parent(RawWindowHandle);
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: constructors/reparenting require the caller to keep the handle live.
        Ok(unsafe { WindowHandle::borrow_raw(self.0) })
    }
}
pub struct Editor {
    window: baseview::Window,
    scale: Arc<AtomicU32>,
}
impl Editor {
    /// # Safety
    /// Parent must use the current platform API and remain live until the editor
    /// is destroyed/reparented. Call only on the host main thread.
    pub unsafe fn create_parented(
        title: String,
        parameters: Vec<Arc<Parameter>>,
        sender: EditSender,
        notify: Arc<dyn Fn() + Send + Sync>,
        host: host::Host,
        size: (u32, u32),
        raw_parent: RawWindowHandle,
    ) -> Result<Self, String> {
        // SAFETY: the caller supplies the same valid parent lifetime.
        unsafe {
            Self::create_parented_scaled(
                title, parameters, sender, notify, host, size, raw_parent, 1.0,
            )
        }
    }
    /// Construct with the retained zoom before the first frame, avoiding a second resize on reopen.
    /// # Safety
    /// Same parent/thread requirements as `create_parented`.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn create_parented_scaled(
        title: String,
        parameters: Vec<Arc<Parameter>>,
        sender: EditSender,
        notify: Arc<dyn Fn() + Send + Sync>,
        host: host::Host,
        size: (u32, u32),
        raw_parent: RawWindowHandle,
        initial_scale: f64,
    ) -> Result<Self, String> {
        if !initial_scale.is_finite() || !(0.5..=4.0).contains(&initial_scale) {
            return Err("invalid initial editor scale".into());
        }
        let parent = Parent(raw_parent);
        // Opaque 2D UI needs neither depth/stencil nor an sRGB-capable visual.
        // Relaxing these requirements also supports software X11 displays.
        let graphics = egui_baseview::GraphicsConfig {
            gl_config: baseview::gl::GlConfig {
                alpha_bits: 0,
                depth_bits: 0,
                stencil_bits: 0,
                srgb: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let settings = EguiWindowSettings::new()
            .with_graphics_config(graphics)
            .with_zoom_factor(initial_scale as f32)
            .with_title(title.clone())
            .with_size(LogicalSize::new(size.0 as f64, size.1 as f64))
            .with_min_size(Some(LogicalSize::new(320.0, 220.0)))
            .with_parent(Some(&parent))
            .with_resizable(true);
        let scale = Arc::new(AtomicU32::new((initial_scale as f32).to_bits()));
        EguiWindow::create_with_host(
            settings,
            Panel {
                trace: std::env::var_os("SONICLAW_EDITOR_TRACE").is_some(),
                traced_frame: false,
                scale: scale.clone(),
                title,
                controls: parameters
                    .iter()
                    .map(|p| ControlState {
                        value: p.get(),
                        pending: 0,
                    })
                    .collect(),
                parameters,
                sender,
                notify,
            },
            Some(host),
        )
        .map(|window| Self { window, scale })
        .map_err(|e| e.to_string())
    }
    /// # Safety
    /// The native parent must remain valid until this editor is destroyed or
    /// reparented. This must run on the host's main thread with the correct API.
    pub unsafe fn set_parent(&self, raw: RawWindowHandle) -> bool {
        self.window.set_parent(&Parent(raw)).is_ok()
    }
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }
    pub fn show(&self) -> bool {
        self.window.show().is_ok()
    }
    pub fn hide(&self) -> bool {
        self.window.hide().is_ok()
    }
    pub fn size(&self) -> (u32, u32) {
        let s = self.window.size().physical;
        (s.width, s.height)
    }
    pub fn resize(&self, width: u32, height: u32) -> bool {
        self.window
            .resize(dpi::PhysicalSize::new(width, height))
            .is_ok()
    }
    pub fn set_scale(&self, scale: f64) -> bool {
        if !scale.is_finite() || !(0.5..=4.0).contains(&scale) {
            return false;
        }
        self.scale
            .store((scale as f32).to_bits(), Ordering::Relaxed);
        true
    }
    pub fn on_main_thread(&self) {
        self.window.host_main_thread_callback();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_queue_and_acknowledgement_preserve_order() {
        let (s, mut c) = edit_channel(2);
        let a = s.submit(0, 0.5).unwrap();
        let b = s.submit(1, 0.75).unwrap();
        assert!(s.submit(0, 1.0).is_none());
        assert_eq!(s.acknowledged(0), 0);
        let first = c.pop().unwrap();
        assert_eq!(first.value, 0.5);
        c.acknowledge(first);
        assert_eq!(s.acknowledged(0), a);
        let second = c.pop().unwrap();
        assert_eq!(second.index, 1);
        c.acknowledge(second);
        assert_eq!(s.acknowledged(1), b);
        assert!(s.submit(0, 1.0).is_some());
        assert!(s.submit(0, f32::NAN).is_none());
        assert!(s.submit(256, 0.0).is_none());
    }
}
