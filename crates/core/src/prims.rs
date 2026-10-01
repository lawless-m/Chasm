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

/// Polymorphic array primitives and `call` are handled by the checker directly.
pub fn is_builtin(name: &str) -> bool {
    numeric(name).is_some()
        || shuffle(name).is_some()
        || special(name).is_some()
        || matches!(
            name,
            "array.new" | "array.len" | "array.at" | "array.at!" | "array.slice" | "call" | "leave"
        )
        || crate::parser::combinator_arity(name).is_some()
}
