//! The numeric primitive set: exactly the wasm numeric and memory
//! instructions, under their wasm names.

use crate::types::Ty;
use wasm_encoder::{Instruction as I, MemArg};

fn mem(align: u32) -> MemArg {
    MemArg {
        offset: 0,
        align,
        memory_index: 0,
    }
}

/// Look up a numeric/memory instruction by its wasm name.
pub fn numeric(name: &str) -> Option<(Vec<Ty>, Vec<Ty>, I<'static>)> {
    use Ty::{F32, F64, I32, I64};
    let un = |t: Ty, i| Some((vec![t.clone()], vec![t], i));
    let bin = |t: Ty, i| Some((vec![t.clone(), t.clone()], vec![t], i));
    let test = |t: Ty, i| Some((vec![t], vec![I32], i));
    let cmp = |t: Ty, i| Some((vec![t.clone(), t], vec![I32], i));
    let cvt = |a: Ty, b: Ty, i| Some((vec![a], vec![b], i));
    let load = |t: Ty, i| Some((vec![I32], vec![t], i));
    let store = |t: Ty, i| Some((vec![I32, t], vec![], i));
    match name {
        // i32
        "i32.eqz" => test(I32, I::I32Eqz),
        "i32.eq" => cmp(I32, I::I32Eq),
        "i32.ne" => cmp(I32, I::I32Ne),
        "i32.lt_s" => cmp(I32, I::I32LtS),
        "i32.lt_u" => cmp(I32, I::I32LtU),
        "i32.gt_s" => cmp(I32, I::I32GtS),
        "i32.gt_u" => cmp(I32, I::I32GtU),
        "i32.le_s" => cmp(I32, I::I32LeS),
        "i32.le_u" => cmp(I32, I::I32LeU),
        "i32.ge_s" => cmp(I32, I::I32GeS),
        "i32.ge_u" => cmp(I32, I::I32GeU),
        "i32.clz" => un(I32, I::I32Clz),
        "i32.ctz" => un(I32, I::I32Ctz),
        "i32.popcnt" => un(I32, I::I32Popcnt),
        "i32.add" => bin(I32, I::I32Add),
        "i32.sub" => bin(I32, I::I32Sub),
        "i32.mul" => bin(I32, I::I32Mul),
        "i32.div_s" => bin(I32, I::I32DivS),
        "i32.div_u" => bin(I32, I::I32DivU),
        "i32.rem_s" => bin(I32, I::I32RemS),
        "i32.rem_u" => bin(I32, I::I32RemU),
        "i32.and" => bin(I32, I::I32And),
        "i32.or" => bin(I32, I::I32Or),
        "i32.xor" => bin(I32, I::I32Xor),
        "i32.shl" => bin(I32, I::I32Shl),
        "i32.shr_s" => bin(I32, I::I32ShrS),
        "i32.shr_u" => bin(I32, I::I32ShrU),
        "i32.rotl" => bin(I32, I::I32Rotl),
        "i32.rotr" => bin(I32, I::I32Rotr),
        "i32.extend8_s" => un(I32, I::I32Extend8S),
        "i32.extend16_s" => un(I32, I::I32Extend16S),
        // i64
        "i64.eqz" => test(I64, I::I64Eqz),
        "i64.eq" => cmp(I64, I::I64Eq),
        "i64.ne" => cmp(I64, I::I64Ne),
        "i64.lt_s" => cmp(I64, I::I64LtS),
        "i64.lt_u" => cmp(I64, I::I64LtU),
        "i64.gt_s" => cmp(I64, I::I64GtS),
        "i64.gt_u" => cmp(I64, I::I64GtU),
        "i64.le_s" => cmp(I64, I::I64LeS),
        "i64.le_u" => cmp(I64, I::I64LeU),
        "i64.ge_s" => cmp(I64, I::I64GeS),
        "i64.ge_u" => cmp(I64, I::I64GeU),
        "i64.clz" => un(I64, I::I64Clz),
        "i64.ctz" => un(I64, I::I64Ctz),
        "i64.popcnt" => un(I64, I::I64Popcnt),
        "i64.add" => bin(I64, I::I64Add),
        "i64.sub" => bin(I64, I::I64Sub),
        "i64.mul" => bin(I64, I::I64Mul),
        "i64.div_s" => bin(I64, I::I64DivS),
        "i64.div_u" => bin(I64, I::I64DivU),
        "i64.rem_s" => bin(I64, I::I64RemS),
        "i64.rem_u" => bin(I64, I::I64RemU),
        "i64.and" => bin(I64, I::I64And),
        "i64.or" => bin(I64, I::I64Or),
        "i64.xor" => bin(I64, I::I64Xor),
        "i64.shl" => bin(I64, I::I64Shl),
        "i64.shr_s" => bin(I64, I::I64ShrS),
        "i64.shr_u" => bin(I64, I::I64ShrU),
        "i64.rotl" => bin(I64, I::I64Rotl),
        "i64.rotr" => bin(I64, I::I64Rotr),
        "i64.extend8_s" => un(I64, I::I64Extend8S),
        "i64.extend16_s" => un(I64, I::I64Extend16S),
        "i64.extend32_s" => un(I64, I::I64Extend32S),
        // f32
        "f32.eq" => cmp(F32, I::F32Eq),
        "f32.ne" => cmp(F32, I::F32Ne),
        "f32.lt" => cmp(F32, I::F32Lt),
        "f32.gt" => cmp(F32, I::F32Gt),
        "f32.le" => cmp(F32, I::F32Le),
        "f32.ge" => cmp(F32, I::F32Ge),
        "f32.abs" => un(F32, I::F32Abs),
        "f32.neg" => un(F32, I::F32Neg),
        "f32.ceil" => un(F32, I::F32Ceil),
        "f32.floor" => un(F32, I::F32Floor),
        "f32.trunc" => un(F32, I::F32Trunc),
        "f32.nearest" => un(F32, I::F32Nearest),
        "f32.sqrt" => un(F32, I::F32Sqrt),
        "f32.add" => bin(F32, I::F32Add),
        "f32.sub" => bin(F32, I::F32Sub),
        "f32.mul" => bin(F32, I::F32Mul),
        "f32.div" => bin(F32, I::F32Div),
        "f32.min" => bin(F32, I::F32Min),
        "f32.max" => bin(F32, I::F32Max),
        "f32.copysign" => bin(F32, I::F32Copysign),
        // f64
        "f64.eq" => cmp(F64, I::F64Eq),
        "f64.ne" => cmp(F64, I::F64Ne),
        "f64.lt" => cmp(F64, I::F64Lt),
        "f64.gt" => cmp(F64, I::F64Gt),
        "f64.le" => cmp(F64, I::F64Le),
        "f64.ge" => cmp(F64, I::F64Ge),
        "f64.abs" => un(F64, I::F64Abs),
        "f64.neg" => un(F64, I::F64Neg),
        "f64.ceil" => un(F64, I::F64Ceil),
        "f64.floor" => un(F64, I::F64Floor),
        "f64.trunc" => un(F64, I::F64Trunc),
        "f64.nearest" => un(F64, I::F64Nearest),
        "f64.sqrt" => un(F64, I::F64Sqrt),
        "f64.add" => bin(F64, I::F64Add),
        "f64.sub" => bin(F64, I::F64Sub),
        "f64.mul" => bin(F64, I::F64Mul),
        "f64.div" => bin(F64, I::F64Div),
        "f64.min" => bin(F64, I::F64Min),
        "f64.max" => bin(F64, I::F64Max),
        "f64.copysign" => bin(F64, I::F64Copysign),
        // conversions
        "i32.wrap_i64" => cvt(I64, I32, I::I32WrapI64),
        "i32.trunc_f32_s" => cvt(F32, I32, I::I32TruncF32S),
        "i32.trunc_f32_u" => cvt(F32, I32, I::I32TruncF32U),
        "i32.trunc_f64_s" => cvt(F64, I32, I::I32TruncF64S),
        "i32.trunc_f64_u" => cvt(F64, I32, I::I32TruncF64U),
        "i32.trunc_sat_f32_s" => cvt(F32, I32, I::I32TruncSatF32S),
        "i32.trunc_sat_f32_u" => cvt(F32, I32, I::I32TruncSatF32U),
        "i32.trunc_sat_f64_s" => cvt(F64, I32, I::I32TruncSatF64S),
        "i32.trunc_sat_f64_u" => cvt(F64, I32, I::I32TruncSatF64U),
        "i32.reinterpret_f32" => cvt(F32, I32, I::I32ReinterpretF32),
        "i64.extend_i32_s" => cvt(I32, I64, I::I64ExtendI32S),
        "i64.extend_i32_u" => cvt(I32, I64, I::I64ExtendI32U),
        "i64.trunc_f32_s" => cvt(F32, I64, I::I64TruncF32S),
        "i64.trunc_f32_u" => cvt(F32, I64, I::I64TruncF32U),
        "i64.trunc_f64_s" => cvt(F64, I64, I::I64TruncF64S),
        "i64.trunc_f64_u" => cvt(F64, I64, I::I64TruncF64U),
        "i64.trunc_sat_f32_s" => cvt(F32, I64, I::I64TruncSatF32S),
        "i64.trunc_sat_f32_u" => cvt(F32, I64, I::I64TruncSatF32U),
        "i64.trunc_sat_f64_s" => cvt(F64, I64, I::I64TruncSatF64S),
        "i64.trunc_sat_f64_u" => cvt(F64, I64, I::I64TruncSatF64U),
        "i64.reinterpret_f64" => cvt(F64, I64, I::I64ReinterpretF64),
        "f32.convert_i32_s" => cvt(I32, F32, I::F32ConvertI32S),
        "f32.convert_i32_u" => cvt(I32, F32, I::F32ConvertI32U),
        "f32.convert_i64_s" => cvt(I64, F32, I::F32ConvertI64S),
        "f32.convert_i64_u" => cvt(I64, F32, I::F32ConvertI64U),
        "f32.demote_f64" => cvt(F64, F32, I::F32DemoteF64),
        "f32.reinterpret_i32" => cvt(I32, F32, I::F32ReinterpretI32),
        "f64.convert_i32_s" => cvt(I32, F64, I::F64ConvertI32S),
        "f64.convert_i32_u" => cvt(I32, F64, I::F64ConvertI32U),
        "f64.convert_i64_s" => cvt(I64, F64, I::F64ConvertI64S),
        "f64.convert_i64_u" => cvt(I64, F64, I::F64ConvertI64U),
        "f64.promote_f32" => cvt(F32, F64, I::F64PromoteF32),
        "f64.reinterpret_i64" => cvt(I64, F64, I::F64ReinterpretI64),
        // loads (natural alignment, offset 0)
        "i32.load" => load(I32, I::I32Load(mem(2))),
        "i64.load" => load(I64, I::I64Load(mem(3))),
        "f32.load" => load(F32, I::F32Load(mem(2))),
        "f64.load" => load(F64, I::F64Load(mem(3))),
        "i32.load8_s" => load(I32, I::I32Load8S(mem(0))),
        "i32.load8_u" => load(I32, I::I32Load8U(mem(0))),
        "i32.load16_s" => load(I32, I::I32Load16S(mem(1))),
        "i32.load16_u" => load(I32, I::I32Load16U(mem(1))),
        "i64.load8_s" => load(I64, I::I64Load8S(mem(0))),
        "i64.load8_u" => load(I64, I::I64Load8U(mem(0))),
        "i64.load16_s" => load(I64, I::I64Load16S(mem(1))),
        "i64.load16_u" => load(I64, I::I64Load16U(mem(1))),
        "i64.load32_s" => load(I64, I::I64Load32S(mem(2))),
        "i64.load32_u" => load(I64, I::I64Load32U(mem(2))),
        // stores: ( addr value -- )
        "i32.store" => store(I32, I::I32Store(mem(2))),
        "i64.store" => store(I64, I::I64Store(mem(3))),
        "f32.store" => store(F32, I::F32Store(mem(2))),
        "f64.store" => store(F64, I::F64Store(mem(3))),
        "i32.store8" => store(I32, I::I32Store8(mem(0))),
        "i32.store16" => store(I32, I::I32Store16(mem(1))),
        "i64.store8" => store(I64, I::I64Store8(mem(0))),
        "i64.store16" => store(I64, I::I64Store16(mem(1))),
        "i64.store32" => store(I64, I::I64Store32(mem(2))),
        // memory
        "memory.size" => Some((vec![], vec![I32], I::MemorySize(0))),
        "memory.grow" => Some((vec![I32], vec![I32], I::MemoryGrow(0))),
        "memory.copy" => Some((
            vec![I32, I32, I32],
            vec![],
            I::MemoryCopy {
                src_mem: 0,
                dst_mem: 0,
            },
        )),
        "memory.fill" => Some((vec![I32, I32, I32], vec![], I::MemoryFill(0))),
        _ => None,
    }
}

