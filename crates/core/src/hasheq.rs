//! The Chasm source of the generated `eq` and `hash` words for types that are
//! not compared inline: `str`, arrays, structs and unions. Each is a body
//! for the effect `( t t -- i32 )` or `( t -- i32 )`, built from primitives,
//! the type's generated readers and `eq`/`hash` on its parts.

use crate::check::{Ctx, Op};
use crate::types::Ty;

/// Seed and multiplier of the fold over a value's parts.
const SEED: i32 = 17;

/// The body of `eq<t>` or `hash<t>`.
pub fn helper_source(ctx: &Ctx, op: Op, t: &Ty) -> String {
    match (op, t) {
        (Op::Eq, Ty::Str) => "\
:> b :> a
a str.len b str.len i32.eq :> r!
r [ a str.len [ :> i
  a str.addr i i32.add i32.load8_u  b str.addr i i32.add i32.load8_u  i32.ne
  [ 0 r! leave ] when ] times ] when
r"
        .into(),
        // FNV-1a over the bytes.
        (Op::Hash, Ty::Str) => "\
:> s
0x811C9DC5 :> h!
s str.len [ :> i  h  s str.addr i i32.add i32.load8_u  i32.xor  0x01000193 i32.mul h! ] times
h"
        .into(),
        (Op::Eq, Ty::Array(_)) => "\
:> b :> a
a array.len b array.len i32.eq :> r!
r [ a array.len [ :> i  a i array.at b i array.at eq i32.eqz [ 0 r! leave ] when ] times ] when
r"
        .into(),
        (Op::Hash, Ty::Array(_)) => "\
:> a
a array.len :> h!
a [ hash :> e  h 31 i32.mul e i32.add h! ] each
h"
        .into(),
        (op, Ty::Struct(name, _)) => {
            if let Some(&k) = ctx.struct_by_name.get(name) {
                let fields: Vec<String> = ctx.structs[k]
                    .fields
                    .iter()
                    .map(|(f, _)| format!("{name}.{f}"))
                    .collect();
                struct_source(op, &fields)
            } else {
                let u = &ctx.unions[ctx.union_by_name[name]];
                let variants: Vec<Vec<String>> = u
                    .variants
                    .iter()
                    .map(|(v, fields)| {
                        fields
                            .iter()
                            .map(|(f, _)| format!("{name}.{v}.{f}"))
                            .collect()
                    })
                    .collect();
                union_source(op, name, &variants)
            }
        }
        _ => unreachable!("{t} is compared inline"),
    }
}

/// A struct: `eq` is the `and` of its fields' `eq`, `hash` folds their hashes.
fn struct_source(op: Op, readers: &[String]) -> String {
    match op {
        Op::Eq => {
            let mut s = String::from(":> b :> a\n1");
            for r in readers {
                s.push_str(&format!("\na {r} b {r} eq i32.and"));
            }
            s
        }
        Op::Hash => {
            let mut s = format!(":> a\n{SEED}");
            for r in readers {
                s.push_str(&format!("\n31 i32.mul a {r} hash i32.add"));
            }
            s
        }
    }
}

/// A union: the tags first, then the fields of the variant both hold.
fn union_source(op: Op, name: &str, variants: &[Vec<String>]) -> String {
    let mut s = String::new();
    match op {
        Op::Eq => {
            s.push_str(&format!(
                ":> b :> a\na {name}.tag b {name}.tag i32.eq :> r!\nr [ a {name}.tag :> t\n"
            ));
            for (k, readers) in variants.iter().enumerate() {
                if readers.is_empty() {
                    continue;
                }
                s.push_str(&format!("  t {k} i32.eq [ 1"));
                for r in readers {
                    s.push_str(&format!("  a {r} b {r} eq i32.and"));
                }
                s.push_str(" r! ] when\n");
            }
            s.push_str("] when\nr");
        }
        Op::Hash => {
            s.push_str(&format!(
                ":> a\na {name}.tag :> t\nt {SEED} i32.add :> h!\n"
            ));
            for (k, readers) in variants.iter().enumerate() {
                if readers.is_empty() {
                    continue;
                }
                s.push_str(&format!("t {k} i32.eq [ h"));
                for r in readers {
                    s.push_str(&format!("  31 i32.mul a {r} hash i32.add"));
                }
                s.push_str(" h! ] when\n");
            }
            s.push('h');
        }
    }
    s
}
