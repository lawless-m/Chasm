//! Run Whackford modules natively with wasmtime.

use std::collections::HashMap;

use wack_core::layout as L;
use wack_core::types::Ty;
use wack_core::{Compilation, TestInfo, Value};
use wasmtime::{
    AsContextMut, Caller, Config as WtConfig, Engine, Func, Global, Instance, Linker, Memory,
    Module, Ref, RefType, RootScope, Store, Table, TableType, Val,
};

use crate::namespace::{Config, NativeHost};
use crate::proc::{Pid, Scheduler, Submit};
use crate::{complete, range, rd, service_entry, service_ring, trap_info, wr};

struct State {
    host: NativeHost,
    procs: Option<Procs>,
}

fn run_procs(s: &mut State) -> &mut Procs {
    s.procs.as_mut().expect("a process run")
}

/// The processes of a transformed module (M12): the scheduler, the running
/// process, the ring user of each parked submission, the `wack.frames` and
/// `wack.spawn` globals, and per pid its saved frame chain and its closure,
/// in host-side tables so the collector keeps them alive.
pub(crate) struct Procs {
    pub sched: Scheduler,
    pub current: Pid,
    pub pending_user: HashMap<Pid, u32>,
    pub frames: Global,
    pub spawn: Global,
    pub chains: Table,
    pub closures: Table,
}

impl Procs {
    pub fn new(store: &mut impl wasmtime::AsContextMut) -> wasmtime::Result<Procs> {
        let ty = TableType::new(RefType::ANYREF, 16, None);
        Ok(Procs {
            sched: Scheduler::new(),
            current: 0,
            pending_user: HashMap::new(),
            frames: anyref_global(&mut *store)?,
            spawn: anyref_global(&mut *store)?,
            chains: Table::new(&mut *store, ty.clone(), Ref::Any(None))?,
            closures: Table::new(&mut *store, ty, Ref::Any(None))?,
        })
    }
}

/// A process killed itself through `/prog/<pid>/ctl`: its call ends with
/// this error, which the driver takes as the process's end.
#[derive(Debug)]
pub(crate) struct Killed;

impl std::fmt::Display for Killed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("killed")
    }
}

impl std::error::Error for Killed {}

/// What one ring entry of a process does.
enum Entry {
    Complete(i32),
    Park,
    Spawn(u32),
}

