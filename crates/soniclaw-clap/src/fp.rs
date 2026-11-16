//! Scoped floating-point control at the native audio ABI boundary. Only FTZ/DAZ
//! (x86-64) or FZ (AArch64) change; the host's original control word is restored.
#[cfg(target_arch = "x86_64")]
type State = u32;
#[cfg(target_arch = "aarch64")]
type State = u64;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
type State = u32;
#[cfg(target_arch = "x86_64")]
const FLUSH: State = 0x8040;
#[cfg(target_arch = "aarch64")]
const FLUSH: State = 1 << 24;
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const FLUSH: State = 0;
fn read() -> State {
    #[cfg(target_arch = "x86_64")]
    {
        let mut state = 0;
        // SAFETY: x86-64 guarantees SSE2. `state` is a live aligned u32, and
        // STMXCSR only writes that four-byte control/status word.
        unsafe {
            core::arch::asm!("stmxcsr [{p}]",p=in(reg)&mut state,options(nostack,preserves_flags));
        }
        state
    }
    #[cfg(target_arch = "aarch64")]
    {
        let state;
        // SAFETY: reading FPCR is a user-mode AArch64 operation.
        unsafe {
            core::arch::asm!("mrs {value}, fpcr",value=out(reg)state,options(nostack,nomem,preserves_flags));
        }
        state
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        0
    }
}
fn write(state: State) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: the value is the original MXCSR with only supported FTZ/DAZ bits
    // added, or that same original value during restoration. No reserved bit changes.
    unsafe {
        core::arch::asm!("ldmxcsr [{p}]",p=in(reg)&state,options(nostack,preserves_flags,readonly));
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: only the defined FZ bit changes in a saved user-mode FPCR value.
    unsafe {
        core::arch::asm!("msr fpcr, {value}",value=in(reg)state,options(nostack,nomem,preserves_flags));
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = state;
}
pub(super) struct Guard {
    saved: State,
    changed: bool,
}
impl Guard {
    pub(super) fn new() -> Self {
        let saved = read();
        let changed = saved & FLUSH != FLUSH;
        if changed {
            write(saved | FLUSH);
        }
        Self { saved, changed }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if self.changed {
            write(self.saved);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audio_scope_enables_flush_and_restores_host_control() {
        let before = read();
        {
            let _guard = Guard::new();
            assert_eq!(read() & FLUSH, FLUSH);
        }
        assert_eq!(read(), before);
    }
}
