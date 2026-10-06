//! The process scheduler (M12): processes, channels and who waits on what,
//! as a state machine, the Rust twin of `web/proc.js`. It makes no wasm
//! calls; the host resumes the processes it names.
//!
//! Channel values never reach the scheduler: a sender pushes its value onto
//! the channel's queue in wasm, and the scheduler only records which process
//! is waiting for that value to be taken (`queued`), so a send is a
//! rendezvous.
//!
//! Timers: `OP_SLEEP` parks a process until a deadline on the scheduler's
//! millisecond clock (injectable, so tests are deterministic). When nothing
//! is ready, the earliest due sleeper wakes; a sleeper is never blocked.

use std::collections::{BTreeMap, HashMap, VecDeque};

use wack_core::layout as L;

/// `/prog` handles are numbered from here, apart from the namespace's.
pub const PROG_HANDLE_BASE: i32 = 0x40000000;

pub type Pid = u32;

/// What a submission does: completes at once, or parks the process.
#[derive(Debug, PartialEq, Eq)]
pub enum Submit {
    Done(i32),
    Park,
}

/// A ready process and the result it resumes with (`start` for a process
/// not yet started).
#[derive(Debug, PartialEq, Eq)]
pub struct Resume {
    pub pid: Pid,
    pub result: i32,
    pub start: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Ready,
    Running,
    Parked,
    Io,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaitKind {
    Recv,
    Send,
    Alt,
    Sleep,
}

struct Wait {
    kind: WaitKind,
    chans: Vec<i32>,
}

struct Proc {
    state: State,
    wait: Option<Wait>,
    slot: Option<u32>,
}

#[derive(Clone, Copy)]
struct Receiver {
    pid: Pid,
    arm: Option<i32>,
}

impl Receiver {
    /// The result a woken receiver resumes with: 1 or 0 for `chan.recv`,
    /// `2 * arm + bit` for an `alt` arm.
    fn bit(self, v: i32) -> i32 {
        match self.arm {
            None => v,
            Some(arm) => 2 * arm + v,
        }
    }
}

struct Chan {
    senders: i32,
    closed: bool,
    queued: VecDeque<Pid>,
    receivers: VecDeque<Receiver>,
}

enum ProgHandle {
    Dir(VecDeque<Vec<u8>>),
    Ctl(Pid),
}

pub struct Scheduler {
    procs: BTreeMap<Pid, Proc>,
    next_pid: Pid,
    ready: VecDeque<Resume>,
    chans: BTreeMap<i32, Chan>,
    next_chan: i32,
    prog_handles: HashMap<i32, ProgHandle>,
    next_prog_handle: i32,
    /// Milliseconds on the scheduler's clock.
    now: Box<dyn Fn() -> i64>,
    /// (deadline, pid) of each sleeping process, in the order they slept.
    sleepers: Vec<(i64, Pid)>,
    /// Each process killed, in order, so the host can drop it.
    pub killed: Vec<Pid>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler {
    pub fn new() -> Self {
        let start = std::time::Instant::now();
        Self::with_clock(Box::new(move || start.elapsed().as_millis() as i64))
    }

    /// A scheduler on the given millisecond clock.
    pub fn with_clock(now: Box<dyn Fn() -> i64>) -> Self {
        let mut procs = BTreeMap::new();
        procs.insert(
            0,
            Proc {
                state: State::Running,
                wait: None,
                slot: None,
            },
        );
        Scheduler {
            procs,
            next_pid: 1,
            ready: VecDeque::new(),
            chans: BTreeMap::new(),
            next_chan: 1,
            prog_handles: HashMap::new(),
            next_prog_handle: PROG_HANDLE_BASE,
            now,
            sleepers: Vec::new(),
            killed: Vec::new(),
        }
    }

    /// Pid 0 runs again: the next test or line of a step.
    pub fn start_main(&mut self) {
        self.procs.insert(
            0,
            Proc {
                state: State::Running,
                wait: None,
                slot: None,
            },
        );
    }

    /// Make process N for the closure in `slot`; it runs when the host next
    /// takes a ready process.
    pub fn spawn(&mut self, slot: u32) -> Pid {
        let pid = self.next_pid;
        self.next_pid += 1;
        self.procs.insert(
            pid,
            Proc {
                state: State::Ready,
                wait: None,
                slot: Some(slot),
            },
        );
        self.ready.push_back(Resume {
            pid,
            result: 0,
            start: true,
        });
        pid
    }

    /// The slot `spawn` was given for `pid`.
    pub fn slot(&self, pid: Pid) -> Option<u32> {
        self.procs.get(&pid).and_then(|p| p.slot)
    }

    fn chan_make(&mut self) -> i32 {
        let id = self.next_chan;
        self.next_chan += 1;
        self.chans.insert(
            id,
            Chan {
                senders: 1,
                closed: false,
                queued: VecDeque::new(),
                receivers: VecDeque::new(),
            },
        );
        id
    }

    /// Make `pid` ready to resume with `result`, unless it was killed.
    fn wake(&mut self, pid: Pid, result: i32) {
        let Some(p) = self.procs.get_mut(&pid) else {
            return;
        };
        if p.state == State::Done {
            return;
        }
        p.state = State::Ready;
        if let Some(w) = p.wait.take() {
            for id in w.chans {
                if let Some(c) = self.chans.get_mut(&id) {
                    c.receivers.retain(|r| r.pid != pid);
                }
            }
        }
        self.ready.push_back(Resume {
            pid,
            result,
            start: false,
        });
    }

    fn park(&mut self, pid: Pid, kind: WaitKind, chans: Vec<i32>) -> Submit {
        let p = self.procs.get_mut(&pid).expect("a parking process exists");
        p.state = State::Parked;
        p.wait = Some(Wait { kind, chans });
        Submit::Park
    }

    pub fn submit(&mut self, pid: Pid, op: i32, a0: i32, a1: i32, _a2: i32, mem: &[u8]) -> Submit {
        if op == L::OP_CHAN_MAKE {
            return Submit::Done(self.chan_make());
        }
        if op == L::OP_ALT {
            return self.alt(pid, a0, a1, mem);
        }
        if op == L::OP_SLEEP {
            let deadline = (self.now)() + a0.max(0) as i64;
            self.sleepers.push((deadline, pid));
            return self.park(pid, WaitKind::Sleep, vec![]);
        }
        let Some(c) = self.chans.get_mut(&a0) else {
            return Submit::Done(L::E_BAD_HANDLE);
        };
        match op {
            L::OP_CHAN_SENDER => {
                c.senders += 1;
                Submit::Done(0)
            }
            L::OP_CHAN_SEND => {
                if c.closed {
                    return Submit::Done(L::E_CLOSED);
                }
                c.queued.push_back(pid);
                let Some(r) = c.receivers.pop_front() else {
                    return self.park(pid, WaitKind::Send, vec![a0]);
                };
                // The receiver takes this value: no sender was queued before it.
                c.queued.pop_front();
                self.wake(r.pid, r.bit(1));
                Submit::Done(0)
            }
            L::OP_CHAN_RECV => {
                if let Some(sender) = c.queued.pop_front() {
                    self.wake(sender, 0);
                    return Submit::Done(1);
                }
                if c.closed {
                    return Submit::Done(0);
                }
                c.receivers.push_back(Receiver { pid, arm: None });
                self.park(pid, WaitKind::Recv, vec![a0])
            }
            L::OP_CHAN_CLOSE => {
                if c.senders == 0 {
                    return Submit::Done(L::E_CLOSED);
                }
                c.senders -= 1;
                if c.senders == 0 {
                    c.closed = true;
                    let rs: Vec<Receiver> = c.receivers.iter().copied().collect();
                    for r in rs {
                        self.wake(r.pid, r.bit(0));
                    }
                }
                Submit::Done(0)
            }
            _ => Submit::Done(L::E_NOT_SUPPORTED),
        }
    }

    fn alt(&mut self, pid: Pid, addr: i32, count: i32, mem: &[u8]) -> Submit {
        let ids: Vec<i32> = (0..count.max(0) as usize)
            .map(|i| {
                let a = addr as usize + 4 * i;
                i32::from_le_bytes(mem[a..a + 4].try_into().unwrap())
            })
            .collect();
        if ids.iter().any(|id| !self.chans.contains_key(id)) {
            return Submit::Done(L::E_BAD_HANDLE);
        }
        for (i, id) in ids.iter().enumerate() {
            let c = self.chans.get_mut(id).unwrap();
            if let Some(sender) = c.queued.pop_front() {
                self.wake(sender, 0);
                return Submit::Done(2 * i as i32 + 1);
            }
            if c.closed {
                return Submit::Done(2 * i as i32);
            }
        }
        for (i, id) in ids.iter().enumerate() {
            let r = Receiver {
                pid,
                arm: Some(i as i32),
            };
            self.chans.get_mut(id).unwrap().receivers.push_back(r);
        }
        self.park(pid, WaitKind::Alt, ids)
    }

    /// The next ready process, in FIFO order; when none is ready, the
    /// earliest due sleeper (equal deadlines in the order they slept).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Resume> {
        if self.ready.is_empty() {
            if let Some(i) = self.earliest() {
                if self.sleepers[i].0 <= (self.now)() {
                    let (_, pid) = self.sleepers.remove(i);
                    self.wake(pid, 0);
                }
            }
        }
        let r = self.ready.pop_front()?;
        self.procs.get_mut(&r.pid).unwrap().state = State::Running;
        Some(r)
    }