/// Service the one submission a transformed module's `rt.ring` made:
/// `/prog` through the scheduler, I/O inline, `spawn` and channel
/// operations through the scheduler. A process that must wait gets no
/// completion: the mode cell is set to unwinding and its frames are saved
/// as the stack returns.
pub(crate) fn process_ring_enter<T: 'static>(
    caller: &mut Caller<'_, T>,
    mem: Memory,
    parts: fn(&mut T) -> (&mut NativeHost, &mut Procs),
) -> wasmtime::Result<()> {
    let (data, state) = mem.data_and_store_mut(&mut *caller);
    let (host, p) = parts(state);
    let head = rd(data, L::SQ_HEAD);
    let e = L::SQ_BASE + (head % L::RING_ENTRIES) * L::SQE_SIZE;
    let op = rd(data, e + L::SQE_OP) as i32;
    let user = rd(data, e + L::SQE_USER);
    let (a0, a1, a2) = (
        rd(data, e + L::SQE_A0) as i32,
        rd(data, e + L::SQE_A1) as i32,
        rd(data, e + L::SQE_A2) as i32,
    );
    wr(data, L::SQ_HEAD, head.wrapping_add(1));
    let prog = Scheduler::is_prog_handle(a0);
    let entry = match op {
        L::OP_OPEN => {
            let path = range(data, a0, a1).and_then(|r| std::str::from_utf8(&data[r]).ok());
            match path {
                Some(path) if path == "/prog" || path.starts_with("/prog/") => {
                    let path = path.to_string();
                    Entry::Complete(p.sched.prog_open(&path, a2))
                }
                _ => Entry::Complete(service_entry(data, host, op, a0, a1, a2)),
            }
        }
        L::OP_READ if prog => Entry::Complete(match range(data, a1, a2) {
            Some(r) => p.sched.prog_read(a0, &mut data[r]),
            None => L::E_IO,
        }),
        L::OP_WRITE if prog => {
            let r = match range(data, a1, a2) {
                Some(r) => p.sched.prog_write(a0, &data[r]),
                None => L::E_IO,
            };
            // A process that killed itself stops here: its stack is
            // abandoned, through code that is not transformed too.
            if !p.sched.live().contains(&p.current) {
                return Err(wasmtime::Error::new(Killed));
            }
            Entry::Complete(r)
        }
        L::OP_CLOSE if prog => Entry::Complete(p.sched.prog_close(a0)),
        L::OP_SPAWN => Entry::Spawn(a0 as u32),
        L::OP_CHAN_MAKE..=L::OP_SLEEP => match p.sched.submit(p.current, op, a0, a1, a2, data) {
            Submit::Done(r) => Entry::Complete(r),
            Submit::Park => {
                p.pending_user.insert(p.current, user);
                Entry::Park
            }
        },
        _ => Entry::Complete(service_entry(data, host, op, a0, a1, a2)),
    };
    match entry {
        Entry::Complete(r) => complete(data, user, r),
        Entry::Park => wr(data, L::UNWIND_MODE, L::UNWINDING as u32),
        Entry::Spawn(slot) => {
            let pid = p.sched.spawn(slot);
            let (spawn, chains, closures) = (p.spawn, p.chains, p.closures);
            let closure = spawn.get(&mut *caller).unwrap_anyref().cloned();
            spawn.set(&mut *caller, Val::AnyRef(None))?;
            let size = closures.size(&mut *caller);
            if pid as u64 >= size {
                let more = (pid as u64 + 1 - size).max(16);
                closures.grow(&mut *caller, more, Ref::Any(None))?;
                chains.grow(&mut *caller, more, Ref::Any(None))?;
            }
            closures.set(&mut *caller, pid as u64, Ref::Any(closure))?;
            complete(mem.data_mut(&mut *caller), user, 0);
        }
    }
    Ok(())
}

