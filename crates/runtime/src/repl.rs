//! The native REPL host: one wasmtime store holding the shared memory and
//! table, into which each step's module is instantiated.
//!
//! The session (`chasm_core::repl`) decides what to compile; this module
//! follows its host contract: place the literals, install the functions in
//! the table, run the line, and restore the memory data stack on a trap.

use std::time::Instant;

use chasm_core::layout as L;
use chasm_core::repl::{read_stack, Defined, Layout, StackEntry, Step};
use chasm_core::{Diagnostic, Location, Session, Value};
use serde::Serialize;
use std::collections::HashMap;

use chasm_core::types::Ty;
use wasmtime::{
    AnyRef, AsContext, Caller, Engine, Linker, Memory, MemoryType, Module, Ref, RefType, RootScope,
    Rooted, Store, Table, TableType, Val,
};

use crate::namespace::{Config, NativeHost};
use crate::native::{describe, same, values, RunError, TestStatus};
use crate::service_ring;

struct ReplState {
    host: NativeHost,
}

/// Wall time of one step, in microseconds.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Timing {
    /// Checking and assembling the step module (core).
    pub compile_us: u64,
    /// Placing literals, wasmtime compilation, instantiation, table installs.
    pub instantiate_us: u64,
    /// Running the line.
    pub run_us: u64,
}

/// A test run in the shared instance. Its console output is not captured.
pub struct ReplTestResult {
    pub word: String,
    pub status: TestStatus,
    pub expected: Vec<Value>,
    pub actual: Option<Vec<Value>>,
    pub error: Option<RunError>,
    pub location: Location,
}

pub struct Outcome {
    pub diagnostics: Vec<Diagnostic>,
    pub defined: Vec<Defined>,
    pub forgotten: Vec<String>,
    pub forced: Vec<chasm_core::repl::Forced>,
    pub rechecked: Vec<String>,
    /// The program as it stands, from `)words`.
    pub listing: Option<String>,
    pub trap: Option<RunError>,
    pub tests: Vec<ReplTestResult>,
    pub stack: Vec<StackEntry>,
    pub timing: Timing,
}

pub struct NativeRepl {
    engine: Engine,
    store: Store<ReplState>,
    linker: Linker<ReplState>,
    memory: Memory,
    table: Table,
    /// `chasm.refs`: references on the memory data stack, by slot index.
    refs: Table,
    pub session: Session,
}