/// Shuffle primitives: (inputs, output permutation as input indices).
pub fn shuffle(name: &str) -> Option<(usize, &'static [usize])> {
    Some(match name {
        "dup" => (1, &[0, 0]),
        "drop" => (1, &[]),
        "swap" => (2, &[1, 0]),
        "over" => (2, &[0, 1, 0]),
        "nip" => (2, &[1]),
        "tuck" => (2, &[1, 0, 1]),
        "rot" => (3, &[1, 2, 0]),
        "-rot" => (3, &[2, 0, 1]),
        "2dup" => (2, &[0, 1, 0, 1]),
        "2drop" => (2, &[]),
        _ => return None,
    })
}

/// Non-numeric primitives with fixed effects and custom code.
pub fn special(name: &str) -> Option<(Vec<Ty>, Vec<Ty>)> {
    use Ty::{Str, I32};
    Some(match name {
        "str.len" => (vec![Str], vec![I32]),
        "str.addr" => (vec![Str], vec![I32]),
        "str.from-raw" => (vec![I32, I32], vec![Str]),
        "mem.alloc" => (vec![I32], vec![I32]),
        "trap" => (vec![Str], vec![]),
        "host.open" => (vec![Str, I32], vec![I32]),
        "host.read" | "host.write" => (vec![I32, I32, I32], vec![I32]),
        "host.close" => (vec![I32], vec![I32]),
        _ => return None,
    })
}