/// Run `main` as process 0, then every process that can run, until process
/// 0 has returned and nothing is ready (processes still parked stay). A
/// process that must wait unwinds into its saved chain; resuming, its chain
/// is put back, its completion written, and its entry called again to
/// rewind. A process is started by calling its closure's slot in `table`.
/// While process 0 runs and nothing is ready, the driver sleeps until the
/// earliest sleeper is due; once it has returned, sleepers stay (or are
/// dropped with the run). When nothing is ready, nothing sleeps and process
/// 0 waits, it is killed and the result is
/// the all-processes-blocked trap, naming it `name`. A trap in a spawned
/// process ends the call, unless `process_traps` is given: then that
/// process alone ends and its trap is collected there.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drive<T: 'static>(
    store: &mut Store<T>,
    get: fn(&mut T) -> &mut Procs,
    mem: Memory,
    main: Func,
    table: Table,
    name: &str,
    results: usize,
    mut process_traps: Option<&mut Vec<RunError>>,
) -> Result<Vec<Val>, RunError> {
    enum Next {
        Main,
        Start(Pid),
        Resume(Pid, i32),
    }
    let fail = |e: wasmtime::Error| RunError {
        message: e.to_string(),
        word: None,
        process: None,
    };
    macro_rules! procs {
        ($scope:expr) => {
            get($scope.as_context_mut().data_mut())
        };
    }
    let p = get(store.data_mut());
    let (frames, chains, closures) = (p.frames, p.chains, p.closures);
    p.sched.start_main();
    let mut main_out = None;
    let mut next = Some(Next::Main);
    while let Some(n) = next.take() {
        let mut scope = RootScope::new(&mut *store);
        let pid = match n {
            Next::Main => 0,
            Next::Start(pid) | Next::Resume(pid, _) => pid,
        };
        if let Next::Resume(_, result) = n {
            let chain = chains
                .get(&mut scope, pid as u64)
                .and_then(|r| r.as_any().flatten().cloned());
            frames.set(&mut scope, Val::AnyRef(chain)).map_err(fail)?;
            chains
                .set(&mut scope, pid as u64, Ref::Any(None))
                .map_err(fail)?;
            let user = procs!(scope).pending_user.remove(&pid).unwrap_or(0);
            let data = mem.data_mut(&mut scope);
            complete(data, user, result);
            wr(data, L::UNWIND_MODE, L::REWINDING as u32);
        }
        procs!(scope).current = pid;
        let mut out = vec![Val::I32(0); if pid == 0 { results } else { 0 }];
        let r = if pid == 0 {
            main.call(&mut scope, &[], &mut out)
        } else {
            let closure = closures
                .get(&mut scope, pid as u64)
                .and_then(|r| r.as_any().flatten().cloned());
            let slot = procs!(scope).sched.slot(pid).unwrap_or(0);
            match table
                .get(&mut scope, slot as u64)
                .and_then(|r| r.as_func().flatten().cloned())
            {
                Some(f) => f.call(&mut scope, &[Val::AnyRef(closure)], &mut []),
                None => Err(wasmtime::Error::msg(format!("no function at slot {slot}"))),
            }
        };
        let killed = matches!(&r, Err(e) if e.downcast_ref::<Killed>().is_some());
        let mut ended = !matches!(r, Ok(()));
        match r {
            Err(_) if killed && pid > 0 => {}
            Err(e) => {
                let mut err = describe(&e, mem.data(&scope));
                if killed {
                    err.word = Some(name.to_string());
                }
                err.process = (pid > 0).then_some(pid);
                // The next trap is reported cleanly; no unwind is in progress.
                let data = mem.data_mut(&mut scope);
                wr(data, L::TRAP_MSG_LEN, 0);
                wr(data, L::UNWIND_MODE, L::UNWIND_OFF as u32);
                frames.set(&mut scope, Val::AnyRef(None)).map_err(fail)?;
                match process_traps.as_deref_mut() {
                    Some(traps) if pid > 0 => {
                        traps.push(err);
                        procs!(scope).sched.kill(pid);
                    }
                    _ => {
                        if pid == 0 {
                            procs!(scope).sched.kill(0);
                        }
                        return Err(err);
                    }
                }
            }
            Ok(()) => {}
        }
        let unwound = !ended && rd(mem.data(&scope), L::UNWIND_MODE) == L::UNWINDING as u32;
        if unwound {
            wr(
                mem.data_mut(&mut scope),
                L::UNWIND_MODE,
                L::UNWIND_OFF as u32,
            );
            let chain = frames.get(&mut scope).unwrap_anyref().cloned();
            frames.set(&mut scope, Val::AnyRef(None)).map_err(fail)?;
            if procs!(scope).sched.live().contains(&pid) {
                chains
                    .set(&mut scope, pid as u64, Ref::Any(chain))
                    .map_err(fail)?;
            }
        } else {
            ended = true;
        }
        if ended {
            procs!(scope).sched.exit(pid);
            if pid == 0 {
                main_out = Some(out);
            }
        }
        // Forget finished and killed processes.
        let mut gone = std::mem::take(&mut procs!(scope).sched.killed);
        if ended && pid > 0 {
            gone.push(pid);
        }
        for k in gone {
            procs!(scope).pending_user.remove(&k);
            for t in [chains, closures] {
                if (k as u64) < t.size(&mut scope) {
                    t.set(&mut scope, k as u64, Ref::Any(None)).map_err(fail)?;
                }
            }
        }
        let mut r = procs!(scope).sched.next();
        while r.is_none() && main_out.is_none() {
            let Some(ms) = procs!(scope).sched.sleep_for() else {
                break;
            };
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
            r = procs!(scope).sched.next();
        }
        next = r.map(|r| {
            if r.start {
                Next::Start(r.pid)
            } else {
                Next::Resume(r.pid, r.result)
            }
        });
    }
    if let Some(out) = main_out {
        return Ok(out);
    }
    let p = get(store.data_mut());
    let message = p
        .sched
        .blocked(&|pid| {
            if pid == 0 {
                name.to_string()
            } else {
                format!("process {pid}")
            }
        })
        .unwrap_or_else(|| "process 0 never finished".into());
    // Process 0 can never resume: abandon it and its chain.
    p.sched.kill(0);
    p.sched.killed.clear();
    p.pending_user.remove(&0);
    chains.set(&mut *store, 0, Ref::Any(None)).map_err(fail)?;
    Err(RunError {
        message,
        word: Some(name.to_string()),
        process: None,
    })
}

