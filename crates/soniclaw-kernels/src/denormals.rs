//! Thread-local floating-point environment for a synchronous audio callback.
use std::{marker::PhantomData, rc::Rc};

/// Restores the calling thread's original flush-mode bit when dropped.
/// Prefer the closure API to enforce nesting and callback-scoped lifetime.
#[must_use]
pub struct DenormalGuard {
    previous: Option<u64>,
    thread: PhantomData<Rc<()>>,
}

impl DenormalGuard {
    pub fn enable() -> Self {
        Self {
            previous: platform::get().map(|control| {
                platform::set(control | platform::FLUSH);
                control & platform::FLUSH
            }),
            thread: PhantomData,
        }
    }

    pub fn active(&self) -> bool {
        self.previous.is_some()
    }
}

impl Drop for DenormalGuard {
    fn drop(&mut self) {
        if let Some((previous, current)) = self.previous.zip(platform::get()) {
            platform::set((current & !platform::FLUSH) | previous);
        }
    }
}

pub fn with_denormals_flushed<R>(process: impl FnOnce() -> R) -> R {
    let _guard = DenormalGuard::enable();
    process()
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod platform {
    pub const FLUSH: u64 = 1 << 15;

    pub fn get() -> Option<u64> {
        if !std::is_x86_feature_detected!("sse") {
            return None;
        }
        let mut value = 0u32;
        // SAFETY: SSE is present. STMXCSR writes only the provided four-byte address.
        unsafe {
            std::arch::asm!("stmxcsr [{p}]", p = in(reg) &mut value, options(nostack, preserves_flags));
        }
        Some(u64::from(value))
    }

    pub fn set(value: u64) {
        let value = value as u32;
        // SAFETY: call sites use a value read from MXCSR and modify only the standard FTZ bit.
        unsafe {
            std::arch::asm!("ldmxcsr [{p}]", p = in(reg) &value, options(nostack, preserves_flags));
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod platform {
    pub const FLUSH: u64 = 1 << 24;

    #[allow(clippy::unnecessary_wraps)]
    pub fn get() -> Option<u64> {
        let value: u64;
        // SAFETY: user mode can read the calling thread's FPCR on AArch64.
        unsafe {
            std::arch::asm!("mrs {v}, fpcr", v = out(reg) value, options(nostack, preserves_flags));
        }
        Some(value)
    }

    pub fn set(value: u64) {
        // SAFETY: preserves unrelated FPCR bits. ISB makes the mode change visible to subsequent FP instructions.
        unsafe {
            std::arch::asm!("msr fpcr, {v}", "isb", v = in(reg) value, options(nostack, preserves_flags));
        }
    }
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
mod platform {
    pub const FLUSH: u64 = 0;
    pub fn get() -> Option<u64> {
        None
    }
    pub fn set(_: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_thread_mode_when_nested_or_unwinding() {
        let prior = platform::get().map(|bits| bits & platform::FLUSH);
        with_denormals_flushed(|| {
            assert_eq!(
                platform::get().map(|bits| bits & platform::FLUSH),
                prior.map(|_| platform::FLUSH)
            );
            let result =
                std::panic::catch_unwind(|| with_denormals_flushed(|| panic!("test unwinding")));
            assert!(result.is_err());
            assert_eq!(
                platform::get().map(|bits| bits & platform::FLUSH),
                prior.map(|_| platform::FLUSH)
            );
        });
        assert_eq!(platform::get().map(|bits| bits & platform::FLUSH), prior);
    }
}