/// Every numeric primitive's name, for suggestions.
const NUMERIC_NAMES: &[&str] = &[
    "f32.abs",
    "f32.add",
    "f32.ceil",
    "f32.convert_i32_s",
    "f32.convert_i32_u",
    "f32.convert_i64_s",
    "f32.convert_i64_u",
    "f32.copysign",
    "f32.demote_f64",
    "f32.div",
    "f32.eq",
    "f32.floor",
    "f32.ge",
    "f32.gt",
    "f32.le",
    "f32.load",
    "f32.lt",
    "f32.max",
    "f32.min",
    "f32.mul",
    "f32.ne",
    "f32.nearest",
    "f32.neg",
    "f32.reinterpret_i32",
    "f32.sqrt",
    "f32.store",
    "f32.sub",
    "f32.trunc",
    "f64.abs",
    "f64.add",
    "f64.ceil",
    "f64.convert_i32_s",
    "f64.convert_i32_u",
    "f64.convert_i64_s",
    "f64.convert_i64_u",
    "f64.copysign",
    "f64.div",
    "f64.eq",
    "f64.floor",
    "f64.ge",
    "f64.gt",
    "f64.le",
    "f64.load",
    "f64.lt",
    "f64.max",
    "f64.min",
    "f64.mul",
    "f64.ne",
    "f64.nearest",
    "f64.neg",
    "f64.promote_f32",
    "f64.reinterpret_i64",
    "f64.sqrt",
    "f64.store",
    "f64.sub",
    "f64.trunc",
    "i32.add",
    "i32.and",
    "i32.clz",
    "i32.ctz",
    "i32.div_s",
    "i32.div_u",
    "i32.eq",
    "i32.eqz",
    "i32.extend16_s",
    "i32.extend8_s",
    "i32.ge_s",
    "i32.ge_u",
    "i32.gt_s",
    "i32.gt_u",
    "i32.le_s",
    "i32.le_u",
    "i32.load",
    "i32.load16_s",
    "i32.load16_u",
    "i32.load8_s",
    "i32.load8_u",
    "i32.lt_s",
    "i32.lt_u",
    "i32.mul",
    "i32.ne",
    "i32.or",
    "i32.popcnt",
    "i32.reinterpret_f32",
    "i32.rem_s",
    "i32.rem_u",
    "i32.rotl",
    "i32.rotr",
    "i32.shl",
    "i32.shr_s",
    "i32.shr_u",
    "i32.store",
    "i32.store16",
    "i32.store8",
    "i32.sub",
    "i32.trunc_f32_s",
    "i32.trunc_f32_u",
    "i32.trunc_f64_s",
    "i32.trunc_f64_u",
    "i32.trunc_sat_f32_s",
    "i32.trunc_sat_f32_u",
    "i32.trunc_sat_f64_s",
    "i32.trunc_sat_f64_u",
    "i32.wrap_i64",
    "i32.xor",
    "i64.add",
    "i64.and",
    "i64.clz",
    "i64.ctz",
    "i64.div_s",
    "i64.div_u",
    "i64.eq",
    "i64.eqz",
    "i64.extend16_s",
    "i64.extend32_s",
    "i64.extend8_s",
    "i64.extend_i32_s",
    "i64.extend_i32_u",
    "i64.ge_s",
    "i64.ge_u",
    "i64.gt_s",
    "i64.gt_u",
    "i64.le_s",
    "i64.le_u",
    "i64.load",
    "i64.load16_s",
    "i64.load16_u",
    "i64.load32_s",
    "i64.load32_u",
    "i64.load8_s",
    "i64.load8_u",
    "i64.lt_s",
    "i64.lt_u",
    "i64.mul",
    "i64.ne",
    "i64.or",
    "i64.popcnt",
    "i64.reinterpret_f64",
    "i64.rem_s",
    "i64.rem_u",
    "i64.rotl",
    "i64.rotr",
    "i64.shl",
    "i64.shr_s",
    "i64.shr_u",
    "i64.store",
    "i64.store16",
    "i64.store32",
    "i64.store8",
    "i64.sub",
    "i64.trunc_f32_s",
    "i64.trunc_f32_u",
    "i64.trunc_f64_s",
    "i64.trunc_f64_u",
    "i64.trunc_sat_f32_s",
    "i64.trunc_sat_f32_u",
    "i64.trunc_sat_f64_s",
    "i64.trunc_sat_f64_u",
    "i64.xor",
];