/// A failed run: the trap message, the word that trapped when known, and
/// the process it trapped in when that is not process 0.
#[derive(Debug, Clone)]
pub struct RunError {
    pub message: String,
    pub word: Option<String>,
    pub process: Option<u32>,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.word, self.process) {
            (Some(w), Some(p)) => write!(f, "trap in `{w}` (process {p}): {}", self.message),
            (Some(w), None) => write!(f, "trap in `{w}`: {}", self.message),
            (None, _) => write!(f, "trap: {}", self.message),
        }
    }
}

pub struct Runner {
    engine: Engine,
    module: Module,
}

pub struct Outcome<T> {
    pub result: Result<T, RunError>,
    pub host: NativeHost,
}

/// The engine every Whackford host uses: WasmGC on, with the copying collector
/// (far faster than the default for many short-lived structs), and Cranelift's
/// function inliner on. With `cache`, compiled modules are kept on disk
/// (`cache_dir`), keyed by the wasm and the engine settings, so an unchanged
/// program is not compiled again.
pub fn engine(cache: bool) -> Result<Engine, String> {
    // Backtraces (on by default) name the trapping word via the name section.
    let mut cfg = WtConfig::new();
    cfg.wasm_gc(true)
        .wasm_function_references(true)
        .collector(wasmtime::Collector::Copying)
        // Whackford has no inliner of its own, so without this every small
        // word (a struct field accessor, a helper) stays a call. Wasm-level
        // traps in an inlined word may be attributed to its caller; the
        // inliner is part of the cache key, so modules cached without it are
        // compiled afresh.
        .compiler_inlining(wasmtime::Inlining::Yes);
    if let Some(dir) = cache.then(cache_dir).flatten() {
        let mut cc = wasmtime::CacheConfig::new();
        cc.with_directory(dir);
        // A cache that cannot be set up only costs the compile.
        if let Ok(c) = wasmtime::Cache::new(cc) {
            cfg.cache(Some(c));
        }
    }
    Engine::new(&cfg).map_err(|e| e.to_string())
}

/// `$XDG_CACHE_HOME/wack`, else `$HOME/.cache/wack`.
fn cache_dir() -> Option<std::path::PathBuf> {
    let base = match std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        Some(x) => std::path::PathBuf::from(x),
        None => std::path::PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(base.join("wack"))
}

impl Runner {
    /// With `cache`, see `engine`.
    pub fn new(wasm: &[u8], cache: bool) -> Result<Self, String> {
        let engine = engine(cache)?;
        let module = Module::new(&engine, wasm).map_err(|e| e.to_string())?;
        Ok(Runner { engine, module })
    }

    /// Whether the module is transformed to unwind and rewind: it runs
    /// processes (M12).
    fn has_processes(&self) -> bool {
        self.module
            .imports()
            .any(|i| i.module() == L::IMPORT_MODULE && i.name() == L::IMPORT_FRAMES)
    }