    fn earliest(&self) -> Option<usize> {
        (0..self.sleepers.len()).min_by_key(|&i| self.sleepers[i].0)
    }

    /// Milliseconds until the earliest sleeper is due (0 if it is); `None`
    /// when nothing sleeps.
    pub fn sleep_for(&self) -> Option<i64> {
        let i = self.earliest()?;
        Some((self.sleepers[i].0 - (self.now)()).max(0))
    }

    pub fn exit(&mut self, pid: Pid) {
        self.sleepers.retain(|&(_, p)| p != pid);
        if let Some(p) = self.procs.get_mut(&pid) {
            p.state = State::Done;
            p.wait = None;
        }
    }

    /// Stop a process: it leaves every wait list and is never resumed. A value
    /// it was sending stays queued and is still delivered.
    pub fn kill(&mut self, pid: Pid) -> bool {
        match self.procs.get_mut(&pid) {
            Some(p) if p.state != State::Done => {
                p.state = State::Done;
                p.wait = None;
            }
            _ => return false,
        }
        for c in self.chans.values_mut() {
            c.receivers.retain(|r| r.pid != pid);
        }
        self.sleepers.retain(|&(_, p)| p != pid);
        self.ready.retain(|r| r.pid != pid);
        self.killed.push(pid);
        true
    }