/// Every primitive's name.
pub fn names() -> impl Iterator<Item = &'static str> {
    const OTHER: &[&str] = &[
        "dup",
        "drop",
        "swap",
        "over",
        "nip",
        "tuck",
        "rot",
        "-rot",
        "2dup",
        "2drop",
        "str.len",
        "str.addr",
        "str.from-raw",
        "mem.alloc",
        "trap",
        "host.open",
        "host.read",
        "host.write",
        "host.close",
        "array.new",
        "array.len",
        "array.at",
        "array.at!",
        "array.slice",
        "call",
        "leave",
        "if",
        "while",
        "until",
        "when",
        "unless",
        "times",
        "each",
        "map",
        "filter",
        "fold",
        "match",
    ];
    NUMERIC_NAMES.iter().chain(OTHER).copied()
}

/// Names from other languages and the Chasm words that do the job.
const ALIASES: &[(&str, &str)] = &[
    ("pop", "`drop`"),
    ("len", "`str.len` or `array.len`"),
    ("length", "`str.len` or `array.len`"),
    ("size", "`str.len` or `array.len`"),
    ("count", "`str.len` or `array.len`"),
    ("concat", "`str.concat`"),
    ("append", "`str.concat`"),
    ("emit", "`print`"),
    ("puts", "`println`"),
    (".", "`i32.to-str println`"),
    (".s", ""),
    ("not", "`i32.eqz`"),
    ("mod", "`i32.rem_s`"),
    ("%", "`i32.rem_s`"),
    ("+", "`i32.add`"),
    ("-", "`i32.sub`"),
    ("*", "`i32.mul`"),
    ("/", "`i32.div_s`"),
    ("=", "`i32.eq`"),
    ("==", "`i32.eq`"),
    ("<>", "`i32.ne`"),
    ("!=", "`i32.ne`"),
    ("<", "`i32.lt_s`"),
    (">", "`i32.gt_s`"),
    ("<=", "`i32.le_s`"),
    (">=", "`i32.ge_s`"),
    ("1+", "`1 i32.add`"),
    ("1-", "`1 i32.sub`"),
];

