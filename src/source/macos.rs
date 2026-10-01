//! macOS backend.
//!
//! The base record for every process comes from `sysctl(KERN_PROC_ALL)`,
//! which any user may read. Memory, CPU, threads and argv come from
//! `proc_pidinfo` / `KERN_PROCARGS2`, which macOS only allows for your own
//! processes unless you are root (`/bin/ps` is setuid root for this reason).

use std::collections::HashMap;
use std::ffi::{CStr, c_void};
use std::io;
use std::mem::{size_of, zeroed};
use std::time::Duration;

use libc::{c_char, c_int, c_uint, gid_t, pid_t, uid_t};

use super::ProcessSource;
use super::procargs::parse_procargs2;
use crate::process::{Process, RunState, Snapshot, StateFlags};
use crate::users::UserCache;

// ---- kinfo_proc (sys/sysctl.h, sys/proc.h), not provided by the libc crate ----

#[repr(C)]
struct ExternProc {
    p_starttime: libc::timeval, // union with two pointers; same size
    p_vmspace: *mut c_void,
    p_sigacts: *mut c_void,
    p_flag: c_int,
    p_stat: c_char,
    p_pid: pid_t,
    p_oppid: pid_t,
    p_dupfd: c_int,
    user_stack: *mut c_char,
    exit_thread: *mut c_void,
    p_debugger: c_int,
    sigwait: c_int,
    p_estcpu: c_uint,
    p_cpticks: c_int,
    p_pctcpu: u32,
    p_wchan: *mut c_void,
    p_wmesg: *mut c_char,
    p_swtime: c_uint,
    p_slptime: c_uint,
    p_realtimer: [libc::timeval; 2],
    p_rtime: libc::timeval,
    p_uticks: u64,
    p_sticks: u64,
    p_iticks: u64,
    p_traceflag: c_int,
    p_tracep: *mut c_void,
    p_siglist: c_int,
    p_textvp: *mut c_void,
    p_holdcnt: c_int,
    p_sigmask: u32,
    p_sigignore: u32,
    p_sigcatch: u32,
    p_priority: u8,
    p_usrpri: u8,
    p_nice: c_char,
    p_comm: [c_char; 17],
    p_pgrp: *mut c_void,
    p_addr: *mut c_void,
    p_xstat: u16,
    p_acflag: u16,
    p_ru: *mut c_void,
}

#[repr(C)]
struct Pcred {
    pc_lock: [c_char; 72],
    pc_ucred: *mut c_void,
    p_ruid: uid_t,
    p_svuid: uid_t,
    p_rgid: gid_t,
    p_svgid: gid_t,
    p_refcnt: c_int,
}

#[repr(C)]
struct Ucred {
    cr_ref: i32,
    cr_uid: uid_t,
    cr_ngroups: i16,
    cr_groups: [gid_t; 16],
}

#[repr(C)]
struct Vmspace {
    dummy: i32,
    dummy2: *mut c_char,
    dummy3: [i32; 5],
    dummy4: [*mut c_char; 3],
}

#[repr(C)]
struct EProc {
    e_paddr: *mut c_void,
    e_sess: *mut c_void,
    e_pcred: Pcred,
    e_ucred: Ucred,
    e_vm: Vmspace,
    e_ppid: pid_t,
    e_pgid: pid_t,
    e_jobc: i16,
    e_tdev: i32,
    e_tpgid: pid_t,
    e_tsess: *mut c_void,
    e_wmesg: [c_char; 8],
    e_xsize: i32,
    e_xrssize: i16,
    e_xccount: i16,
    e_xswrss: i16,
    e_flag: i32,
    e_login: [c_char; 12],
    e_spare: [i32; 4],
}

#[repr(C)]
struct KinfoProc {
    kp_proc: ExternProc,
    kp_eproc: EProc,
}

// Layout checked against the C headers (clang, arm64 macOS).
const _: () = {
    use std::mem::offset_of;
    assert!(size_of::<KinfoProc>() == 648);
    assert!(offset_of!(ExternProc, p_flag) == 32);
    assert!(offset_of!(ExternProc, p_stat) == 36);
    assert!(offset_of!(ExternProc, p_pid) == 40);
    assert!(offset_of!(ExternProc, p_priority) == 240);
    assert!(offset_of!(ExternProc, p_nice) == 242);
    assert!(offset_of!(ExternProc, p_comm) == 243);
    assert!(offset_of!(KinfoProc, kp_eproc) == 296);
    assert!(296 + offset_of!(EProc, e_pcred) + offset_of!(Pcred, p_ruid) == 392);
    assert!(296 + offset_of!(EProc, e_ucred) + offset_of!(Ucred, cr_uid) == 420);
    assert!(296 + offset_of!(EProc, e_ppid) == 560);
    assert!(296 + offset_of!(EProc, e_pgid) == 564);
    assert!(296 + offset_of!(EProc, e_tdev) == 572);
    assert!(296 + offset_of!(EProc, e_tpgid) == 576);
    assert!(296 + offset_of!(EProc, e_flag) == 612);
};