    pub fn io_start(&mut self, pid: Pid) {
        self.procs.get_mut(&pid).unwrap().state = State::Io;
    }

    pub fn io_done(&mut self, pid: Pid) {
        let p = self.procs.get_mut(&pid).unwrap();
        if p.state == State::Io {
            p.state = State::Running;
        }
    }

    pub fn live(&self) -> Vec<Pid> {
        self.procs
            .iter()
            .filter(|(_, p)| p.state != State::Done)
            .map(|(&pid, _)| pid)
            .collect()
    }

    /// `None` while a process can still run (one is ready, running, sleeping
    /// or waiting on I/O); otherwise the message naming what each parked process waits on.
    pub fn blocked(&self, name: &dyn Fn(Pid) -> String) -> Option<String> {
        if self
            .procs
            .values()
            .any(|p| matches!(p.state, State::Ready | State::Running | State::Io))
            || !self.sleepers.is_empty()
        {
            return None;
        }
        let parts: Vec<String> = self
            .procs
            .iter()
            .filter(|(_, p)| p.state == State::Parked)
            .map(|(&pid, p)| {
                let w = p.wait.as_ref().unwrap();
                let chans = w
                    .chans
                    .iter()
                    .map(|id| format!("chan {id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                match w.kind {
                    WaitKind::Recv => format!("{} waits to receive on {chans}", name(pid)),
                    WaitKind::Send => format!("{} waits to send on {chans}", name(pid)),
                    WaitKind::Alt => format!("{} waits on {chans} (alt)", name(pid)),
                    WaitKind::Sleep => format!("{} sleeps", name(pid)),
                }
            })
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(format!("all processes blocked: {}", parts.join("; ")))
        }
    }

