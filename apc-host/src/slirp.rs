use anyhow::{Context, Result};
use std::os::unix::io::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static NOTIFY_PIPE_R: AtomicI32 = AtomicI32::new(-1);
static NOTIFY_PIPE_W: AtomicI32 = AtomicI32::new(-1);

#[repr(C)]
struct Slirp {
    _opaque: [u8; 0],
}

type SlirpWriteCb =
    unsafe extern "C" fn(buf: *const u8, len: usize, opaque: *mut libc::c_void) -> isize;
type SlirpTimerCb = unsafe extern "C" fn(opaque: *mut libc::c_void);
type SlirpAddPollCb = unsafe extern "C" fn(fd: i32, events: i32, opaque: *mut libc::c_void) -> i32;
type SlirpGetREventsCb = unsafe extern "C" fn(idx: i32, opaque: *mut libc::c_void) -> i32;

#[repr(C)]
struct SlirpCb {
    send_packet: SlirpWriteCb,
    guest_error: unsafe extern "C" fn(msg: *const libc::c_char, opaque: *mut libc::c_void),
    clock_get_ns: unsafe extern "C" fn(opaque: *mut libc::c_void) -> i64,
    timer_new: unsafe extern "C" fn(
        cb: SlirpTimerCb,
        cb_opaque: *mut libc::c_void,
        opaque: *mut libc::c_void,
    ) -> *mut libc::c_void,
    timer_free: unsafe extern "C" fn(timer: *mut libc::c_void, opaque: *mut libc::c_void),
    timer_mod:
        unsafe extern "C" fn(timer: *mut libc::c_void, expire_time: i64, opaque: *mut libc::c_void),
    register_poll_fd: unsafe extern "C" fn(fd: i32, opaque: *mut libc::c_void),
    unregister_poll_fd: unsafe extern "C" fn(fd: i32, opaque: *mut libc::c_void),
    notify: unsafe extern "C" fn(opaque: *mut libc::c_void),
    init_completed: *const libc::c_void,
    timer_new_opaque: *const libc::c_void,
    register_poll_socket: *const libc::c_void,
    unregister_poll_socket: *const libc::c_void,
}

#[repr(C)]
struct SlirpConfig {
    version: u32,
    restricted: i32,
    in_enabled: bool,
    vnetwork: libc::in_addr,
    vnetmask: libc::in_addr,
    vhost: libc::in_addr,
    in6_enabled: bool,
    vprefix_addr6: libc::in6_addr,
    vprefix_len: u8,
    vhost6: libc::in6_addr,
    vhostname: *const libc::c_char,
    tftp_server_name: *const libc::c_char,
    tftp_path: *const libc::c_char,
    bootfile: *const libc::c_char,
    vdhcp_start: libc::in_addr,
    vnameserver: libc::in_addr,
    vnameserver6: libc::in6_addr,
    vdnssearch: *const *const libc::c_char,
    vdomainname: *const libc::c_char,
    if_mtu: usize,
    if_mru: usize,
    disable_host_loopback: bool,
    enable_emu: bool,
    outbound_addr: *const libc::sockaddr_in,
    outbound_addr6: *const libc::sockaddr_in6,
    disable_dns: bool,
    disable_dhcp: bool,
}

unsafe extern "C" {
    fn slirp_new(
        cfg: *const SlirpConfig,
        callbacks: *const SlirpCb,
        opaque: *mut libc::c_void,
    ) -> *mut Slirp;
    fn slirp_cleanup(slirp: *mut Slirp);
    fn slirp_input(slirp: *mut Slirp, pkt: *const u8, pkt_len: i32);
    fn slirp_pollfds_fill(
        slirp: *mut Slirp,
        timeout: *mut u32,
        add_poll: SlirpAddPollCb,
        opaque: *mut libc::c_void,
    );
    fn slirp_pollfds_poll(
        slirp: *mut Slirp,
        select_error: i32,
        get_revents: SlirpGetREventsCb,
        opaque: *mut libc::c_void,
    );
}

const SLIRP_POLL_IN: i32 = 1 << 0;
const SLIRP_POLL_OUT: i32 = 1 << 1;
const SLIRP_POLL_ERR: i32 = 1 << 3;
const SLIRP_POLL_HUP: i32 = 1 << 4;

