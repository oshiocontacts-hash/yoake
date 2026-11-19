//! Expression property tests: deterministic token soup (no panic, bounded cost) and a differential test against a
//! reference evaluator.

use horizon_mobmodel::expr::{compile, ExprErrorKind, ExprInputs, Query, Symbols, MAX_SOURCE_CHARS};
use horizon_mobmodel::limits::MAX_EXPR_INSTRUCTIONS;
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

const TOKENS: &[&str] = &[
    "1", "0", "2.5", ".5", "100000000", "1e30", "+", "-", "*", "/", "%", "(", ")", ",", "?", ":", "!", "<", "<=", ">",
    ">=", "==", "!=", "&&", "||", "math.sin", "math.cos", "math.clamp", "math.lerp", "math.random", "math.pow",
    "math.sqrt", "math.min", "math.max", "math.abs", "q.health", "query.ground_speed", "v.a", "variable.b", "q.synced",
    "query.synced('mode')", "'x'", "true", "false", ".", ";", "=", "{", "}", "\u{e9}", "$", "math.nope", "temp.x",
    "q.anim_time", "m.floor", "v.", "query.", "  ", "->", "??",
];

fn inputs(rng: &mut StdRng) -> ExprInputs {
    let mut i = ExprInputs::default();
    for v in i.query.iter_mut() {
        let pick: [f32; 7] = [0.0, 1.0, -1.0, 1e30, f32::NAN, f32::INFINITY, rng.random_range(-100.0..100.0)];
        *v = pick[rng.random_range(0..7usize)];
    }
    i.synced[0] = rng.random_range(-5.0..5.0);
    i.seed = rng.random();
    i
}

#[test]
fn token_soup_never_panics_and_cost_is_bounded() {
    let mut rng = StdRng::seed_from_u64(0xC0FFEE);
    let (mut ok, mut rejected) = (0, 0);
    for _ in 0..20_000 {
        let n = rng.random_range(0..40usize);
        let src: String = (0..n).map(|_| TOKENS[rng.random_range(0..TOKENS.len())]).collect::<Vec<_>>().join(" ");
        let mut syms = Symbols::new(&["mode"]);
        match compile(&src, &mut syms) {
            Ok(e) => {
                ok += 1;
                assert!(!e.is_empty() && e.len() <= MAX_EXPR_INSTRUCTIONS, "{src}");
                for _ in 0..3 {
                    let inp = inputs(&mut rng);
                    let (v, steps) = e.run(&inp);
                    assert!(v.is_finite(), "{src}");
                    assert!(steps <= MAX_EXPR_INSTRUCTIONS && steps <= e.len(), "{src}");
                }
            }
            Err(err) => {
                rejected += 1;
                assert!(err.column >= 1 && err.column <= src.chars().count() + 1, "{src}: {err}");
            }
        }
    }
    assert!(ok > 50 && rejected > 1000, "ok {ok}, rejected {rejected}"); // the soup exercises both paths
}

#[test]
fn raw_character_soup_never_panics() {
    let mut rng = StdRng::seed_from_u64(7);
    let alphabet: Vec<char> = "0123456789.+-*/%()?:!<>=&|,;'\" abcxyzqmvathrndo_\u{e9}\u{3042}\n\t{}[]".chars().collect();
    for _ in 0..20_000 {
        let n = rng.random_range(0..200usize);
        let src: String = (0..n).map(|_| alphabet[rng.random_range(0..alphabet.len())]).collect();
        let mut syms = Symbols::new(&["mode"]);
        if let Ok(e) = compile(&src, &mut syms) {
            assert!(e.eval(&inputs(&mut rng)).is_finite());
        }
    }
}

#[test]
fn compile_rejects_over_256_instructions() {
    let mut syms = Symbols::new(&[]);
    // n operands and n-1 operators = 2n - 1 instructions; q.health is not foldable.
    let ops = |n: usize| vec!["q.health"; n].join(" + ");
    assert_eq!(compile(&ops(128), &mut syms).unwrap().len(), 255);
    assert_eq!(compile(&ops(129), &mut syms).unwrap_err().kind, ExprErrorKind::Limit);
    // exactly 256: 255 + one unary minus
    assert_eq!(compile(&format!("-({})", ops(128)), &mut syms).unwrap().len(), 256);
    assert!(compile(&format!("-(-({}))", ops(128)), &mut syms).is_err());
    assert_eq!(compile(&"1+".repeat(MAX_SOURCE_CHARS), &mut syms).unwrap_err().kind, ExprErrorKind::Limit);
}