    fn instantiate(&self, store: &mut Store<State>) -> Result<(Instance, Memory), String> {
        if self.has_processes() {
            return self.instantiate_processes(store);
        }
        let mut linker: Linker<State> = Linker::new(&self.engine);
        linker
            .func_wrap(
                wack_core::layout::IMPORT_MODULE,
                wack_core::layout::IMPORT_RING_ENTER,
                |mut caller: wasmtime::Caller<'_, State>| -> wasmtime::Result<()> {
                    let Some(mem) = caller
                        .get_export(wack_core::layout::EXPORT_MEMORY)
                        .and_then(|e| e.into_memory())
                    else {
                        return Ok(());
                    };
                    let (data, state) = mem.data_and_store_mut(&mut caller);
                    service_ring(data, &mut state.host);
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?;
        let inst = linker
            .instantiate(&mut *store, &self.module)
            .map_err(|e| e.to_string())?;
        let mem = inst
            .get_memory(&mut *store, wack_core::layout::EXPORT_MEMORY)
            .ok_or("module has no memory export")?;
        Ok((inst, mem))
    }

    /// Instantiate a transformed module: its ring entries go through the
    /// scheduler, and `wack.frames` and `wack.spawn` are the host's.
    fn instantiate_processes(
        &self,
        store: &mut Store<State>,
    ) -> Result<(Instance, Memory), String> {
        let e = |e: wasmtime::Error| e.to_string();
        let procs = Procs::new(&mut *store).map_err(e)?;
        let mut linker: Linker<State> = Linker::new(&self.engine);
        linker
            .func_wrap(
                L::IMPORT_MODULE,
                L::IMPORT_RING_ENTER,
                |mut caller: Caller<'_, State>| -> wasmtime::Result<()> {
                    let mem = caller
                        .get_export(L::EXPORT_MEMORY)
                        .and_then(|e| e.into_memory())
                        .expect("a module exports its memory");
                    process_ring_enter(&mut caller, mem, |s: &mut State| {
                        (&mut s.host, s.procs.as_mut().expect("a process run"))
                    })
                },
            )
            .map_err(e)?;
        linker
            .define(
                &mut *store,
                L::IMPORT_MODULE,
                L::IMPORT_FRAMES,
                procs.frames,
            )
            .map_err(e)?;
        linker
            .define(&mut *store, L::IMPORT_MODULE, L::IMPORT_SPAWN, procs.spawn)
            .map_err(e)?;
        store.data_mut().procs = Some(procs);
        let inst = linker.instantiate(&mut *store, &self.module).map_err(e)?;
        let mem = inst
            .get_memory(&mut *store, L::EXPORT_MEMORY)
            .ok_or("module has no memory export")?;
        Ok((inst, mem))
    }

    /// Call an exported function with no arguments in a fresh instance.
    /// When the module runs processes, the call is process 0, named `name`
    /// in an all-processes-blocked message.
    pub fn call(
        &self,
        config: Config,
        export: &str,
        name: &str,
        results: usize,
    ) -> Outcome<(Vec<Val>, Vec<u8>)> {
        let mut store = Store::new(
            &self.engine,
            State {
                host: NativeHost::new(config),
                procs: None,
            },
        );
        let result = (|| {
            let (inst, mem) = self.instantiate(&mut store).map_err(|m| RunError {
                message: m,
                word: None,
                process: None,
            })?;
            if store.data().procs.is_some() {
                let fail = |message: String| RunError {
                    message,
                    word: None,
                    process: None,
                };
                let main = inst
                    .get_func(&mut store, export)
                    .ok_or_else(|| fail(format!("no exported function `{export}`")))?;
                let table = inst
                    .get_table(&mut store, L::EXPORT_TABLE)
                    .ok_or_else(|| fail("a transformed module exports its table".into()))?;
                let out = drive(&mut store, run_procs, mem, main, table, name, results, None)?;
                return Ok((out, mem.data(&store).to_vec()));
            }
            let f: Func = inst.get_func(&mut store, export).ok_or_else(|| RunError {
                message: format!("no exported function `{export}`"),
                word: None,
                process: None,
            })?;
            let mut out = vec![Val::I32(0); results];
            match f.call(&mut store, &[], &mut out) {
                Ok(()) => Ok((out, mem.data(&store).to_vec())),
                Err(e) => Err(describe(&e, mem.data(&store))),
            }
        })();
        Outcome {
            result,
            host: store.into_data().host,
        }
    }

    /// Run `main`.
    pub fn run_main(&self, config: Config) -> Outcome<()> {
        let o = self.call(config, "main", "main", 0);
        Outcome {
            result: o.result.map(|_| ()),
            host: o.host,
        }
    }
}

/// A mutable `anyref` global, null: `wack.frames` or `wack.spawn`.
pub(crate) fn anyref_global<T>(
    store: impl wasmtime::AsContextMut<Data = T>,
) -> wasmtime::Result<wasmtime::Global> {
    wasmtime::Global::new(
        store,
        wasmtime::GlobalType::new(wasmtime::ValType::ANYREF, wasmtime::Mutability::Var),
        Val::AnyRef(None),
    )
}

