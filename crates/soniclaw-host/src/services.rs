use super::*;
#[cfg(unix)]
use clap_sys::ext::posix_fd_support::*;
use clap_sys::ext::timer_support::*;
use std::time::{Duration, Instant};
struct Timer {
    id: u32,
    period: Duration,
    next: Instant,
}
#[derive(Default)]
pub(crate) struct Services {
    timers: Vec<Timer>,
    next_id: u32,
    pub(crate) timer_dispatches: u64,
    pub(crate) fd_dispatches: u64,
    #[cfg(unix)]
    fds: Vec<(i32, u32)>,
}
unsafe extern "C" fn register_timer(host: *const clap_host, period: u32, id: *mut u32) -> bool {
    if !unsafe { is_main(host) } || id.is_null() || !(1..=60000).contains(&period) {
        return false;
    }
    let Ok(mut s) = unsafe { context(host) }.services.lock() else {
        return false;
    };
    if s.timers.len() >= 64 || s.next_id == u32::MAX {
        return false;
    }
    let value = s.next_id;
    s.next_id += 1;
    let period = Duration::from_millis(period as u64);
    s.timers.push(Timer {
        id: value,
        period,
        next: Instant::now() + period,
    });
    unsafe {
        *id = value;
    }
    true
}
unsafe extern "C" fn unregister_timer(host: *const clap_host, id: u32) -> bool {
    if !unsafe { is_main(host) } {
        return false;
    }
    let Ok(mut s) = unsafe { context(host) }.services.lock() else {
        return false;
    };
    let old = s.timers.len();
    s.timers.retain(|t| t.id != id);
    s.timers.len() != old
}
static TIMERS: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(register_timer),
    unregister_timer: Some(unregister_timer),
};
#[cfg(unix)]
unsafe extern "C" fn register_fd(host: *const clap_host, fd: i32, flags: u32) -> bool {
    if !unsafe { is_main(host) } || fd < 0 || flags == 0 || flags & !7 != 0 {
        return false;
    }
    let Ok(mut s) = unsafe { context(host) }.services.lock() else {
        return false;
    };
    if s.fds.len() >= 64 || s.fds.iter().any(|(f, _)| *f == fd) {
        return false;
    }
    s.fds.push((fd, flags));
    true
}
#[cfg(unix)]
unsafe extern "C" fn modify_fd(host: *const clap_host, fd: i32, flags: u32) -> bool {
    if !unsafe { is_main(host) } || flags == 0 || flags & !7 != 0 {
        return false;
    }
    let Ok(mut s) = unsafe { context(host) }.services.lock() else {
        return false;
    };
    if let Some((_, f)) = s.fds.iter_mut().find(|(f, _)| *f == fd) {
        *f = flags;
        true
    } else {
        false
    }
}
#[cfg(unix)]
unsafe extern "C" fn unregister_fd(host: *const clap_host, fd: i32) -> bool {
    if !unsafe { is_main(host) } {
        return false;
    }
    let Ok(mut s) = unsafe { context(host) }.services.lock() else {
        return false;
    };
    let old = s.fds.len();
    s.fds.retain(|(f, _)| *f != fd);
    s.fds.len() != old
}
#[cfg(unix)]
static FDS: clap_host_posix_fd_support = clap_host_posix_fd_support {
    register_fd: Some(register_fd),
    modify_fd: Some(modify_fd),
    unregister_fd: Some(unregister_fd),
};
pub(crate) fn extension(id: &CStr) -> Option<*const c_void> {
    if id == CLAP_EXT_TIMER_SUPPORT {
        return Some((&TIMERS as *const clap_host_timer_support).cast());
    }
    #[cfg(unix)]
    if id == CLAP_EXT_POSIX_FD_SUPPORT {
        return Some((&FDS as *const clap_host_posix_fd_support).cast());
    }
    None
}
pub(crate) fn pump(instance: &ClapInstance) {
    pump_parts(instance.plugin, &instance.context);
}
pub(crate) fn pump_parts(plugin: *const clap_plugin, context: &HostContext) {
    let now = Instant::now();
    let timers = if let Ok(mut s) = context.services.lock() {
        s.timers
            .iter_mut()
            .filter_map(|t| {
                if t.next <= now {
                    t.next = now + t.period;
                    Some(t.id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    } else {
        vec![]
    };
    unsafe {
        let ext =
            (*plugin).get_extension.expect("checked")(plugin, CLAP_EXT_TIMER_SUPPORT.as_ptr())
                .cast::<clap_plugin_timer_support>();
        if !ext.is_null() {
            if let Some(callback) = (*ext).on_timer {
                for id in timers {
                    let exists = context
                        .services
                        .lock()
                        .is_ok_and(|s| s.timers.iter().any(|t| t.id == id));
                    if exists {
                        if let Ok(mut s) = context.services.lock() {
                            s.timer_dispatches += 1;
                        }
                        callback(plugin, id);
                    }
                }
            }
        }
    }
    #[cfg(unix)]
    {
        let registrations = context
            .services
            .lock()
            .map(|s| s.fds.clone())
            .unwrap_or_default();
        let mut fds: Vec<_> = registrations
            .iter()
            .map(|(fd, flags)| libc::pollfd {
                fd: *fd,
                events: if flags & CLAP_POSIX_FD_READ != 0 {
                    libc::POLLIN
                } else {
                    0
                } | if flags & CLAP_POSIX_FD_WRITE != 0 {
                    libc::POLLOUT
                } else {
                    0
                },
                revents: 0,
            })
            .collect();
        if !fds.is_empty()
            && unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) } > 0
        {
            unsafe {
                let ext = (*plugin).get_extension.expect("checked")(
                    plugin,
                    CLAP_EXT_POSIX_FD_SUPPORT.as_ptr(),
                )
                .cast::<clap_plugin_posix_fd_support>();
                if !ext.is_null() {
                    if let Some(callback) = (*ext).on_fd {
                        for fd in fds {
                            let mut flags = 0;
                            if fd.revents & libc::POLLIN != 0 {
                                flags |= CLAP_POSIX_FD_READ;
                            }
                            if fd.revents & libc::POLLOUT != 0 {
                                flags |= CLAP_POSIX_FD_WRITE;
                            }
                            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                                flags |= CLAP_POSIX_FD_ERROR;
                            }
                            let exists = context
                                .services
                                .lock()
                                .is_ok_and(|s| s.fds.iter().any(|(f, _)| *f == fd.fd));
                            if flags != 0 && exists {
                                if let Ok(mut s) = context.services.lock() {
                                    s.fd_dispatches += 1;
                                }
                                callback(plugin, fd.fd, flags);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ServiceStats {
    pub timers: usize,
    pub fds: usize,
    pub timer_dispatches: u64,
    pub fd_dispatches: u64,
}
impl crate::live::LiveRack {
    pub fn service_stats(&self, node: usize) -> Result<ServiceStats, HostError> {
        let instance = self.instances.get(node).ok_or("unknown service node")?;
        let s = instance
            .context
            .services
            .lock()
            .map_err(|_| "service lock poisoned")?;
        #[cfg(unix)]
        let fds = s.fds.len();
        #[cfg(not(unix))]
        let fds = 0;
        Ok(ServiceStats {
            timers: s.timers.len(),
            fds,
            timer_dispatches: s.timer_dispatches,
            fd_dispatches: s.fd_dispatches,
        })
    }
}