const P_CONTROLT: c_int = 0x2;
const P_PPWAIT: c_int = 0x10;
const P_TRACED: c_int = 0x800;
const P_WEXIT: c_int = 0x2000;
const EPROC_SLEADER: i32 = 0x2;
const NODEV: i32 = -1;

const PROC_PIDLISTTHREADS: c_int = 6;
const TH_USAGE_SCALE: f64 = 1000.0;
/// A waiting thread asleep longer than this is "idle" (I) rather than "sleeping" (S).
const IDLE_AFTER_SECS: i32 = 20;

#[repr(C)]
struct MachTimebaseInfo {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn devname(dev: libc::dev_t, kind: libc::mode_t) -> *mut c_char;
    fn mach_timebase_info(info: *mut MachTimebaseInfo) -> c_int;
}

pub struct MacSource {
    users: UserCache,
    ttys: HashMap<i32, Option<String>>,
    /// Mach absolute-time ticks → nanoseconds (numer/denom).
    timebase: (u32, u32),
}

impl MacSource {
    pub fn new() -> Self {
        let mut tb = MachTimebaseInfo { numer: 1, denom: 1 };
        // SAFETY: writes into a live local.
        unsafe { mach_timebase_info(&mut tb) };
        MacSource {
            users: UserCache::default(),
            ttys: HashMap::new(),
            timebase: (tb.numer, tb.denom.max(1)),
        }
    }

    fn ticks(&self, t: u64) -> Duration {
        let (n, d) = self.timebase;
        Duration::from_nanos((t as u128 * n as u128 / d as u128) as u64)
    }

    fn tty_name(&mut self, dev: i32) -> Option<String> {
        if dev == NODEV {
            return None;
        }
        self.ttys
            .entry(dev)
            .or_insert_with(|| {
                // SAFETY: devname returns NULL or a pointer to a static buffer.
                let p = unsafe { devname(dev, libc::S_IFCHR) };
                (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
            })
            .clone()
    }

    fn build(&mut self, kp: &KinfoProc) -> Process {
        let p = &kp.kp_proc;
        let e = &kp.kp_eproc;
        let pid = p.p_pid;
        let uid = e.e_ucred.cr_uid;
        let ruid = e.e_pcred.p_ruid;
        let comm = cstr(&p.p_comm);
        let nice = p.p_nice as i32;
        let stat = p.p_stat as u32;

        let mut proc = Process {
            pid,
            ppid: e.e_ppid,
            pgid: e.e_pgid,
            uid,
            ruid,
            user: self.users.name(uid),
            ruser: self.users.name(ruid),
            comm,
            tty: self.tty_name(e.e_tdev),
            nice,
            priority: p.p_priority as i32,
            raw_flags: p.p_flag as u32,
            start: p.p_starttime.tv_sec,
            flags: StateFlags {
                foreground: p.p_flag & P_CONTROLT != 0 && e.e_pgid == e.e_tpgid,
                high_priority: nice < 0,
                low_priority: nice > 0,
                session_leader: e.e_flag & EPROC_SLEADER != 0,
                traced: p.p_flag & P_TRACED != 0,
                exiting: p.p_flag & P_WEXIT != 0,
                vfork_wait: p.p_flag & P_PPWAIT != 0,
            },
            ..Default::default()
        };

        if stat == libc::SZOMB {
            proc.state = RunState::Zombie;
            return proc;
        }

        if let Some(ti) = pidinfo::<libc::proc_taskinfo>(pid, libc::PROC_PIDTASKINFO, 0) {
            proc.rss = Some(ti.pti_resident_size);
            proc.vsz = Some(ti.pti_virtual_size);
            proc.cpu_time = Some(self.ticks(ti.pti_total_user + ti.pti_total_system));
            proc.threads = Some(ti.pti_threadnum.max(0) as u32);
            if let Some((state, pcpu)) = thread_summary(pid, ti.pti_threadnum) {
                proc.state = state;
                proc.pcpu = Some(pcpu);
            }
        }
        if stat == libc::SSTOP {
            proc.state = RunState::Stopped;
        }
        proc.args = procargs(pid);
        proc
    }
}

impl ProcessSource for MacSource {
    fn snapshot(&mut self) -> io::Result<Snapshot> {
        let kps = all_kinfo()?;
        // pid 0 is kernel_task, which ps never lists.
        let processes = kps
            .iter()
            .filter(|kp| kp.kp_proc.p_pid != 0)
            .map(|kp| self.build(kp))
            .collect();
        Ok(Snapshot {
            processes,
            total_memory: total_memory(),
            now: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            // SAFETY: trivial libc getters.
            my_uid: unsafe { libc::getuid() },
            my_pid: std::process::id() as i32,
        })
    }
}

fn cstr(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn all_kinfo() -> io::Result<Vec<KinfoProc>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL, 0];
    for _ in 0..8 {
        let mut size: usize = 0;
        // SAFETY: size query with a NULL buffer.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // Leave room for processes started between the two calls.
        let cap = size / size_of::<KinfoProc>() + 32;
        let mut buf: Vec<KinfoProc> = Vec::with_capacity(cap);
        let mut bytes = cap * size_of::<KinfoProc>();
        // SAFETY: the kernel writes at most `bytes` into the buffer's capacity.
        let rc = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr() as *mut c_void,
                &mut bytes,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::ENOMEM) {
                continue; // grew again; retry with a fresh size
            }
            return Err(err);
        }
        // SAFETY: the kernel filled `bytes` worth of whole records.
        unsafe { buf.set_len(bytes / size_of::<KinfoProc>()) };
        return Ok(buf);
    }
    Err(io::Error::other("process table kept growing during sysctl"))
}