pub(crate) fn describe(e: &wasmtime::Error, mem: &[u8]) -> RunError {
    if let Some((message, word)) = trap_info(mem) {
        return RunError {
            message,
            word: Some(word),
            process: None,
        };
    }
    let word = e.downcast_ref::<wasmtime::WasmBacktrace>().and_then(|bt| {
        bt.frames()
            .iter()
            .filter_map(|f| f.func_name())
            .find(|n| !n.starts_with("rt."))
            .map(str::to_string)
    });
    let message = match e.downcast_ref::<wasmtime::Trap>() {
        Some(t) => t.to_string(),
        None => e.to_string(),
    };
    RunError {
        message,
        word,
        process: None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TestStatus {
    Pass,
    Fail,
    Pending,
}

impl TestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestStatus::Pass => "pass",
            TestStatus::Fail => "fail",
            TestStatus::Pending => "pending",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestResult {
    pub test: TestInfo,
    pub status: TestStatus,
    pub actual: Option<Vec<Value>>,
    pub error: Option<RunError>,
    pub output: Vec<u8>,
}

pub(crate) fn values(tys: &[Ty], vals: &[Val], mem: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    for t in tys {
        match t {
            Ty::I64 => out.push(Value::I64(vals[i].unwrap_i64())),
            Ty::F32 => out.push(Value::F32(vals[i].unwrap_f32())),
            Ty::F64 => out.push(Value::F64(vals[i].unwrap_f64())),
            Ty::Str => {
                let (a, n) = (
                    vals[i].unwrap_i32() as u32 as usize,
                    vals[i + 1].unwrap_i32() as u32 as usize,
                );
                let bytes = mem.get(a..a.saturating_add(n)).unwrap_or(&[]);
                out.push(Value::Str(String::from_utf8_lossy(bytes).into_owned()));
                i += 1;
            }
            Ty::Bytes => {
                out.push(Value::Opaque(format!(
                    "<{} bytes>",
                    vals[i + 1].unwrap_i32() as u32
                )));
                i += 1;
            }
            Ty::Quot(_) => out.push(Value::Opaque(t.to_string())),
            _ => out.push(Value::I32(vals[i].unwrap_i32())),
        }
        i += 1;
    }
    out
}

pub(crate) fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::F32(x), Value::F32(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Value::F64(x), Value::F64(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => a == b,
    }
}

/// Run every test of a compilation built with `test_exports`, each in a
/// fresh instance with a captured console. With `cache`, see `engine`.
pub fn run_tests(c: &Compilation, base: &Config, cache: bool) -> Result<Vec<TestResult>, String> {
    let wasm = c.wasm.as_ref().ok_or("no module: the program has errors")?;
    let runner = Runner::new(wasm, cache)?;
    let mut out = Vec::new();
    for t in &c.tests {
        if t.pending {
            out.push(TestResult {
                test: t.clone(),
                status: TestStatus::Pending,
                actual: None,
                error: None,
                output: Vec::new(),
            });
            continue;
        }
        let mut cfg = base.clone();
        cfg.console = crate::namespace::Console::Capture {
            input: Vec::new(),
            pos: 0,
            output: Vec::new(),
        };
        let n: usize = t.result_types.iter().map(|t| t.width() as usize).sum();
        let o = runner.call(cfg, &t.export_name, &t.word, n);
        let output = o.host.captured_output().to_vec();
        let r = match o.result {
            Ok((vals, mem)) => {
                let actual = values(&t.result_types, &vals, &mem);
                let pass = !t.traps
                    && actual.len() == t.expected.len()
                    && actual.iter().zip(&t.expected).all(|(a, b)| same(a, b));
                TestResult {
                    test: t.clone(),
                    status: if pass {
                        TestStatus::Pass
                    } else {
                        TestStatus::Fail
                    },
                    actual: Some(actual),
                    error: None,
                    output,
                }
            }
            Err(e) => TestResult {
                test: t.clone(),
                status: if t.traps {
                    TestStatus::Pass
                } else {
                    TestStatus::Fail
                },
                actual: None,
                error: Some(e),
                output,
            },
        };
        out.push(r);
    }
    Ok(out)
}
