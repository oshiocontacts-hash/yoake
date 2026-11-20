//! Robustness tests for the expression compiler (`src/expr.rs`).

use horizon_mobmodel::expr::{compile, ExprError, ExprErrorKind, Symbols, MAX_NESTING, MAX_SOURCE_CHARS, MAX_VARIABLES};
use horizon_mobmodel::limits::{MAX_EXPR_INSTRUCTIONS, MAX_SYNCED};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

const PEAK_CAP: usize = 64 * 1024 * 1024;

struct PeakAlloc;

thread_local! {
    static ALLOC_CUR: Cell<usize> = const { Cell::new(0) };
    static ALLOC_PEAK: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for PeakAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let _ = ALLOC_CUR.try_with(|c| {
            let next = c.get().saturating_add(l.size());
            c.set(next);
            let _ = ALLOC_PEAK.try_with(|p| if next > p.get() { p.set(next) });
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, l: Layout) {
        let _ = ALLOC_CUR.try_with(|c| c.set(c.get().saturating_sub(l.size())));
        unsafe { System.dealloc(_ptr, l) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, l: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOC_CUR.try_with(|c| {
            let next = c.get().saturating_sub(l.size()).saturating_add(new_size);
            c.set(next);
            let _ = ALLOC_PEAK.try_with(|p| if next > p.get() { p.set(next) });
        });
        unsafe { System.realloc(ptr, l, new_size) }
    }
}

#[global_allocator]
static GLOBAL: PeakAlloc = PeakAlloc;

fn within_cap<T>(f: impl FnOnce() -> T) -> T {
    let base = ALLOC_CUR.with(Cell::get);
    ALLOC_PEAK.with(|p| p.set(base));
    let out = f();
    let peak = ALLOC_PEAK.with(Cell::get).saturating_sub(base);
    assert!(peak <= PEAK_CAP, "peak allocation {peak} exceeded {PEAK_CAP}");
    out
}

fn assert_err_col(r: Result<horizon_mobmodel::expr::Expr, ExprError>, src: &str, want: Option<ExprErrorKind>) {
    let e = r.unwrap_err();
    let max_col = src.chars().count() + 1;
    assert!(e.column >= 1 && e.column <= max_col, "{src:?}: bad column {} (max {max_col}): {e}", e.column);
    assert!(!e.message.is_empty(), "{src:?}: empty message");
    if let Some(k) = want {
        assert_eq!(e.kind, k, "{src:?}: {e}");
    }
}

#[test]
fn truncated_expressions_carry_valid_columns() {
    let samples = [
        "math.clamp(q.health + 1.5, 0, 20)",
        "query.synced('state') == 'sit' ? math.sin(q.anim_time * 90) : -v.offset",
        "!(q.is_on_ground && (v.a > 0 || m.pi <= 4))",
        "math.lerp(10, 20, math.random(0, 1))",
        "'idle' != q.synced('state')",
    ];
    for full in samples {
        for (idx, _) in full.char_indices() {
            let prefix = &full[..idx];
            let mut syms = Symbols::new(&["state"]);
            syms.set_enum_values("state", &["idle", "sit"]);
            if let Err(e) = within_cap(|| compile(prefix, &mut syms)) {
                let max_col = prefix.chars().count() + 1;
                assert!(e.column >= 1 && e.column <= max_col, "{prefix:?}: {e}");
                assert!(!e.message.is_empty());
            }
        }
    }
    for incomplete in ["1 +", "(1", "math.", "math.sin(", "math.min(1,", "1 ?", "1 &&", "q.synced(", "q.synced('s", "v."] {
        let mut syms = Symbols::new(&["s"]);
        assert_err_col(within_cap(|| compile(incomplete, &mut syms)), incomplete, None);
    }
}

#[test]
fn deep_nesting_past_limit_returns_limit_with_column() {
    let mut syms = Symbols::new(&[]);
    let cases = [
        format!("{}1{}", "(".repeat(MAX_NESTING + 10), ")".repeat(MAX_NESTING + 10)),
        format!("{}1{}", "(".repeat(600), ")".repeat(600)),
        format!("{}q.health", "-".repeat(600)),
        format!("{}q.health", "!".repeat(600)),
        format!("{}q.health", "+".repeat(600)),
        format!("{}0", "1 ? 2 : ".repeat(200)),
        format!("{}1{}", "math.abs(".repeat(200), ")".repeat(200)),
    ];
    for src in &cases {
        assert_err_col(within_cap(|| compile(src, &mut syms)), src, Some(ExprErrorKind::Limit));
    }
}

#[test]
fn huge_and_malformed_numbers_and_string_misuse() {
    let mut syms = Symbols::new(&["state", "num"]);
    syms.set_enum_values("state", &["idle", "sit"]);

    for bad_num in ["1e400", "-1e400", "1e39", "1e999999", "1e+", "1e-", "1.2.3", "0x10", ".e1", "..", "NaN", "Infinity"] {
        assert_err_col(within_cap(|| compile(bad_num, &mut syms)), bad_num, None);
    }

    for bad_str in [
        "'1' + 2",
        "math.sin('0')",
        "1 < 'sit'",
        "'sit' + 'idle'",
        "'sit' ? 1 : 2",
        "q.synced('state') == 1 ? 'sit' : 'idle'",
        "'unterminated",
        "\"unterminated",
    ] {
        assert_err_col(within_cap(|| compile(bad_str, &mut syms)), bad_str, None);
    }

    assert_err_col(within_cap(|| compile("q.synced('state') == 'unknown'", &mut syms)), "q.synced('state') == 'unknown'", Some(ExprErrorKind::UnknownEnumValue));
    assert_err_col(within_cap(|| compile("q.synced('num') == 'sit'", &mut syms)), "q.synced('num') == 'sit'", Some(ExprErrorKind::UnknownEnumValue));
    assert_err_col(within_cap(|| compile("q.synced('missing') == 0", &mut syms)), "q.synced('missing') == 0", Some(ExprErrorKind::UnknownSynced));
}

#[test]
fn limits_empty_utf8_garbage_and_random_inputs() {
    let mut syms = Symbols::new(&[]);
    for bad in ["", "   \t\n", "\u{FEFF}1", "\u{FEFF}", "\0", "🦀 + 1", "あ + 1", "1; 2", "1 = 2", "1 ?? 2", "a->b"] {
        assert_err_col(within_cap(|| compile(bad, &mut syms)), bad, None);
    }

    let huge_src = "1 + ".repeat(MAX_SOURCE_CHARS);
    assert_err_col(within_cap(|| compile(&huge_src, &mut syms)), &huge_src, Some(ExprErrorKind::Limit));
    let utf8_huge = "あ".repeat(MAX_SOURCE_CHARS + 500);
    assert_err_col(within_cap(|| compile(&utf8_huge, &mut syms)), &utf8_huge, Some(ExprErrorKind::Limit));

    // > MAX_EXPR_INSTRUCTIONS (256): 129 non-constant additions = 257 instructions.
    let over_ops = vec!["q.health"; (MAX_EXPR_INSTRUCTIONS / 2) + 1].join(" + ");
    assert_err_col(within_cap(|| compile(&over_ops, &mut syms)), &over_ops, Some(ExprErrorKind::Limit));

    // > MAX_VARIABLES (64).
    let mut v_syms = Symbols::new(&[]);
    for i in 0..MAX_VARIABLES {
        assert!(within_cap(|| compile(&format!("v.x{i}"), &mut v_syms)).is_ok());
    }
    assert_err_col(within_cap(|| compile("v.overflow", &mut v_syms)), "v.overflow", Some(ExprErrorKind::Limit));

    // > MAX_SYNCED (32) truncates at MAX_SYNCED.
    let synced_names: Vec<String> = (0..=MAX_SYNCED).map(|i| format!("s{i}")).collect();
    let refs: Vec<&str> = synced_names.iter().map(String::as_str).collect();
    let mut s_syms = Symbols::new(&refs);
    assert!(within_cap(|| compile("q.synced('s31')", &mut s_syms)).is_ok());
    assert_err_col(within_cap(|| compile("q.synced('s32')", &mut s_syms)), "q.synced('s32')", Some(ExprErrorKind::UnknownSynced));

    // Random UTF-8/ASCII soup up to 2500 chars.
    let mut rng = StdRng::seed_from_u64(0xE891);
    let alpha: Vec<char> = "0123456789.+-*/%()?:!<>=&|,'\" qvm.e\t\n\u{3042}\u{1F98A}\0$".chars().collect();
    for _ in 0..2000 {
        let len = rng.random_range(0..2500);
        let src: String = (0..len).map(|_| alpha[rng.random_range(0..alpha.len())]).collect();
        let mut rsyms = Symbols::new(&["state"]);
        if let Err(e) = within_cap(|| compile(&src, &mut rsyms)) {
            let max_col = src.chars().count() + 1;
            assert!(e.column >= 1 && e.column <= max_col, "bad col {}: {e}", e.column);
            assert!(!e.message.is_empty());
        }
    }
}
