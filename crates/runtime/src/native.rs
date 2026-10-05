//! Run Whackford modules natively with wasmtime.

use wack_core::types::Ty;
use wack_core::{Compilation, TestInfo, Value};
use wasmtime::{Config as WtConfig, Engine, Func, Instance, Linker, Memory, Module, Store, Val};

use crate::namespace::{Config, NativeHost};
use crate::{service_ring, trap_info};

struct State {
    host: NativeHost,
}

/// A failed run: the trap message and, when known, the word that trapped.
#[derive(Debug, Clone)]
pub struct RunError {
    pub message: String,
    pub word: Option<String>,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.word {
            Some(w) => write!(f, "trap in `{w}`: {}", self.message),
            None => write!(f, "trap: {}", self.message),
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
/// (far faster than the default for many short-lived structs).
pub fn engine() -> Result<Engine, String> {
    // Backtraces (on by default) name the trapping word via the name section.
    let mut cfg = WtConfig::new();
    cfg.wasm_gc(true)
        .wasm_function_references(true)
        .collector(wasmtime::Collector::Copying);
    Engine::new(&cfg).map_err(|e| e.to_string())
}

impl Runner {
    pub fn new(wasm: &[u8]) -> Result<Self, String> {
        let engine = engine()?;
        let module = Module::new(&engine, wasm).map_err(|e| e.to_string())?;
        Ok(Runner { engine, module })
    }

    fn instantiate(&self, store: &mut Store<State>) -> Result<(Instance, Memory), String> {
        let mut linker: Linker<State> = Linker::new(&self.engine);
        linker
            .func_wrap(
                wack_core::layout::IMPORT_MODULE,
                wack_core::layout::IMPORT_RING_ENTER,
                |mut caller: wasmtime::Caller<'_, State>| {
                    let Some(mem) = caller
                        .get_export(wack_core::layout::EXPORT_MEMORY)
                        .and_then(|e| e.into_memory())
                    else {
                        return;
                    };
                    let (data, state) = mem.data_and_store_mut(&mut caller);
                    service_ring(data, &mut state.host);
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

    /// Call an exported function with no arguments in a fresh instance.
    pub fn call(
        &self,
        config: Config,
        export: &str,
        results: usize,
    ) -> Outcome<(Vec<Val>, Vec<u8>)> {
        let mut store = Store::new(
            &self.engine,
            State {
                host: NativeHost::new(config),
            },
        );
        let result = (|| {
            let (inst, mem) = self.instantiate(&mut store).map_err(|m| RunError {
                message: m,
                word: None,
            })?;
            let f: Func = inst.get_func(&mut store, export).ok_or_else(|| RunError {
                message: format!("no exported function `{export}`"),
                word: None,
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
        let o = self.call(config, "main", 0);
        Outcome {
            result: o.result.map(|_| ()),
            host: o.host,
        }
    }
}

pub(crate) fn describe(e: &wasmtime::Error, mem: &[u8]) -> RunError {
    if let Some((message, word)) = trap_info(mem) {
        return RunError {
            message,
            word: Some(word),
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
    RunError { message, word }
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
/// fresh instance with a captured console.
pub fn run_tests(c: &Compilation, base: &Config) -> Result<Vec<TestResult>, String> {
    let wasm = c.wasm.as_ref().ok_or("no module: the program has errors")?;
    let runner = Runner::new(wasm)?;
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
        let o = runner.call(cfg, &t.export_name, n);
        let output = o.host.captured_output().to_vec();
        let r = match o.result {
            Ok((vals, mem)) => {
                let actual = values(&t.result_types, &vals, &mem);
                let pass = actual.len() == t.expected.len()
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
                status: TestStatus::Fail,
                actual: None,
                error: Some(e),
                output,
            },
        };
        out.push(r);
    }
    Ok(out)
}