/// "; did you mean ...?" for an unknown word: a known alias from another
/// language, else the nearest primitive or dictionary name by edit distance.
pub fn suggest<'a>(name: &str, dictionary: impl Iterator<Item = &'a str>) -> String {
    if let Some((_, s)) = ALIASES.iter().find(|(a, _)| *a == name) {
        return if s.is_empty() {
            "; there is no stack-printing word: the REPL prints the stack after every line"
                .to_string()
        } else {
            format!("; did you mean {s}?")
        };
    }
    let limit = if name.chars().count() >= 8 { 2 } else { 1 };
    let mut best: Vec<&str> = Vec::new();
    let mut best_d = limit + 1;
    let static_to_a = |n: &'static str| -> &'a str { n };
    for cand in names().map(static_to_a).chain(dictionary) {
        if cand.starts_with('[') || cand == name {
            continue;
        }
        let d = distance(name, cand);
        if d < best_d {
            best_d = d;
            best.clear();
        }
        if d == best_d && !best.contains(&cand) {
            best.push(cand);
        }
    }
    best.sort();
    best.truncate(3);
    match best.as_slice() {
        [] => String::new(),
        [one] => format!("; did you mean `{one}`?"),
        many => format!(
            "; did you mean {}?",
            many.iter()
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(" or ")
        ),
    }
}