// ---- differential test --------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Ast {
    Num(i32),
    Health,
    Speed,
    Neg(Box<Ast>),
    Not(Box<Ast>),
    Bin(&'static str, Box<Ast>, Box<Ast>),
    Tern(Box<Ast>, Box<Ast>, Box<Ast>),
    Call(&'static str, Vec<Ast>),
}

fn make_ast(rng: &mut StdRng, depth: u32) -> Ast {
    if depth == 0 || rng.random_range(0..5) == 0 {
        return match rng.random_range(0..4) {
            0 => Ast::Health,
            1 => Ast::Speed,
            _ => Ast::Num(rng.random_range(-9..10)),
        };
    }
    let d = depth - 1;
    match rng.random_range(0..9) {
        0 => Ast::Neg(Box::new(make_ast(rng, d))),
        1 => Ast::Not(Box::new(make_ast(rng, d))),
        2 => Ast::Tern(Box::new(make_ast(rng, d)), Box::new(make_ast(rng, d)), Box::new(make_ast(rng, d))),
        3 => {
            let f = ["math.abs", "math.floor", "math.ceil", "math.trunc", "math.sign", "math.round"][rng.random_range(0..6usize)];
            Ast::Call(f, vec![make_ast(rng, d)])
        }
        4 => {
            let f = ["math.min", "math.max", "math.mod"][rng.random_range(0..3usize)];
            Ast::Call(f, vec![make_ast(rng, d), make_ast(rng, d)])
        }
        5 => Ast::Call("math.clamp", vec![make_ast(rng, d), make_ast(rng, d), make_ast(rng, d)]),
        _ => {
            let o = ["+", "-", "*", "/", "%", "<", "<=", ">", ">=", "==", "!=", "&&", "||"][rng.random_range(0..13usize)];
            Ast::Bin(o, Box::new(make_ast(rng, d)), Box::new(make_ast(rng, d)))
        }
    }
}

fn show(a: &Ast) -> String {
    match a {
        Ast::Num(n) => format!("({n})"),
        Ast::Health => "q.health".into(),
        Ast::Speed => "q.ground_speed".into(),
        Ast::Neg(x) => format!("(-{})", show(x)),
        Ast::Not(x) => format!("(!{})", show(x)),
        Ast::Bin(o, l, r) => format!("({} {o} {})", show(l), show(r)),
        Ast::Tern(c, a, b) => format!("({} ? {} : {})", show(c), show(a), show(b)),
        Ast::Call(f, args) => format!("{f}({})", args.iter().map(show).collect::<Vec<_>>().join(", ")),
    }
}

fn fin(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn reference(a: &Ast, h: f32, s: f32) -> f32 {
    let b = |x: bool| if x { 1.0 } else { 0.0 };
    match a {
        Ast::Num(n) => *n as f32,
        Ast::Health => h,
        Ast::Speed => s,
        Ast::Neg(x) => fin(-reference(x, h, s)),
        Ast::Not(x) => b(reference(x, h, s) == 0.0),
        Ast::Tern(c, x, y) => {
            let (c, x, y) = (reference(c, h, s), reference(x, h, s), reference(y, h, s));
            if c != 0.0 {
                x
            } else {
                y
            }
        }
        Ast::Call(f, args) => {
            let v: Vec<f32> = args.iter().map(|a| reference(a, h, s)).collect();
            fin(match *f {
                "math.abs" => v[0].abs(),
                "math.floor" => v[0].floor(),
                "math.ceil" => v[0].ceil(),
                "math.trunc" => v[0].trunc(),
                "math.round" => v[0].round(),
                "math.sign" => {
                    if v[0] > 0.0 {
                        1.0
                    } else if v[0] < 0.0 {
                        -1.0
                    } else {
                        0.0
                    }
                }
                "math.min" => v[0].min(v[1]),
                "math.max" => v[0].max(v[1]),
                "math.mod" => {
                    if v[1] == 0.0 {
                        0.0
                    } else {
                        v[0] % v[1]
                    }
                }
                "math.clamp" => v[0].max(v[1]).min(v[2]),
                _ => unreachable!(),
            })
        }
        Ast::Bin(o, l, r) => {
            let (l, r) = (reference(l, h, s), reference(r, h, s));
            match *o {
                "+" => fin(l + r),
                "-" => fin(l - r),
                "*" => fin(l * r),
                "/" => {
                    if r == 0.0 {
                        0.0
                    } else {
                        fin(l / r)
                    }
                }
                "%" => {
                    if r == 0.0 {
                        0.0
                    } else {
                        fin(l % r)
                    }
                }
                "<" => b(l < r),
                "<=" => b(l <= r),
                ">" => b(l > r),
                ">=" => b(l >= r),
                "==" => b(l == r),
                "!=" => b(l != r),
                "&&" => b(l != 0.0 && r != 0.0),
                "||" => b(l != 0.0 || r != 0.0),
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn compiled_vm_matches_a_reference_evaluator() {
    let mut rng = StdRng::seed_from_u64(1234);
    let mut compiled = 0;
    for _ in 0..4000 {
        let ast = make_ast(&mut rng, 5);
        let src = show(&ast);
        let mut syms = Symbols::new(&[]);
        let Ok(e) = compile(&src, &mut syms) else { continue }; // too long is allowed to be rejected
        compiled += 1;
        for _ in 0..4 {
            let h = rng.random_range(-20..20) as f32 * 0.5;
            let s = rng.random_range(-4..5) as f32 * 0.25;
            let mut inp = ExprInputs::default();
            inp.set(Query::Health, h);
            inp.set(Query::GroundSpeed, s);
            let want = reference(&ast, h, s);
            let got = e.eval(&inp);
            assert!(got == want || (got - want).abs() <= want.abs() * 1e-6, "{src} h={h} s={s}: {got} vs {want}");
        }
    }
    assert!(compiled > 3000, "{compiled}");
}