fn ipv4(a: u8, b: u8, c: u8, d: u8) -> libc::in_addr {
    libc::in_addr {
        s_addr: u32::from_ne_bytes([a, b, c, d]),
    }
}

struct SlirpState {
    vm_fd: RawFd,
    poll_fds: Vec<(i32, i32)>,
    poll_revents: Vec<i32>,
    timers: Vec<Option<Timer>>,
}

struct Timer {
    cb: SlirpTimerCb,
    cb_opaque: *mut libc::c_void,
    expire_ns: i64,
}

unsafe impl Send for Timer {}

unsafe extern "C" fn cb_send_packet(
    buf: *const u8,
    len: usize,
    opaque: *mut libc::c_void,
) -> isize {
    unsafe {
        let state = &*(opaque as *const SlirpState);
        let data = std::slice::from_raw_parts(buf, len);
        libc::send(
            state.vm_fd,
            data.as_ptr() as *const libc::c_void,
            data.len(),
            0,
        )
    }
}

unsafe extern "C" fn cb_guest_error(msg: *const libc::c_char, _opaque: *mut libc::c_void) {
    unsafe {
        let s = std::ffi::CStr::from_ptr(msg);
        tracing::warn!(msg = %s.to_string_lossy(), "slirp guest error");
    }
}

unsafe extern "C" fn cb_clock_get_ns(_opaque: *mut libc::c_void) -> i64 {
    unsafe {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
        ts.tv_sec * 1_000_000_000 + ts.tv_nsec
    }
}

unsafe extern "C" fn cb_timer_new(
    cb: SlirpTimerCb,
    cb_opaque: *mut libc::c_void,
    opaque: *mut libc::c_void,
) -> *mut libc::c_void {
    unsafe {
        let state = &mut *(opaque as *mut SlirpState);
        let idx = state.timers.len();
        state.timers.push(Some(Timer {
            cb,
            cb_opaque,
            expire_ns: i64::MAX,
        }));
        idx as *mut libc::c_void
    }
}

unsafe extern "C" fn cb_timer_free(timer: *mut libc::c_void, opaque: *mut libc::c_void) {
    unsafe {
        let state = &mut *(opaque as *mut SlirpState);
        let idx = timer as usize;
        if idx < state.timers.len() {
            state.timers[idx] = None;
        }
    }
}

unsafe extern "C" fn cb_timer_mod(
    timer: *mut libc::c_void,
    expire_time_ms: i64,
    opaque: *mut libc::c_void,
) {
    // slirp passes milliseconds; convert to nanoseconds for clock_gettime comparison
    unsafe {
        let state = &mut *(opaque as *mut SlirpState);
        let idx = timer as usize;
        if idx < state.timers.len() {
            if let Some(t) = &mut state.timers[idx] {
                t.expire_ns = expire_time_ms * 1_000_000;
            }
        }
    }
}

unsafe extern "C" fn cb_register_poll_fd(fd: i32, _opaque: *mut libc::c_void) {
    tracing::debug!(fd, "slirp register_poll_fd");
}
unsafe extern "C" fn cb_unregister_poll_fd(fd: i32, _opaque: *mut libc::c_void) {
    tracing::debug!(fd, "slirp unregister_poll_fd");
}
unsafe extern "C" fn cb_notify(_opaque: *mut libc::c_void) {
    unsafe {
        let fd = NOTIFY_PIPE_W.load(Ordering::Relaxed);
        if fd >= 0 {
            let b: u8 = 1;
            libc::write(fd, &b as *const u8 as *const libc::c_void, 1);
        }
    }
    tracing::debug!("slirp notify (woke pipe)");
}

unsafe extern "C" fn cb_add_poll(fd: i32, events: i32, opaque: *mut libc::c_void) -> i32 {
    unsafe {
        let state = &mut *(opaque as *mut SlirpState);
        let idx = state.poll_fds.len() as i32;
        state.poll_fds.push((fd, events));
        idx
    }
}

unsafe extern "C" fn cb_get_revents(idx: i32, opaque: *mut libc::c_void) -> i32 {
    unsafe {
        let state = &*(opaque as *const SlirpState);
        if (idx as usize) < state.poll_revents.len() {
            state.poll_revents[idx as usize]
        } else {
            0
        }
    }
}