/// Levenshtein distance counting an adjacent swap as one edit.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Polymorphic array primitives and `call` are handled by the checker directly.
pub fn is_builtin(name: &str) -> bool {
    numeric(name).is_some()
        || shuffle(name).is_some()
        || special(name).is_some()
        || matches!(
            name,
            "array.new"
                | "array.len"
                | "array.at"
                | "array.at!"
                | "array.slice"
                | "call"
                | "leave"
                | "match"
        )
        || crate::parser::combinator_arity(name).is_some()
}

#[cfg(test)]
mod suggest_tests {
    use super::*;

    #[test]
    fn every_listed_name_is_a_primitive() {
        for n in names() {
            assert!(is_builtin(n), "{n}");
        }
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("pop", std::iter::empty()), "; did you mean `drop`?");
        assert_eq!(suggest("dupp", std::iter::empty()), "; did you mean `dup`?");
        assert_eq!(
            suggest("i32.ad", std::iter::empty()),
            "; did you mean `i32.add` or `i32.and`?"
        );
        assert_eq!(
            suggest("sqaure", ["square"].into_iter()),
            "; did you mean `square`?"
        );
        assert_eq!(suggest("zzzzzzzz", std::iter::empty()), "");
        assert_eq!(suggest("total", std::iter::empty()), "");
        assert!(suggest(".s", std::iter::empty()).contains("prints the stack"));
    }
}