    // ---- `/prog`: the live processes, and `/prog/<pid>/ctl` to kill one.

    pub fn prog_open(&mut self, path: &str, mode: i32) -> i32 {
        if path == "/prog" || path == "/prog/" {
            if mode != L::MODE_READ {
                return L::E_PERMISSION;
            }
            let records = self
                .live()
                .into_iter()
                .map(|pid| {
                    let name = pid.to_string();
                    let mut rec = Vec::with_capacity(13 + name.len());
                    rec.extend_from_slice(&(name.len() as u32).to_le_bytes());
                    rec.extend_from_slice(name.as_bytes());
                    rec.extend_from_slice(&0u64.to_le_bytes()); // size 0
                    rec.push(1); // a directory
                    rec
                })
                .collect();
            return self.prog_add(ProgHandle::Dir(records));
        }
        let pid = path
            .strip_prefix("/prog/")
            .and_then(|s| s.strip_suffix("/ctl"))
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|s| s.parse::<Pid>().ok());
        match pid.and_then(|pid| self.procs.get(&pid).map(|p| (pid, p))) {
            Some((pid, p)) if p.state != State::Done => self.prog_add(ProgHandle::Ctl(pid)),
            _ => L::E_NOT_FOUND,
        }
    }

    fn prog_add(&mut self, h: ProgHandle) -> i32 {
        let id = self.next_prog_handle;
        self.next_prog_handle += 1;
        self.prog_handles.insert(id, h);
        id
    }

    pub fn prog_read(&mut self, handle: i32, buf: &mut [u8]) -> i32 {
        let records = match self.prog_handles.get_mut(&handle) {
            None => return L::E_BAD_HANDLE,
            Some(ProgHandle::Ctl(_)) => return L::E_PERMISSION,
            Some(ProgHandle::Dir(records)) => records,
        };
        let mut n = 0;
        while let Some(rec) = records.front() {
            if n + rec.len() > buf.len() {
                break;
            }
            buf[n..n + rec.len()].copy_from_slice(rec);
            n += rec.len();
            records.pop_front();
        }
        if n == 0 && !records.is_empty() {
            L::E_IO
        } else {
            n as i32
        }
    }

    pub fn prog_write(&mut self, handle: i32, bytes: &[u8]) -> i32 {
        let pid = match self.prog_handles.get(&handle) {
            None => return L::E_BAD_HANDLE,
            Some(ProgHandle::Dir(_)) => return L::E_PERMISSION,
            Some(ProgHandle::Ctl(pid)) => *pid,
        };
        let text = String::from_utf8_lossy(bytes);
        if text.strip_suffix('\n').unwrap_or(&text) != "kill" {
            return L::E_MALFORMED;
        }
        self.kill(pid);
        bytes.len() as i32
    }

    pub fn prog_close(&mut self, handle: i32) -> i32 {
        if self.prog_handles.remove(&handle).is_some() {
            0
        } else {
            L::E_BAD_HANDLE
        }
    }