/// `proc_pidinfo` for a fixed-size flavor; `None` on any failure
/// (permission denied, process gone, short read).
fn pidinfo<T>(pid: pid_t, flavor: c_int, arg: u64) -> Option<T> {
    // SAFETY: T is a plain C struct; zeroed is a valid value.
    let mut info: T = unsafe { zeroed() };
    let size = size_of::<T>() as c_int;
    // SAFETY: the buffer is `size` bytes of writable memory.
    let n =
        unsafe { libc::proc_pidinfo(pid, flavor, arg, &mut info as *mut T as *mut c_void, size) };
    (n == size).then_some(info)
}

/// Combine per-thread run states (as Apple's ps does) and sum CPU usage.
fn thread_summary(pid: pid_t, threadnum: i32) -> Option<(RunState, f64)> {
    let mut ids = vec![0u64; threadnum.max(0) as usize + 16];
    let bytes = (ids.len() * size_of::<u64>()) as c_int;
    // SAFETY: `ids` is `bytes` long.
    let n = unsafe {
        libc::proc_pidinfo(
            pid,
            PROC_PIDLISTTHREADS,
            0,
            ids.as_mut_ptr() as *mut c_void,
            bytes,
        )
    };
    if n <= 0 {
        return None;
    }
    ids.truncate(n as usize / size_of::<u64>());

    let mut best: Option<RunState> = None;
    let mut usage = 0.0;
    for id in ids {
        let Some(t) = pidinfo::<libc::proc_threadinfo>(pid, libc::PROC_PIDTHREADINFO, id) else {
            continue;
        };
        usage += t.pth_cpu_usage as f64;
        let s = thread_state(t.pth_run_state, t.pth_sleep_time);
        if best.is_none_or(|b| rank(s) < rank(b)) {
            best = Some(s);
        }
    }
    Some((best?, usage * 100.0 / TH_USAGE_SCALE))
}

fn thread_state(run_state: c_int, sleep_time: c_int) -> RunState {
    match run_state {
        libc::TH_STATE_RUNNING => RunState::Running,
        libc::TH_STATE_UNINTERRUPTIBLE => RunState::Uninterruptible,
        libc::TH_STATE_WAITING if sleep_time > IDLE_AFTER_SECS => RunState::Idle,
        libc::TH_STATE_WAITING => RunState::Sleeping,
        libc::TH_STATE_STOPPED => RunState::Stopped,
        libc::TH_STATE_HALTED => RunState::Halted,
        _ => RunState::Unknown,
    }
}

/// Lower is "more interesting": a process is R if any thread is running.
fn rank(s: RunState) -> u8 {
    match s {
        RunState::Running => 0,
        RunState::Uninterruptible => 1,
        RunState::Sleeping => 2,
        RunState::Idle => 3,
        RunState::Stopped => 4,
        RunState::Halted => 5,
        RunState::Zombie | RunState::Unknown => 6,
    }
}

fn procargs(pid: pid_t) -> Option<Vec<String>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: usize = 0;
    // SAFETY: size query with a NULL buffer.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` is `size` bytes long.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(size);
    parse_procargs2(&buf)
}

fn total_memory() -> u64 {
    let mut mem: u64 = 0;
    let mut size = size_of::<u64>();
    // SAFETY: writes 8 bytes into `mem`.
    let rc = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            &mut mem as *mut u64 as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc == 0 { mem } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sees_itself_with_full_detail() {
        let snap = MacSource::new().snapshot().unwrap();
        let me = snap
            .processes
            .iter()
            .find(|p| p.pid == snap.my_pid)
            .expect("own process in snapshot");
        assert_eq!(me.uid, snap.my_uid);
        assert!(me.rss.unwrap() > 0);
        assert!(me.args.as_ref().is_some_and(|a| !a.is_empty()));
        assert_eq!(me.ppid, std::os::unix::process::parent_id() as i32);
        assert!(snap.total_memory > 0);
    }

    #[test]
    fn sees_launchd_without_root() {
        let snap = MacSource::new().snapshot().unwrap();
        let launchd = snap.processes.iter().find(|p| p.pid == 1).unwrap();
        assert_eq!(launchd.user, "root");
        assert_eq!(launchd.comm, "launchd");
    }

    #[test]
    fn thread_states_rank_like_ps() {
        assert_eq!(thread_state(libc::TH_STATE_WAITING, 30), RunState::Idle);
        assert_eq!(thread_state(libc::TH_STATE_WAITING, 0), RunState::Sleeping);
        assert!(rank(RunState::Running) < rank(RunState::Sleeping));
    }
}