impl NativeRepl {
    pub fn new(config: Config, prelude: bool) -> Result<Self, String> {
        let e = |e: wasmtime::Error| e.to_string();
        let engine = crate::native::engine()?;
        let mut store = Store::new(
            &engine,
            ReplState {
                host: NativeHost::new(config),
            },
        );
        let memory =
            Memory::new(&mut store, MemoryType::new(L::INITIAL_PAGES as u32, None)).map_err(e)?;
        let table = Table::new(
            &mut store,
            TableType::new(RefType::FUNCREF, 0, None),
            Ref::Func(None),
        )
        .map_err(e)?;
        let refs = Table::new(
            &mut store,
            TableType::new(RefType::ANYREF, 0, None),
            Ref::Any(None),
        )
        .map_err(e)?;
        memory
            .write(
                &mut store,
                L::HEAP_PTR as usize,
                &L::LITERALS_BASE.to_le_bytes(),
            )
            .map_err(|e| e.to_string())?;
        memory
            .write(
                &mut store,
                L::DATA_STACK_PTR as usize,
                &L::DATA_STACK_BASE.to_le_bytes(),
            )
            .map_err(|e| e.to_string())?;
        let mut linker: Linker<ReplState> = Linker::new(&engine);
        linker
            .define(&store, L::IMPORT_MODULE, L::IMPORT_MEMORY, memory)
            .map_err(e)?;
        linker
            .define(&store, L::IMPORT_MODULE, L::IMPORT_TABLE, table)
            .map_err(e)?;
        linker
            .define(&store, L::IMPORT_MODULE, L::IMPORT_REFS, refs)
            .map_err(e)?;
        linker
            .func_wrap(
                L::IMPORT_MODULE,
                L::IMPORT_RING_ENTER,
                move |mut caller: Caller<'_, ReplState>| {
                    let (data, state) = memory.data_and_store_mut(&mut caller);
                    service_ring(data, &mut state.host);
                },
            )
            .map_err(e)?;
        let (session, step) = Session::new(prelude, false, L::LITERALS_BASE);
        let mut repl = NativeRepl {
            engine,
            store,
            linker,
            memory,
            table,
            refs,
            session,
        };
        if !step.ok() {
            let msgs: Vec<String> = step.diagnostics.iter().map(Diagnostic::render).collect();
            return Err(msgs.join("\n"));
        }
        repl.install(&step)?;
        Ok(repl)
    }

    /// The host namespace, e.g. for captured console output.
    pub fn host(&self) -> &NativeHost {
        &self.store.data().host
    }

    pub fn host_mut(&mut self) -> &mut NativeHost {
        &mut self.store.data_mut().host
    }

    fn read_u32(&self, addr: u32) -> u32 {
        let d = self.memory.data(&self.store);
        let a = addr as usize;
        u32::from_le_bytes(d[a..a + 4].try_into().unwrap())
    }

    fn write_u32(&mut self, addr: u32, v: u32) {
        let a = addr as usize;
        self.memory.data_mut(&mut self.store)[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn install(&mut self, step: &Step) -> Result<(), String> {
        let e = |e: wasmtime::Error| e.to_string();
        let end = step.literal_addr as u64 + step.literal_bytes.len() as u64;
        let size = self.memory.data_size(&self.store) as u64;
        if end > size {
            let pages = (end - size).div_ceil(L::PAGE as u64);
            self.memory.grow(&mut self.store, pages).map_err(e)?;
        }
        self.memory
            .write(
                &mut self.store,
                step.literal_addr as usize,
                &step.literal_bytes,
            )
            .map_err(|e| e.to_string())?;
        self.write_u32(L::HEAP_PTR, (end as u32 + 7) & !7);
        let have = self.table.size(&self.store);
        if have < step.table_size as u64 {
            self.table
                .grow(
                    &mut self.store,
                    step.table_size as u64 - have,
                    Ref::Func(None),
                )
                .map_err(e)?;
        }
        let have = self.refs.size(&self.store);
        if have < step.refs_size as u64 {
            self.refs
                .grow(
                    &mut self.store,
                    step.refs_size as u64 - have,
                    Ref::Any(None),
                )
                .map_err(e)?;
        }
        if let Some(bytes) = &step.module {
            let module = Module::new(&self.engine, bytes).map_err(e)?;
            let instance = self
                .linker
                .instantiate(&mut self.store, &module)
                .map_err(e)?;
            for i in &step.installs {
                let f = instance
                    .get_func(&mut self.store, &i.export)
                    .ok_or_else(|| format!("step module has no export `{}`", i.export))?;
                self.table
                    .set(&mut self.store, i.slot as u64, Ref::Func(Some(f)))
                    .map_err(e)?;
            }
        }
        Ok(())
    }

    /// Clear the trap cells so the next trap is reported cleanly.
    fn clear_trap(&mut self) {
        self.write_u32(L::TRAP_MSG_LEN, 0);
    }

    /// Call the function in table `slot` with no arguments.
    fn call_slot(&mut self, slot: u32, results: &mut [wasmtime::Val]) -> Result<(), RunError> {
        let f = match self.table.get(&mut self.store, slot as u64) {
            Some(Ref::Func(Some(f))) => f,
            _ => {
                return Err(RunError {
                    message: format!("no function in table slot {slot}"),
                    word: None,
                })
            }
        };
        match f.call(&mut self.store, &[], results) {
            Ok(()) => Ok(()),
            Err(e) => {
                let err = describe(&e, self.memory.data(&self.store));
                self.clear_trap();
                Err(err)
            }
        }
    }

    /// Compile, install and run one chunk of input.
    pub fn step(&mut self, text: &str) -> Outcome {
        let heap = self.read_u32(L::HEAP_PTR);
        let t0 = Instant::now();
        let step = self.session.step(text, heap);
        let t1 = Instant::now();
        let mut diagnostics = step.diagnostics.clone();
        if let Err(m) = self.install(&step) {
            diagnostics.push(Diagnostic::error(
                chasm_core::diag::codes::E_INTERNAL,
                m,
                Location::default(),
            ));
        }
        let t2 = Instant::now();
        let mut trap = None;
        let installed = !diagnostics.iter().any(Diagnostic::is_error);
        let mut tests = Vec::new();
        if installed {
            for t in &step.tests {
                let n = t.result_types.iter().map(|t| t.width() as usize).sum();
                let mut vals = vec![wasmtime::Val::I32(0); n];
                let (status, actual, error) = match self.call_slot(t.slot, &mut vals) {
                    Ok(()) => {
                        let actual = values(&t.result_types, &vals, self.memory.data(&self.store));
                        let pass = actual.len() == t.expected.len()
                            && actual.iter().zip(&t.expected).all(|(a, b)| same(a, b));
                        let status = if pass {
                            TestStatus::Pass
                        } else {
                            TestStatus::Fail
                        };
                        (status, Some(actual), None)
                    }
                    Err(e) => (TestStatus::Fail, None, Some(e)),
                };
                tests.push(ReplTestResult {
                    word: t.word.clone(),
                    status,
                    expected: t.expected.clone(),
                    actual,
                    error,
                    location: t.location.clone(),
                });
            }
        }
        if let (true, Some(line)) = (installed, &step.line) {
            let sp = self.read_u32(L::DATA_STACK_PTR);
            let saved =
                self.memory.data(&self.store)[L::DATA_STACK_BASE as usize..sp as usize].to_vec();
            match self.call_slot(line.slot, &mut []) {
                Ok(()) => self.session.stack = line.stack_after.clone(),
                Err(e) => {
                    trap = Some(e);
                    let base = L::DATA_STACK_BASE as usize;
                    self.memory.data_mut(&mut self.store)[base..base + saved.len()]
                        .copy_from_slice(&saved);
                    self.write_u32(L::DATA_STACK_PTR, sp);
                }
            }
        }
        let t3 = Instant::now();
        let us = |a: Instant, b: Instant| (b - a).as_micros() as u64;
        Outcome {
            diagnostics,
            defined: step.defined,
            forgotten: step.forgotten,
            forced: step.forced,
            rechecked: step.rechecked,
            listing: step.listing,
            trap,
            tests,
            stack: {
                let structs = self.render_structs();
                read_stack(
                    self.memory.data(&self.store),
                    &self.session.stack,
                    &mut |i, _| structs.get(&i).cloned().unwrap_or(Value::Null),
                )
            },
            timing: Timing {
                compile_us: us(t0, t1),
                instantiate_us: us(t1, t2),
                run_us: us(t2, t3),
            },
        }
    }

    /// The struct and union values on the stack, by slot index, read from
    /// `chasm.refs`.
    fn render_structs(&mut self) -> HashMap<u32, Value> {
        let mut slots = Vec::new();
        let mut i = 0;
        for t in &self.session.stack {
            if let Ty::Struct(..) = t {
                slots.push((i, t.to_string()));
            }
            i += t.width();
        }
        let mut out = HashMap::new();
        let (refs, memory, table) = (self.refs, self.memory, self.table);
        let session = &self.session;
        let mut scope = RootScope::new(&mut self.store);
        let mut r = Reader {
            session,
            memory,
            table,
        };
        for (i, name) in slots {
            let v = match refs.get(&mut scope, i as u64) {
                Some(Ref::Any(Some(a))) => r.value(&mut scope, &a, &name, 0),
                _ => Value::Null,
            };
            out.insert(i, v);
        }
        out
    }
}

/// Below this nesting depth a struct shows its fields; at it, `name{...}`.
const MAX_DEPTH: u32 = 3;

/// Reads struct and union values out of the store for the stack echo.
struct Reader<'a> {
    session: &'a Session,
    memory: Memory,
    table: Table,
}