    pub fn is_prog_handle(h: i32) -> bool {
        h >= PROG_HANDLE_BASE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Submit::{Done, Park};

    fn ids(mem: &mut [u8], xs: &[i32]) -> (i32, i32) {
        for (i, x) in xs.iter().enumerate() {
            mem[4 * i..4 * i + 4].copy_from_slice(&x.to_le_bytes());
        }
        (0, xs.len() as i32)
    }

    fn run(s: &mut Scheduler) -> Option<(Pid, i32)> {
        s.next().map(|r| (r.pid, r.result))
    }

    fn sub(s: &mut Scheduler, pid: Pid, op: i32, a0: i32) -> Submit {
        s.submit(pid, op, a0, 0, 0, &[])
    }

    fn alt(s: &mut Scheduler, pid: Pid, chans: &[i32]) -> Submit {
        let mut mem = [0u8; 64];
        let (a, n) = ids(&mut mem, chans);
        s.submit(pid, L::OP_ALT, a, n, 0, &mem)
    }

    fn make(s: &mut Scheduler) -> i32 {
        match sub(s, 0, L::OP_CHAN_MAKE, 0) {
            Done(id) => id,
            Park => panic!("chan.make parked"),
        }
    }

    fn default_name(pid: Pid) -> String {
        if pid == 0 {
            "main".into()
        } else {
            format!("process {pid}")
        }
    }

    #[test]
    fn send_parks_until_a_receive_takes_the_value() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        let p = s.spawn(7);
        assert_eq!(
            s.next(),
            Some(Resume {
                pid: p,
                result: 0,
                start: true
            })
        );
        assert_eq!(sub(&mut s, p, L::OP_CHAN_SEND, c), Park);
        assert_eq!(s.next(), None);
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Done(1));
        assert_eq!(
            s.next(),
            Some(Resume {
                pid: p,
                result: 0,
                start: false
            })
        );
    }

    #[test]
    fn receive_parks_until_a_send() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        let p = s.spawn(7);
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Park);
        assert_eq!(run(&mut s), Some((p, 0)));
        assert_eq!(sub(&mut s, p, L::OP_CHAN_SEND, c), Done(0));
        assert_eq!(run(&mut s), Some((0, 1)));
    }

    #[test]
    fn two_senders_and_counted_close() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_SENDER, c), Done(0));
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Park);
        let p = s.spawn(7);
        run(&mut s);
        assert_eq!(sub(&mut s, p, L::OP_CHAN_CLOSE, c), Done(0));
        assert_eq!(s.next(), None, "one sender left: the receiver still waits");
        assert_eq!(sub(&mut s, p, L::OP_CHAN_CLOSE, c), Done(0));
        assert_eq!(run(&mut s), Some((0, 0)));
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Done(0));
        assert_eq!(
            sub(&mut s, 0, L::OP_CHAN_CLOSE, c),
            Done(L::E_CLOSED),
            "over-close"
        );
        assert_eq!(
            sub(&mut s, 0, L::OP_CHAN_SEND, c),
            Done(L::E_CLOSED),
            "send after close"
        );
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, 99), Done(L::E_BAD_HANDLE));
    }

    #[test]
    fn a_value_queued_before_the_close_is_still_received() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        let p = s.spawn(7);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_SEND, c);
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_CLOSE, c), Done(0));
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Done(1));
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Done(0));
    }

    #[test]
    fn alt_lowest_ready_arm_and_wake_from_either() {
        let mut s = Scheduler::new();
        let a = make(&mut s);
        let b = make(&mut s);
        let p = s.spawn(7);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_SEND, b);
        let q = s.spawn(8);
        run(&mut s);
        sub(&mut s, q, L::OP_CHAN_SEND, a);
        assert_eq!(alt(&mut s, 0, &[a, b]), Done(1), "arm 0 takes a value");
        assert_eq!(run(&mut s), Some((q, 0)));
        assert_eq!(alt(&mut s, 0, &[a, b]), Done(3), "arm 1 takes a value");
        assert_eq!(run(&mut s), Some((p, 0)));
        assert_eq!(alt(&mut s, 0, &[a, b]), Park);
        let r = s.spawn(9);
        run(&mut s);
        assert_eq!(sub(&mut s, r, L::OP_CHAN_SEND, b), Done(0));
        assert_eq!(run(&mut s), Some((0, 3)), "woken by b, arm 1");
        assert_eq!(s.chans[&a].receivers.len(), 0, "removed from a's waiters");
        assert_eq!(alt(&mut s, 0, &[a, b]), Park);
        assert_eq!(sub(&mut s, r, L::OP_CHAN_CLOSE, a), Done(0));
        assert_eq!(
            run(&mut s),
            Some((0, 0)),
            "a closed and drained: arm 0, bit 0"
        );
        assert_eq!(alt(&mut s, 0, &[a, 42]), Done(L::E_BAD_HANDLE));
    }

    #[test]
    fn a_killed_receiver_is_not_woken() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        let p = s.spawn(7);
        run(&mut s);
        assert_eq!(sub(&mut s, p, L::OP_CHAN_RECV, c), Park);
        assert!(s.kill(p));
        assert!(!s.kill(p));
        assert!(!s.kill(42));
        sub(&mut s, 0, L::OP_CHAN_CLOSE, c);
        assert_eq!(s.next(), None);
        assert_eq!(s.live(), vec![0]);
    }

    #[test]
    fn blocked_names_each_parked_process() {
        let mut s = Scheduler::new();
        let a = make(&mut s);
        let b = make(&mut s);
        let p = s.spawn(7);
        let q = s.spawn(8);
        assert_eq!(s.blocked(&default_name), None, "processes are ready");
        run(&mut s);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_SEND, a);
        alt(&mut s, q, &[a, b]);
        assert_eq!(s.blocked(&default_name), None, "main is running");
        sub(&mut s, 0, L::OP_CHAN_RECV, b);
        // q's alt took p's value at once, so p and q are ready again.
        assert_eq!(s.blocked(&default_name), None);
        run(&mut s);
        run(&mut s);
        s.exit(p);
        sub(&mut s, q, L::OP_CHAN_RECV, a);
        let line = |pid: Pid| {
            if pid == 0 {
                "[line 1]".into()
            } else {
                format!("process {pid}")
            }
        };
        assert_eq!(
            s.blocked(&line).as_deref(),
            Some(
                "all processes blocked: [line 1] waits to receive on chan 2; \
                 process 2 waits to receive on chan 1"
            )
        );
        s.io_start(0);
        assert_eq!(
            s.blocked(&default_name),
            None,
            "waiting on I/O is not blocked"
        );
        s.io_done(0);
        assert_eq!(s.blocked(&default_name), None, "running again");
    }

    #[test]
    fn blocked_names_alt_and_send_waits() {
        let mut s = Scheduler::new();
        let a = make(&mut s);
        let b = make(&mut s);
        let p = s.spawn(7);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_SEND, a);
        alt(&mut s, 0, &[b, a]);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_SEND, a);
        alt(&mut s, 0, &[b]);
        assert_eq!(
            s.blocked(&default_name).as_deref(),
            Some("all processes blocked: main waits on chan 2 (alt); process 1 waits to send on chan 1")
        );
    }

    #[test]
    fn prog_lists_live_pids_and_kills_through_ctl() {
        let mut s = Scheduler::new();
        let c = make(&mut s);
        let p = s.spawn(7);
        run(&mut s);
        sub(&mut s, p, L::OP_CHAN_RECV, c);
        let d = s.prog_open("/prog", L::MODE_READ);
        assert!(Scheduler::is_prog_handle(d));
        let mut buf = [0u8; 64];
        let n = s.prog_read(d, &mut buf) as usize;
        let mut names = Vec::new();
        let mut at = 0;
        while at < n {
            let len = u32::from_le_bytes(buf[at..at + 4].try_into().unwrap()) as usize;
            names.push(String::from_utf8(buf[at + 4..at + 4 + len].to_vec()).unwrap());
            assert_eq!(buf[at + 12 + len], 1, "a directory");
            at += 13 + len;
        }
        assert_eq!(names, vec!["0".to_string(), p.to_string()]);
        assert_eq!(s.prog_read(d, &mut buf), 0, "every record read");
        assert_eq!(s.prog_close(d), 0);
        assert_eq!(s.prog_open("/prog", L::MODE_WRITE), L::E_PERMISSION);
        let ctl = s.prog_open(&format!("/prog/{p}/ctl"), L::MODE_WRITE);
        assert!(ctl > 0);
        assert_eq!(s.prog_write(ctl, b"stop"), L::E_MALFORMED);
        assert_eq!(s.prog_read(ctl, &mut buf), L::E_PERMISSION);
        assert_eq!(s.prog_write(ctl, b"kill\n"), 5);
        assert_eq!(s.killed, vec![p]);
        assert_eq!(
            s.chans[&c].receivers.len(),
            0,
            "the killed receiver left its channel"
        );
        assert_eq!(s.prog_open("/prog/9/ctl", L::MODE_WRITE), L::E_NOT_FOUND);
        assert_eq!(
            s.prog_open(&format!("/prog/{p}/ctl"), L::MODE_WRITE),
            L::E_NOT_FOUND,
            "finished"
        );
        assert_eq!(s.prog_open("/prog/x", L::MODE_READ), L::E_NOT_FOUND);
        assert_eq!(s.prog_close(12345), L::E_BAD_HANDLE);
    }

    #[test]
    fn prog_read_returns_whole_records_only() {
        let mut s = Scheduler::new();
        s.spawn(7);
        let d = s.prog_open("/prog/", L::MODE_READ);
        let mut small = [0u8; 10];
        assert_eq!(s.prog_read(d, &mut small), L::E_IO, "no record fits");
        let mut one = [0u8; 14];
        assert_eq!(s.prog_read(d, &mut one), 14, "pid 0's record");
        assert_eq!(s.prog_read(d, &mut one), 14, "pid 1's record");
        assert_eq!(s.prog_read(d, &mut one), 0);
    }

    fn clocked() -> (Scheduler, std::rc::Rc<std::cell::Cell<i64>>) {
        let t = std::rc::Rc::new(std::cell::Cell::new(0));
        let c = t.clone();
        (Scheduler::with_clock(Box::new(move || c.get())), t)
    }

    fn started(s: &mut Scheduler) -> Pid {
        let p = s.spawn(7);
        assert_eq!(run(s), Some((p, 0)));
        p
    }

    #[test]
    fn sleep_parks_until_the_deadline() {
        let (mut s, t) = clocked();
        let p = started(&mut s);
        assert_eq!(sub(&mut s, p, L::OP_SLEEP, 20), Park);
        assert_eq!(s.next(), None);
        assert_eq!(s.sleep_for(), Some(20));
        t.set(19);
        assert_eq!(s.next(), None);
        t.set(20);
        assert_eq!(
            s.next(),
            Some(Resume {
                pid: p,
                result: 0,
                start: false
            })
        );
        assert_eq!(s.sleep_for(), None);
    }

    #[test]
    fn sleepers_wake_in_deadline_order_then_sleep_order() {
        let (mut s, t) = clocked();
        let (a, b, c, d) = (
            started(&mut s),
            started(&mut s),
            started(&mut s),
            started(&mut s),
        );
        sub(&mut s, a, L::OP_SLEEP, 30);
        sub(&mut s, b, L::OP_SLEEP, 10);
        sub(&mut s, c, L::OP_SLEEP, 40);
        sub(&mut s, d, L::OP_SLEEP, 40);
        t.set(100);
        assert_eq!(run(&mut s), Some((b, 0)));
        assert_eq!(run(&mut s), Some((a, 0)));
        assert_eq!(run(&mut s), Some((c, 0)));
        assert_eq!(run(&mut s), Some((d, 0)));
        assert_eq!(run(&mut s), None);
    }

    #[test]
    fn a_sleeper_is_not_blocked() {
        let (mut s, _t) = clocked();
        let c = make(&mut s);
        let p = started(&mut s);
        assert_eq!(sub(&mut s, p, L::OP_SLEEP, 10), Park);
        assert_eq!(sub(&mut s, 0, L::OP_CHAN_RECV, c), Park);
        assert_eq!(s.blocked(&default_name), None);
    }

    #[test]
    fn a_killed_sleeper_is_gone() {
        let (mut s, t) = clocked();
        let p = started(&mut s);
        sub(&mut s, p, L::OP_SLEEP, 10);
        assert!(s.kill(p));
        assert_eq!(s.sleep_for(), None);
        t.set(50);
        assert_eq!(s.next(), None);
    }

    #[test]
    fn a_negative_sleep_is_due_at_once() {
        let (mut s, _t) = clocked();
        let p = started(&mut s);
        assert_eq!(sub(&mut s, p, L::OP_SLEEP, -5), Park);
        assert_eq!(s.sleep_for(), Some(0));
        assert_eq!(run(&mut s), Some((p, 0)));
    }
}