pub fn create_socketpair() -> Result<(RawFd, RawFd)> {
    let mut fds = [0i32; 2];
    let ret = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_DGRAM, 0, fds.as_mut_ptr()) };
    if ret < 0 {
        return Err(anyhow::anyhow!(
            "socketpair failed: {}",
            std::io::Error::last_os_error()
        ));
    }

    unsafe {
        let sndbuf: libc::c_int = 65550;
        let rcvbuf: libc::c_int = 1024 * 1024;
        for fd in &fds {
            libc::setsockopt(
                *fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &sndbuf as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as u32,
            );
            libc::setsockopt(
                *fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &rcvbuf as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as u32,
            );
        }
    }

    Ok((fds[0], fds[1]))
}

pub fn start(vm_fd: RawFd) -> Result<Arc<AtomicBool>> {
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::Builder::new()
        .name("slirp".into())
        .spawn(move || {
            if let Err(e) = run_slirp(vm_fd, &running_clone) {
                tracing::error!(error = %e, "slirp thread failed");
            }
            running_clone.store(false, Ordering::Relaxed);
        })
        .context("spawn slirp thread")?;

    Ok(running)
}

fn run_slirp(vm_fd: RawFd, running: &AtomicBool) -> Result<()> {
    // Self-pipe for notify callback to wake poll loop
    let notify_r;
    unsafe {
        let mut pipe_fds = [0i32; 2];
        let ret = libc::pipe(pipe_fds.as_mut_ptr());
        if ret < 0 {
            return Err(anyhow::anyhow!(
                "pipe failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        libc::fcntl(pipe_fds[0], libc::F_SETFL, libc::O_NONBLOCK);
        libc::fcntl(pipe_fds[1], libc::F_SETFL, libc::O_NONBLOCK);
        NOTIFY_PIPE_R.store(pipe_fds[0], Ordering::Relaxed);
        NOTIFY_PIPE_W.store(pipe_fds[1], Ordering::Relaxed);
        notify_r = pipe_fds[0];
    }

    let mut state = SlirpState {
        vm_fd,
        poll_fds: Vec::new(),
        poll_revents: Vec::new(),
        timers: Vec::new(),
    };

    let callbacks = SlirpCb {
        send_packet: cb_send_packet,
        guest_error: cb_guest_error,
        clock_get_ns: cb_clock_get_ns,
        timer_new: cb_timer_new,
        timer_free: cb_timer_free,
        timer_mod: cb_timer_mod,
        register_poll_fd: cb_register_poll_fd,
        unregister_poll_fd: cb_unregister_poll_fd,
        notify: cb_notify,
        init_completed: std::ptr::null(),
        timer_new_opaque: std::ptr::null(),
        register_poll_socket: std::ptr::null(),
        unregister_poll_socket: std::ptr::null(),
    };

    // 10.0.2.0/24 network, host=10.0.2.2, dhcp=10.0.2.15, dns=10.0.2.3
    let config = SlirpConfig {
        version: 4,
        restricted: 0,
        in_enabled: true,
        vnetwork: ipv4(10, 0, 2, 0),
        vnetmask: ipv4(255, 255, 255, 0),
        vhost: ipv4(10, 0, 2, 2),
        in6_enabled: false,
        vprefix_addr6: libc::in6_addr { s6_addr: [0; 16] },
        vprefix_len: 0,
        vhost6: libc::in6_addr { s6_addr: [0; 16] },
        vhostname: b"apc\0".as_ptr() as *const libc::c_char,
        tftp_server_name: std::ptr::null(),
        tftp_path: std::ptr::null(),
        bootfile: std::ptr::null(),
        vdhcp_start: ipv4(10, 0, 2, 15),
        vnameserver: ipv4(10, 0, 2, 3),
        vnameserver6: libc::in6_addr { s6_addr: [0; 16] },
        vdnssearch: std::ptr::null(),
        vdomainname: std::ptr::null(),
        if_mtu: 0,
        if_mru: 0,
        disable_host_loopback: false,
        enable_emu: false,
        outbound_addr: std::ptr::null(),
        outbound_addr6: std::ptr::null(),
        disable_dns: false,
        disable_dhcp: false,
    };

    let state_ptr = &mut state as *mut SlirpState as *mut libc::c_void;

    let slirp = unsafe { slirp_new(&config, &callbacks, state_ptr) };
    if slirp.is_null() {
        return Err(anyhow::anyhow!("slirp_new returned null"));
    }

    tracing::info!("slirp userspace network started (10.0.2.0/24, gateway 10.0.2.2)");

    let mut recv_buf = vec![0u8; 65536];

    while running.load(Ordering::Relaxed) {
        // Fill pollfds from slirp
        state.poll_fds.clear();
        let mut timeout_ms: u32 = 100;
        unsafe {
            slirp_pollfds_fill(slirp, &mut timeout_ms, cb_add_poll, state_ptr);
        }

        // Add our vm_fd for reading guest frames
        let vm_poll_idx = state.poll_fds.len();
        state.poll_fds.push((vm_fd, SLIRP_POLL_IN));

        // Add notify pipe to wake on slirp state changes
        let notify_poll_idx = state.poll_fds.len();
        state.poll_fds.push((notify_r, SLIRP_POLL_IN));

        // Build pollfd array
        let mut pollfds: Vec<libc::pollfd> = state
            .poll_fds
            .iter()
            .map(|&(fd, events)| {
                let mut pev: i16 = 0;
                if events & SLIRP_POLL_IN != 0 {
                    pev |= libc::POLLIN;
                }
                if events & SLIRP_POLL_OUT != 0 {
                    pev |= libc::POLLOUT;
                }
                libc::pollfd {
                    fd,
                    events: pev,
                    revents: 0,
                }
            })
            .collect();

        let timeout = timeout_ms.min(100) as i32;
        let ret =
            unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout) };

        // Fire expired timers
        let now_ns = unsafe { cb_clock_get_ns(std::ptr::null_mut()) };
        for timer in &mut state.timers {
            if let Some(t) = timer {
                if t.expire_ns <= now_ns {
                    t.expire_ns = i64::MAX;
                    unsafe { (t.cb)(t.cb_opaque) };
                }
            }
        }

        if ret < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            tracing::error!(error = %err, "poll failed");
            break;
        }

        // Convert poll results back to slirp revents
        state.poll_revents = pollfds
            .iter()
            .map(|pfd| {
                let mut rev = 0;
                if pfd.revents & libc::POLLIN != 0 {
                    rev |= SLIRP_POLL_IN;
                }
                if pfd.revents & libc::POLLOUT != 0 {
                    rev |= SLIRP_POLL_OUT;
                }
                if pfd.revents & libc::POLLERR != 0 {
                    rev |= SLIRP_POLL_ERR;
                }
                if pfd.revents & libc::POLLHUP != 0 {
                    rev |= SLIRP_POLL_HUP;
                }
                rev
            })
            .collect();

        // Let slirp process its fds
        unsafe {
            slirp_pollfds_poll(
                slirp,
                if ret < 0 { 1 } else { 0 },
                cb_get_revents,
                state_ptr,
            );
        }

        // Drain notify pipe if it was signaled
        if notify_poll_idx < pollfds.len() && pollfds[notify_poll_idx].revents & libc::POLLIN != 0 {
            let mut drain = [0u8; 64];
            unsafe {
                while libc::read(
                    notify_r,
                    drain.as_mut_ptr() as *mut libc::c_void,
                    drain.len(),
                ) > 0
                {}
            }
            tracing::debug!("slirp notify pipe drained");
        }

        // Read frames from VM and feed to slirp
        if vm_poll_idx < pollfds.len() && pollfds[vm_poll_idx].revents & libc::POLLIN != 0 {
            loop {
                let n = unsafe {
                    libc::recv(
                        vm_fd,
                        recv_buf.as_mut_ptr() as *mut libc::c_void,
                        recv_buf.len(),
                        libc::MSG_DONTWAIT,
                    )
                };
                if n <= 0 {
                    break;
                }
                unsafe {
                    slirp_input(slirp, recv_buf.as_ptr(), n as i32);
                }
            }
        }
    }

    tracing::info!("slirp shutting down");
    unsafe { slirp_cleanup(slirp) };
    Ok(())
}