impl Reader<'_> {
    /// The value of type `ty` (display name) that `r` references.
    fn value(
        &mut self,
        scope: &mut RootScope<&mut Store<ReplState>>,
        r: &Rooted<AnyRef>,
        ty: &str,
        depth: u32,
    ) -> Value {
        let short = ty.split(' ').next().unwrap_or(ty);
        if depth >= MAX_DEPTH {
            return Value::Opaque(format!("{short}{{...}}"));
        }
        match self.session.layout_of(ty) {
            Some(Layout::Struct { fields, .. }) => self.fields(scope, r, short, &fields, depth),
            Some(Layout::Union {
                name,
                tag_slot,
                variants,
            }) => {
                let mut res = [Val::I32(-1)];
                let tag = match self.table.get(&mut *scope, tag_slot as u64) {
                    Some(Ref::Func(Some(f))) => f
                        .call(&mut *scope, &[Val::AnyRef(Some(*r))], &mut res)
                        .ok()
                        .and_then(|()| res[0].i32()),
                    _ => None,
                };
                match tag.and_then(|t| variants.get(t as usize)) {
                    Some((v, fields, _)) => {
                        self.fields(scope, r, &format!("{name}.{v}"), fields, depth)
                    }
                    None => Value::Opaque(format!("<{ty}>")),
                }
            }
            None => Value::Opaque(format!("<{ty}>")),
        }
    }

    /// A struct (or union variant) named `name` with these fields.
    fn fields(
        &mut self,
        scope: &mut RootScope<&mut Store<ReplState>>,
        r: &Rooted<AnyRef>,
        name: &str,
        fields: &[(String, Ty)],
        depth: u32,
    ) -> Value {
        let Ok(Some(s)) = r.as_struct(&*scope) else {
            return Value::Opaque(format!("<{name}>"));
        };
        let get = |scope: &mut RootScope<&mut Store<ReplState>>, k: u32| -> Option<Val> {
            s.field(scope, k as usize).ok()
        };
        let int = |v: Option<Val>| v.and_then(|v| v.i32()).unwrap_or(0);
        let mut out = Vec::new();
        let mut w = 0;
        for (f, t) in fields {
            let v = match t {
                Ty::I32 | Ty::Var(_) | Ty::Param(_) => Value::I32(int(get(scope, w))),
                Ty::I64 => Value::I64(get(scope, w).and_then(|v| v.i64()).unwrap_or(0)),
                Ty::F32 => Value::F32(get(scope, w).and_then(|v| v.f32()).unwrap_or(0.0)),
                Ty::F64 => Value::F64(get(scope, w).and_then(|v| v.f64()).unwrap_or(0.0)),
                Ty::Str => {
                    let a = int(get(scope, w)) as u32 as usize;
                    let n = int(get(scope, w + 1)) as u32 as usize;
                    let data = self.memory.data(scope.as_context());
                    let bytes = data.get(a..a.saturating_add(n)).unwrap_or(&[]);
                    Value::Str(String::from_utf8_lossy(bytes).into_owned())
                }
                Ty::Array(e) => {
                    let at = if matches!(e.as_ref(), Ty::Struct(..)) {
                        w + 2
                    } else {
                        w + 1
                    };
                    Value::Opaque(format!("<{} elements>", int(get(scope, at)) as u32))
                }
                Ty::Quot(_) => Value::Opaque(format!("#{}", int(get(scope, w)))),
                Ty::Struct(..) => match get(scope, w) {
                    Some(Val::AnyRef(Some(a))) => self.value(scope, &a, &t.to_string(), depth + 1),
                    _ => Value::Null,
                },
            };
            out.push((f.clone(), v));
            w += t.width();
        }
        Value::Struct {
            name: name.to_string(),
            fields: out,
        }
    }
}
