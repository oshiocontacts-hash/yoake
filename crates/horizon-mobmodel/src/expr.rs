//! The safe expression language (FR-F4-MOBS.md section 3.6): a small Molang-like subset, compiled to flat bytecode.
//!
//! Grammar (lowest to highest precedence; names are case-insensitive):
//!
//! ```text
//! expr    := or ( '?' expr ( ':' expr )? )?          a ? b : c   (a ? b  means  a ? b : 0; right-associative)
//! or      := and ( '||' and )*
//! and     := equal ( '&&' equal )*
//! equal   := rel ( ('==' | '!=') rel )*
//! rel     := add ( ('<' | '<=' | '>' | '>=') add )*
//! add     := mul ( ('+' | '-') mul )*
//! mul     := unary ( ('*' | '/' | '%') unary )*
//! unary   := ('-' | '+' | '!') unary | primary
//! primary := number | 'true' | 'false' | '(' expr ')' | math.NAME '(' args ')' | math.pi
//!          | query.NAME | query.synced '(' string ')' | variable.NAME
//! enum    := query.synced '(' string ')' ('==' | '!=') string     (and the string first)
//! ```
//!
//! `q.`, `v.` and `m.` are aliases of `query.`, `variable.` and `math.`. A single trailing `;` is accepted.
//!
//! Semantics:
//! * Every value is an `f32`. Comparisons and logic give 1.0 or 0.0; any non-zero value is true.
//! * `x / 0` and `x % 0` are 0. Every operation result that is NaN or infinite becomes 0 (inputs are sanitised the
//!   same way), so the VM only ever sees and returns finite numbers.
//! * `%` and `math.mod` are the truncated remainder (the sign follows the left operand).
//! * `math.sin`, `cos`, `tan` take degrees; `asin`, `acos`, `atan`, `atan2` return degrees.
//! * `math.random(lo, hi)` is a pure function of the per-mob seed, a caller-chosen epoch, the expression text and the
//!   position of the call: the same on server and client, stable within a frame, and the caller advances the epoch
//!   to get new numbers.
//! * Both branches of a ternary are always evaluated (there are no jumps; cost is exactly the instruction count).
//!
//! Enum names: after `Symbols::set_enum_values`, `query.synced('state') == 'sit'` (also `!=`, also string first) compiles to a
//! comparison with the value's index (its position in the list given), so scripts and JSON can say what they mean. An
//! unknown value name is [`ExprErrorKind::UnknownEnumValue`]. Strings anywhere else are still unsupported.
//!
//! Unsupported Molang constructs (unknown functions or queries, assignment, strings other than the synced name,
//! `loop`, `return`, structs, several statements) give [`ExprErrorKind::Unsupported`]; loaders turn those into
//! warnings. Everything else that is wrong (bad syntax, unknown synced name, too long) is a hard error.

use crate::limits::{MAX_EXPR_INSTRUCTIONS, MAX_SYNCED};
use std::fmt;

/// Per-mob `variable.*` slots.
pub const MAX_VARIABLES: usize = 64;
/// Longest accepted source text, in characters.
pub const MAX_SOURCE_CHARS: usize = 2048;
/// Deepest nesting (parentheses, unary chains, ternary branches) the parser accepts.
pub const MAX_NESTING: usize = 48;

/// Read-only per-mob inputs, `query.NAME`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    AnimTime,
    LifeTime,
    GroundSpeed,
    VerticalSpeed,
    YawSpeed,
    HeadXRotation,
    HeadYRotation,
    BodyYRotation,
    Health,
    MaxHealth,
    IsOnGround,
    IsInWater,
    IsBaby,
    HurtTime,
    DeathTime,
    IsRiding,
    HasTarget,
    TimeOfDay,
}

impl Query {
    pub const ALL: [Query; 18] = [
        Query::AnimTime,
        Query::LifeTime,
        Query::GroundSpeed,
        Query::VerticalSpeed,
        Query::YawSpeed,
        Query::HeadXRotation,
        Query::HeadYRotation,
        Query::BodyYRotation,
        Query::Health,
        Query::MaxHealth,
        Query::IsOnGround,
        Query::IsInWater,
        Query::IsBaby,
        Query::HurtTime,
        Query::DeathTime,
        Query::IsRiding,
        Query::HasTarget,
        Query::TimeOfDay,
    ];
    pub const COUNT: usize = 18;

    pub fn name(self) -> &'static str {
        match self {
            Query::AnimTime => "anim_time",
            Query::LifeTime => "life_time",
            Query::GroundSpeed => "ground_speed",
            Query::VerticalSpeed => "vertical_speed",
            Query::YawSpeed => "yaw_speed",
            Query::HeadXRotation => "head_x_rotation",
            Query::HeadYRotation => "head_y_rotation",
            Query::BodyYRotation => "body_y_rotation",
            Query::Health => "health",
            Query::MaxHealth => "max_health",
            Query::IsOnGround => "is_on_ground",
            Query::IsInWater => "is_in_water",
            Query::IsBaby => "is_baby",
            Query::HurtTime => "hurt_time",
            Query::DeathTime => "death_time",
            Query::IsRiding => "is_riding",
            Query::HasTarget => "has_target",
            Query::TimeOfDay => "time_of_day",
        }
    }

    /// `name` must already be lower case.
    pub fn from_name(name: &str) -> Option<Query> {
        Query::ALL.iter().copied().find(|q| q.name() == name)
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Everything an expression can read. The caller fills it per mob and frame; the VM never writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ExprInputs {
    pub query: [f32; Query::COUNT],
    /// Synced values as floats (bool 0/1, enum index, i32 and f32 as is), by slot.
    pub synced: [f32; MAX_SYNCED],
    pub variables: [f32; MAX_VARIABLES],
    /// The per-mob seed for `math.random`.
    pub seed: u32,
    /// Advance this to get fresh random numbers (for example each game tick).
    pub random_epoch: u32,
}

impl Default for ExprInputs {
    fn default() -> Self {
        Self {
            query: [0.0; Query::COUNT],
            synced: [0.0; MAX_SYNCED],
            variables: [0.0; MAX_VARIABLES],
            seed: 0,
            random_epoch: 0,
        }
    }
}

impl ExprInputs {
    pub fn set(&mut self, q: Query, v: f32) {
        self.query[q.index()] = v;
    }
    pub fn get(&self, q: Query) -> f32 {
        self.query[q.index()]
    }
}

/// Names the compiler resolves at compile time: the type's synced values and the interned `variable.*` names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Symbols {
    synced: Vec<String>,
    /// Per synced slot: the names of an enum's values, in index order (empty for non-enums).
    enums: Vec<Vec<String>>,
    variables: Vec<String>,
}

impl Symbols {
    /// `synced_names` are the type's tracked values in slot order (at most [`MAX_SYNCED`]); names are matched
    /// case-insensitively.
    pub fn new(synced_names: &[&str]) -> Self {
        Self {
            synced: synced_names.iter().take(MAX_SYNCED).map(|s| s.to_ascii_lowercase()).collect(),
            enums: vec![Vec::new(); synced_names.len().min(MAX_SYNCED)],
            variables: Vec::new(),
        }
    }

    /// Declare that the synced value `synced_name` is an enum with these value names (index = position). Afterwards
    /// `query.synced('name') == 'value'` (or `!=`, either operand order) compiles to a comparison with the index.
    /// Returns false if the synced name is unknown.
    pub fn set_enum_values(&mut self, synced_name: &str, values: &[&str]) -> bool {
        match self.synced_slot(synced_name) {
            Some(slot) => {
                self.enums[slot] = values.iter().map(|v| v.to_ascii_lowercase()).collect();
                true
            }
            None => false,
        }
    }

    pub fn synced_slot(&self, name: &str) -> Option<usize> {
        let n = name.to_ascii_lowercase();
        self.synced.iter().position(|s| *s == n)
    }

    pub fn variable_slot(&self, name: &str) -> Option<usize> {
        let n = name.to_ascii_lowercase();
        self.variables.iter().position(|s| *s == n)
    }

    /// Interned `variable.*` names in slot order.
    pub fn variables(&self) -> &[String] {
        &self.variables
    }

    fn intern_variable(&mut self, name: &str) -> Option<usize> {
        if let Some(i) = self.variable_slot(name) {
            return Some(i);
        }
        if self.variables.len() >= MAX_VARIABLES {
            return None;
        }
        self.variables.push(name.to_ascii_lowercase());
        Some(self.variables.len() - 1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExprErrorKind {
    /// Malformed text.
    Syntax,
    /// A Molang construct this subset does not have (loaders warn and use 0).
    Unsupported,
    /// `query.synced('x')` with an undeclared name.
    UnknownSynced,
    /// Too many instructions, too long, too deep, too many variables.
    Limit,
    /// `query.synced('state') == 'nope'`: the enum has no such value (or the synced value is not an enum).
    UnknownEnumValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExprError {
    pub kind: ExprErrorKind,
    /// 1-based character column in the source.
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "column {}: {}", self.column, self.message)
    }
}

impl std::error::Error for ExprError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fn1 {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Abs,
    Floor,
    Ceil,
    Round,
    Sqrt,
    Trunc,
    Sign,
    Exp,
    Ln,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fn2 {
    Min,
    Max,
    Pow,
    Mod,
    Atan2,
    Random,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fn3 {
    Clamp,
    Lerp,
}

/// One bytecode instruction (stack machine, no jumps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Op {
    Push(f32),
    Query(Query),
    Synced(u8),
    Var(u8),
    Neg,
    Not,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Or,
    /// Pops else, then, condition.
    Select,
    F1(Fn1),
    F2(Fn2),
    F3(Fn3),
}

impl Op {
    /// Values popped.
    fn pops(self) -> usize {
        match self {
            Op::Push(_) | Op::Query(_) | Op::Synced(_) | Op::Var(_) => 0,
            Op::Neg | Op::Not | Op::F1(_) => 1,
            Op::Select | Op::F3(_) => 3,
            _ => 2,
        }
    }
}

/// A compiled expression. Fields are private: every `Expr` was built by the compiler, so the VM's stack discipline
/// holds by construction.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    ops: Vec<Op>,
    max_depth: u16,
    salt: u32,
}

#[inline]
fn fin(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[inline]
fn truth(x: f32) -> bool {
    x != 0.0
}

#[inline]
fn bool_f(b: bool) -> f32 {
    if b {
        1.0
    } else {
        0.0
    }
}

fn apply1(op: Op, a: f32) -> f32 {
    match op {
        Op::Neg => fin(-a),
        Op::Not => bool_f(!truth(a)),
        Op::F1(f) => fin(match f {
            Fn1::Sin => a.to_radians().sin(),
            Fn1::Cos => a.to_radians().cos(),
            Fn1::Tan => a.to_radians().tan(),
            Fn1::Asin => a.asin().to_degrees(),
            Fn1::Acos => a.acos().to_degrees(),
            Fn1::Atan => a.atan().to_degrees(),
            Fn1::Abs => a.abs(),
            Fn1::Floor => a.floor(),
            Fn1::Ceil => a.ceil(),
            Fn1::Round => a.round(),
            Fn1::Sqrt => a.sqrt(),
            Fn1::Trunc => a.trunc(),
            Fn1::Sign => {
                if a > 0.0 {
                    1.0
                } else if a < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            Fn1::Exp => a.exp(),
            Fn1::Ln => a.ln(),
        }),
        _ => 0.0,
    }
}

fn apply2(op: Op, a: f32, b: f32) -> f32 {
    match op {
        Op::Add => fin(a + b),
        Op::Sub => fin(a - b),
        Op::Mul => fin(a * b),
        Op::Div => {
            if b == 0.0 {
                0.0
            } else {
                fin(a / b)
            }
        }
        Op::Rem | Op::F2(Fn2::Mod) => {
            if b == 0.0 {
                0.0
            } else {
                fin(a % b)
            }
        }
        Op::Lt => bool_f(a < b),
        Op::Le => bool_f(a <= b),
        Op::Gt => bool_f(a > b),
        Op::Ge => bool_f(a >= b),
        Op::Eq => bool_f(a == b),
        Op::Ne => bool_f(a != b),
        Op::And => bool_f(truth(a) && truth(b)),
        Op::Or => bool_f(truth(a) || truth(b)),
        Op::F2(Fn2::Min) => a.min(b),
        Op::F2(Fn2::Max) => a.max(b),
        Op::F2(Fn2::Pow) => fin(a.powf(b)),
        Op::F2(Fn2::Atan2) => fin(a.atan2(b).to_degrees()),
        _ => 0.0,
    }
}

fn apply3(op: Op, a: f32, b: f32, c: f32) -> f32 {
    match op {
        Op::Select => {
            if truth(a) {
                b
            } else {
                c
            }
        }
        Op::F3(Fn3::Clamp) => a.max(b).min(c),
        Op::F3(Fn3::Lerp) => fin(a + (b - a) * c),
        _ => 0.0,
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// A number in `[0, 1)`, a pure function of its arguments.
fn random_unit(seed: u32, epoch: u32, salt: u32, pc: u32) -> f32 {
    let a = splitmix((seed as u64) << 32 | epoch as u64);
    let b = splitmix(a ^ ((salt as u64) << 32 | pc as u64));
    (b >> 40) as f32 / 16_777_216.0
}

fn fnv1a(text: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for b in text.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

impl Expr {
    /// An expression that is the constant `v` (0 if `v` is not finite).
    pub fn constant(v: f32) -> Self {
        Self { ops: vec![Op::Push(fin(v))], max_depth: 1, salt: 0 }
    }

    /// The value if the expression is a single constant.
    pub fn as_constant(&self) -> Option<f32> {
        match self.ops.as_slice() {
            [Op::Push(v)] => Some(*v),
            _ => None,
        }
    }

    /// This expression negated (one more instruction; constants stay constants). `None` if it would exceed the cap.
    pub fn negated(&self) -> Option<Expr> {
        if let Some(v) = self.as_constant() {
            return Some(Expr::constant(-v));
        }
        if self.ops.len() >= MAX_EXPR_INSTRUCTIONS {
            return None;
        }
        let mut e = self.clone();
        e.ops.push(Op::Neg);
        Some(e)
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Instruction count; also the exact cost of one evaluation.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Evaluate. Never panics, never allocates; the result is always finite.
    pub fn eval(&self, inp: &ExprInputs) -> f32 {
        self.run(inp).0
    }

    /// `eval` as a condition: non-zero is true.
    pub fn eval_bool(&self, inp: &ExprInputs) -> bool {
        truth(self.eval(inp))
    }

    /// Evaluate and also return the number of instructions executed (always `len()`).
    pub fn run(&self, inp: &ExprInputs) -> (f32, usize) {
        if let Some(v) = self.as_constant() {
            return (v, 1);
        }
        if self.max_depth <= 24 {
            self.exec::<24>(inp)
        } else {
            self.exec::<MAX_EXPR_INSTRUCTIONS>(inp)
        }
    }

    fn exec<const N: usize>(&self, inp: &ExprInputs) -> (f32, usize) {
        let mut stack = [0.0f32; N];
        let mut sp = 0usize;
        let mut steps = 0usize;
        for (pc, &op) in self.ops.iter().enumerate() {
            steps += 1;
            let value = match op {
                Op::Push(v) => v,
                Op::Query(q) => fin(inp.query.get(q.index()).copied().unwrap_or(0.0)),
                Op::Synced(i) => fin(inp.synced.get(i as usize).copied().unwrap_or(0.0)),
                Op::Var(i) => fin(inp.variables.get(i as usize).copied().unwrap_or(0.0)),
                Op::F2(Fn2::Random) => {
                    let b = pop(&stack, &mut sp);
                    let a = pop(&stack, &mut sp);
                    let u = random_unit(inp.seed, inp.random_epoch, self.salt, pc as u32);
                    fin(a + (b - a) * u)
                }
                other => match other.pops() {
                    1 => {
                        let a = pop(&stack, &mut sp);
                        apply1(other, a)
                    }
                    2 => {
                        let b = pop(&stack, &mut sp);
                        let a = pop(&stack, &mut sp);
                        apply2(other, a, b)
                    }
                    _ => {
                        let c = pop(&stack, &mut sp);
                        let b = pop(&stack, &mut sp);
                        let a = pop(&stack, &mut sp);
                        apply3(other, a, b, c)
                    }
                },
            };
            if sp < N {
                stack[sp] = value;
                sp += 1;
            }
        }
        let result = if sp > 0 { stack[sp - 1] } else { 0.0 };
        (fin(result), steps)
    }
}

#[inline]
fn pop<const N: usize>(stack: &[f32; N], sp: &mut usize) -> f32 {
    *sp = sp.saturating_sub(1);
    stack[(*sp).min(N - 1)]
}

// ---------------------------------------------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum TokKind {
    Num(f32),
    Ident(String),
    Str(String),
    Dot,
    LParen,
    RParen,
    Comma,
    Question,
    Colon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Bang,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Semi,
}

#[derive(Clone, Debug)]
struct Tok {
    kind: TokKind,
    col: usize,
}

fn err<T>(kind: ExprErrorKind, column: usize, message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError { kind, column, message: message.into() })
}

fn tokenize(chars: &[char]) -> Result<Vec<Tok>, ExprError> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let two = |next: char| chars.get(i + 1) == Some(&next);
        let (kind, len) = match c {
            '0'..='9' | '.' if c != '.' || chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()) => {
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '.' {
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }
                if j < chars.len() && (chars[j] == 'e' || chars[j] == 'E') {
                    let mut k = j + 1;
                    if k < chars.len() && (chars[k] == '+' || chars[k] == '-') {
                        k += 1;
                    }
                    if k < chars.len() && chars[k].is_ascii_digit() {
                        while k < chars.len() && chars[k].is_ascii_digit() {
                            k += 1;
                        }
                        j = k;
                    }
                }
                let text: String = chars[i..j].iter().collect();
                match text.parse::<f32>() {
                    Ok(v) if v.is_finite() => (TokKind::Num(v), j - i),
                    _ => return err(ExprErrorKind::Syntax, col, format!("bad number '{text}'")),
                }
            }
            'a'..='z' | 'A'..='Z' | '_' => {
                let mut j = i;
                while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                let text: String = chars[i..j].iter().collect::<String>().to_ascii_lowercase();
                (TokKind::Ident(text), j - i)
            }
            '\'' | '"' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j] != c {
                    j += 1;
                }
                if j >= chars.len() {
                    return err(ExprErrorKind::Syntax, col, "unterminated string");
                }
                (TokKind::Str(chars[i + 1..j].iter().collect()), j + 1 - i)
            }
            '.' => (TokKind::Dot, 1),
            '(' => (TokKind::LParen, 1),
            ')' => (TokKind::RParen, 1),
            ',' => (TokKind::Comma, 1),
            '?' => {
                if two('?') {
                    return err(ExprErrorKind::Unsupported, col, "null coalescing '??' is not supported");
                }
                (TokKind::Question, 1)
            }
            ':' => (TokKind::Colon, 1),
            '+' => (TokKind::Plus, 1),
            '-' => {
                if two('>') {
                    return err(ExprErrorKind::Unsupported, col, "entity access '->' is not supported");
                }
                (TokKind::Minus, 1)
            }
            '*' => (TokKind::Star, 1),
            '/' => (TokKind::Slash, 1),
            '%' => (TokKind::Percent, 1),
            ';' => (TokKind::Semi, 1),
            '!' => {
                if two('=') {
                    (TokKind::Ne, 2)
                } else {
                    (TokKind::Bang, 1)
                }
            }
            '<' => {
                if two('=') {
                    (TokKind::Le, 2)
                } else {
                    (TokKind::Lt, 1)
                }
            }
            '>' => {
                if two('=') {
                    (TokKind::Ge, 2)
                } else {
                    (TokKind::Gt, 1)
                }
            }
            '=' => {
                if two('=') {
                    (TokKind::EqEq, 2)
                } else {
                    return err(ExprErrorKind::Unsupported, col, "assignment '=' is not supported");
                }
            }
            '&' => {
                if two('&') {
                    (TokKind::AndAnd, 2)
                } else {
                    return err(ExprErrorKind::Syntax, col, "expected '&&'");
                }
            }
            '|' => {
                if two('|') {
                    (TokKind::OrOr, 2)
                } else {
                    return err(ExprErrorKind::Syntax, col, "expected '||'");
                }
            }
            '{' | '}' | '[' | ']' => {
                return err(ExprErrorKind::Unsupported, col, format!("'{c}' (blocks and arrays) is not supported"))
            }
            other => return err(ExprErrorKind::Syntax, col, format!("unexpected character '{other}'")),
        };
        toks.push(Tok { kind, col });
        i += len;
    }
    Ok(toks)
}

struct Parser<'a> {
    toks: Vec<Tok>,
    pos: usize,
    end_col: usize,
    ops: Vec<Op>,
    depth: usize,
    syms: &'a mut Symbols,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&TokKind> {
        self.toks.get(self.pos).map(|t| &t.kind)
    }

    fn col(&self) -> usize {
        self.toks.get(self.pos).map_or(self.end_col, |t| t.col)
    }

    fn eat(&mut self, kind: &TokKind) -> bool {
        if self.peek() == Some(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokKind, what: &str) -> Result<(), ExprError> {
        if self.eat(kind) {
            Ok(())
        } else {
            err(ExprErrorKind::Syntax, self.col(), format!("expected {what}"))
        }
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_NESTING {
            return err(ExprErrorKind::Limit, self.col(), format!("nesting deeper than {MAX_NESTING}"));
        }
        Ok(())
    }

    /// Append an instruction, folding it into a constant when all its operands are constants.
    fn emit(&mut self, op: Op) -> Result<(), ExprError> {
        let pops = op.pops();
        let foldable = !matches!(op, Op::F2(Fn2::Random))
            && pops > 0
            && self.ops.len() >= pops
            && self.ops[self.ops.len() - pops..].iter().all(|o| matches!(o, Op::Push(_)));
        if foldable {
            let args: Vec<f32> = self.ops[self.ops.len() - pops..]
                .iter()
                .map(|o| if let Op::Push(v) = o { *v } else { 0.0 })
                .collect();
            let v = match pops {
                1 => apply1(op, args[0]),
                2 => apply2(op, args[0], args[1]),
                _ => apply3(op, args[0], args[1], args[2]),
            };
            let at = self.ops.len() - pops;
            self.ops.truncate(at);
            self.ops.push(Op::Push(v));
            return Ok(());
        }
        if self.ops.len() >= MAX_EXPR_INSTRUCTIONS {
            return err(
                ExprErrorKind::Limit,
                self.col(),
                format!("expression needs more than {MAX_EXPR_INSTRUCTIONS} instructions"),
            );
        }
        self.ops.push(op);
        Ok(())
    }

    fn expr(&mut self) -> Result<(), ExprError> {
        self.enter()?;
        self.or()?;
        if self.eat(&TokKind::Question) {
            self.expr()?;
            if self.eat(&TokKind::Colon) {
                self.expr()?;
            } else {
                self.emit(Op::Push(0.0))?;
            }
            self.emit(Op::Select)?;
        }
        self.depth -= 1;
        Ok(())
    }

    fn or(&mut self) -> Result<(), ExprError> {
        self.and()?;
        while self.eat(&TokKind::OrOr) {
            self.and()?;
            self.emit(Op::Or)?;
        }
        Ok(())
    }

    fn and(&mut self) -> Result<(), ExprError> {
        self.equal()?;
        while self.eat(&TokKind::AndAnd) {
            self.equal()?;
            self.emit(Op::And)?;
        }
        Ok(())
    }

    fn equal(&mut self) -> Result<(), ExprError> {
        let start = self.ops.len();
        // `'sit' == query.synced('state')`: the string comes first.
        if let (Some(TokKind::Str(name)), Some(Tok { kind: TokKind::EqEq | TokKind::Ne, .. })) =
            (self.peek().cloned(), self.toks.get(self.pos + 1))
        {
            let scol = self.col();
            self.pos += 1;
            let op = if matches!(self.peek(), Some(TokKind::EqEq)) { Op::Eq } else { Op::Ne };
            self.pos += 1;
            self.rel()?;
            self.push_enum_value(start, &name, scol)?;
            self.emit(op)?;
        } else {
            self.rel()?;
        }
        loop {
            let op = match self.peek() {
                Some(TokKind::EqEq) => Op::Eq,
                Some(TokKind::Ne) => Op::Ne,
                _ => return Ok(()),
            };
            self.pos += 1;
            let left_synced = (self.ops.len() == start + 1).then(|| self.ops.last().copied()).flatten();
            if let (Some(Op::Synced(_)), Some(TokKind::Str(name))) = (left_synced, self.peek().cloned()) {
                let col = self.col();
                self.pos += 1;
                self.push_enum_value(start, &name, col)?;
            } else {
                self.rel()?;
            }
            self.emit(op)?;
        }
    }

    /// The previous operand (ops from `start`) must be exactly one `Synced` op: push the index of enum value `name`.
    fn push_enum_value(&mut self, start: usize, name: &str, col: usize) -> Result<(), ExprError> {
        let slot = match (self.ops.len() == start + 1, self.ops.last()) {
            (true, Some(Op::Synced(slot))) => *slot as usize,
            _ => {
                return err(
                    ExprErrorKind::Unsupported,
                    col,
                    "strings are only allowed in query.synced('name') and as the other operand of == / != against it",
                )
            }
        };
        let lower = name.to_ascii_lowercase();
        match self.syms.enums.get(slot).and_then(|v| v.iter().position(|x| *x == lower)) {
            Some(i) => self.emit(Op::Push(i as f32)),
            None => {
                let known = self.syms.enums.get(slot).map(|v| v.join(", ")).unwrap_or_default();
                err(
                    ExprErrorKind::UnknownEnumValue,
                    col,
                    format!("unknown enum value '{name}' for synced value '{}' (values: {known})", self.syms.synced[slot]),
                )
            }
        }
    }

    fn rel(&mut self) -> Result<(), ExprError> {
        self.add()?;
        loop {
            let op = match self.peek() {
                Some(TokKind::Lt) => Op::Lt,
                Some(TokKind::Le) => Op::Le,
                Some(TokKind::Gt) => Op::Gt,
                Some(TokKind::Ge) => Op::Ge,
                _ => return Ok(()),
            };
            self.pos += 1;
            self.add()?;
            self.emit(op)?;
        }
    }

    fn add(&mut self) -> Result<(), ExprError> {
        self.mul()?;
        loop {
            let op = match self.peek() {
                Some(TokKind::Plus) => Op::Add,
                Some(TokKind::Minus) => Op::Sub,
                _ => return Ok(()),
            };
            self.pos += 1;
            self.mul()?;
            self.emit(op)?;
        }
    }

    fn mul(&mut self) -> Result<(), ExprError> {
        self.unary()?;
        loop {
            let op = match self.peek() {
                Some(TokKind::Star) => Op::Mul,
                Some(TokKind::Slash) => Op::Div,
                Some(TokKind::Percent) => Op::Rem,
                _ => return Ok(()),
            };
            self.pos += 1;
            self.unary()?;
            self.emit(op)?;
        }
    }

    fn unary(&mut self) -> Result<(), ExprError> {
        match self.peek() {
            Some(TokKind::Minus) => {
                self.pos += 1;
                self.enter()?;
                self.unary()?;
                self.depth -= 1;
                self.emit(Op::Neg)
            }
            Some(TokKind::Plus) => {
                self.pos += 1;
                self.enter()?;
                self.unary()?;
                self.depth -= 1;
                Ok(())
            }
            Some(TokKind::Bang) => {
                self.pos += 1;
                self.enter()?;
                self.unary()?;
                self.depth -= 1;
                self.emit(Op::Not)
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<(), ExprError> {
        let col = self.col();
        let kind = match self.toks.get(self.pos) {
            Some(t) => t.kind.clone(),
            None => return err(ExprErrorKind::Syntax, col, "unexpected end of expression"),
        };
        self.pos += 1;
        match kind {
            TokKind::Num(v) => self.emit(Op::Push(v)),
            TokKind::LParen => {
                self.expr()?;
                self.expect(&TokKind::RParen, "')'")
            }
            TokKind::Ident(name) => self.name(&name, col),
            TokKind::Str(_) => err(ExprErrorKind::Unsupported, col, "strings are only allowed in query.synced('name') and as the other operand of == / != against a synced enum"),
            other => err(ExprErrorKind::Syntax, col, format!("unexpected {}", describe(&other))),
        }
    }

    fn name(&mut self, first: &str, col: usize) -> Result<(), ExprError> {
        match first {
            "true" => return self.emit(Op::Push(1.0)),
            "false" => return self.emit(Op::Push(0.0)),
            _ => {}
        }
        let ns = match first {
            "math" | "m" => "math",
            "query" | "q" => "query",
            "variable" | "v" => "variable",
            other => {
                return err(
                    ExprErrorKind::Unsupported,
                    col,
                    format!("'{other}' is not supported (only math., query., variable. and their aliases)"),
                )
            }
        };
        if !self.eat(&TokKind::Dot) {
            return err(ExprErrorKind::Syntax, self.col(), format!("expected '.' after '{first}'"));
        }
        let ncol = self.col();
        let member = match self.toks.get(self.pos).map(|t| t.kind.clone()) {
            Some(TokKind::Ident(n)) => {
                self.pos += 1;
                n
            }
            _ => return err(ExprErrorKind::Syntax, ncol, format!("expected a name after '{first}.'")),
        };
        if self.peek() == Some(&TokKind::Dot) {
            return err(ExprErrorKind::Unsupported, self.col(), "nested names (structs) are not supported");
        }
        match ns {
            "math" => self.math(&member, ncol),
            "query" => self.query(&member, ncol),
            _ => match self.syms.intern_variable(&member) {
                Some(slot) => self.emit(Op::Var(slot as u8)),
                None => err(ExprErrorKind::Limit, ncol, format!("more than {MAX_VARIABLES} variables")),
            },
        }
    }

    fn query(&mut self, member: &str, col: usize) -> Result<(), ExprError> {
        if member == "synced" {
            self.expect(&TokKind::LParen, "'(' after query.synced")?;
            let scol = self.col();
            let name = match self.toks.get(self.pos).map(|t| t.kind.clone()) {
                Some(TokKind::Str(s)) => {
                    self.pos += 1;
                    s
                }
                _ => return err(ExprErrorKind::Syntax, scol, "query.synced needs a quoted name"),
            };
            self.expect(&TokKind::RParen, "')'")?;
            return match self.syms.synced_slot(&name) {
                Some(slot) => self.emit(Op::Synced(slot as u8)),
                None => err(ExprErrorKind::UnknownSynced, scol, format!("unknown synced value '{name}'")),
            };
        }
        match Query::from_name(member) {
            Some(q) => self.emit(Op::Query(q)),
            None => err(ExprErrorKind::Unsupported, col, format!("query.{member} is not supported")),
        }
    }

    fn math(&mut self, member: &str, col: usize) -> Result<(), ExprError> {
        if member == "pi" {
            return self.emit(Op::Push(std::f32::consts::PI));
        }
        let (op, arity) = match member {
            "sin" => (Op::F1(Fn1::Sin), 1),
            "cos" => (Op::F1(Fn1::Cos), 1),
            "tan" => (Op::F1(Fn1::Tan), 1),
            "asin" => (Op::F1(Fn1::Asin), 1),
            "acos" => (Op::F1(Fn1::Acos), 1),
            "atan" => (Op::F1(Fn1::Atan), 1),
            "abs" => (Op::F1(Fn1::Abs), 1),
            "floor" => (Op::F1(Fn1::Floor), 1),
            "ceil" => (Op::F1(Fn1::Ceil), 1),
            "round" => (Op::F1(Fn1::Round), 1),
            "sqrt" => (Op::F1(Fn1::Sqrt), 1),
            "trunc" => (Op::F1(Fn1::Trunc), 1),
            "sign" => (Op::F1(Fn1::Sign), 1),
            "exp" => (Op::F1(Fn1::Exp), 1),
            "ln" => (Op::F1(Fn1::Ln), 1),
            "min" => (Op::F2(Fn2::Min), 2),
            "max" => (Op::F2(Fn2::Max), 2),
            "pow" => (Op::F2(Fn2::Pow), 2),
            "mod" => (Op::F2(Fn2::Mod), 2),
            "atan2" => (Op::F2(Fn2::Atan2), 2),
            "random" => (Op::F2(Fn2::Random), 2),
            "clamp" => (Op::F3(Fn3::Clamp), 3),
            "lerp" => (Op::F3(Fn3::Lerp), 3),
            _ => return err(ExprErrorKind::Unsupported, col, format!("math.{member} is not supported")),
        };
        self.expect(&TokKind::LParen, &format!("'(' after math.{member}"))?;
        for i in 0..arity {
            if i > 0 {
                self.expect(&TokKind::Comma, &format!("',' (math.{member} takes {arity} arguments)"))?;
            }
            self.expr()?;
        }
        if self.peek() == Some(&TokKind::Comma) {
            return err(ExprErrorKind::Syntax, self.col(), format!("math.{member} takes {arity} arguments"));
        }
        self.expect(&TokKind::RParen, "')'")?;
        self.emit(op)
    }
}

fn describe(k: &TokKind) -> String {
    match k {
        TokKind::Num(_) => "number".into(),
        TokKind::Ident(n) => format!("name '{n}'"),
        TokKind::Str(_) => "string".into(),
        TokKind::Dot => "'.'".into(),
        TokKind::LParen => "'('".into(),
        TokKind::RParen => "')'".into(),
        TokKind::Comma => "','".into(),
        TokKind::Question => "'?'".into(),
        TokKind::Colon => "':'".into(),
        TokKind::Plus => "'+'".into(),
        TokKind::Minus => "'-'".into(),
        TokKind::Star => "'*'".into(),
        TokKind::Slash => "'/'".into(),
        TokKind::Percent => "'%'".into(),
        TokKind::Bang => "'!'".into(),
        TokKind::Lt => "'<'".into(),
        TokKind::Le => "'<='".into(),
        TokKind::Gt => "'>'".into(),
        TokKind::Ge => "'>='".into(),
        TokKind::EqEq => "'=='".into(),
        TokKind::Ne => "'!='".into(),
        TokKind::AndAnd => "'&&'".into(),
        TokKind::OrOr => "'||'".into(),
        TokKind::Semi => "';'".into(),
    }
}

/// Compile `src`. Interns new `variable.*` names into `syms`. Never panics.
pub fn compile(src: &str, syms: &mut Symbols) -> Result<Expr, ExprError> {
    let chars: Vec<char> = src.chars().take(MAX_SOURCE_CHARS + 1).collect();
    if chars.len() > MAX_SOURCE_CHARS {
        return err(ExprErrorKind::Limit, MAX_SOURCE_CHARS + 1, format!("longer than {MAX_SOURCE_CHARS} characters"));
    }
    let toks = tokenize(&chars)?;
    if toks.is_empty() {
        return err(ExprErrorKind::Syntax, 1, "empty expression");
    }
    let mut p = Parser { toks, pos: 0, end_col: chars.len() + 1, ops: Vec::new(), depth: 0, syms };
    p.expr()?;
    if p.eat(&TokKind::Semi) && p.pos < p.toks.len() {
        return err(ExprErrorKind::Unsupported, p.col(), "several statements are not supported");
    }
    if p.pos < p.toks.len() {
        let kind = describe(&p.toks[p.pos].kind);
        return err(ExprErrorKind::Syntax, p.col(), format!("unexpected {kind}"));
    }
    // Verify the stack discipline once, and measure the depth.
    let mut depth = 0usize;
    let mut max_depth = 0usize;
    for op in &p.ops {
        let pops = op.pops();
        if depth < pops {
            return err(ExprErrorKind::Syntax, 1, "internal error: stack underflow");
        }
        depth = depth - pops + 1;
        max_depth = max_depth.max(depth);
    }
    if depth != 1 || p.ops.len() > MAX_EXPR_INSTRUCTIONS {
        return err(ExprErrorKind::Syntax, 1, "internal error: unbalanced expression");
    }
    Ok(Expr { ops: p.ops, max_depth: max_depth as u16, salt: fnv1a(src) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str) -> f32 {
        let mut s = Symbols::new(&["rage", "mode"]);
        compile(src, &mut s).unwrap_or_else(|e| panic!("{src}: {e}")).eval(&ExprInputs::default())
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(eval("1 + 2 * 3"), 7.0);
        assert_eq!(eval("(1 + 2) * 3"), 9.0);
        assert_eq!(eval("10 - 4 - 3"), 3.0);
        assert_eq!(eval("2 * 3 % 4"), 2.0);
        assert_eq!(eval("-5 % 3"), -2.0);
        assert_eq!(eval("-(2 + 1)"), -3.0);
        assert_eq!(eval("--4"), 4.0);
        assert_eq!(eval(".5 + 1."), 1.5);
        assert_eq!(eval("1e2"), 100.0);
    }

    #[test]
    fn logic_and_comparisons() {
        assert_eq!(eval("1 < 2 && 2 < 3"), 1.0);
        assert_eq!(eval("1 > 2 || 0"), 0.0);
        assert_eq!(eval("!0"), 1.0);
        assert_eq!(eval("!5"), 0.0);
        assert_eq!(eval("1 + 1 == 2"), 1.0);
        assert_eq!(eval("1 < 2 == 1"), 1.0);
        assert_eq!(eval("true && !false"), 1.0);
        assert_eq!(eval("0 || 1 && 0"), 0.0);
    }

    #[test]
    fn ternary_forms() {
        assert_eq!(eval("1 ? 5 : 6"), 5.0);
        assert_eq!(eval("0 ? 5 : 6"), 6.0);
        assert_eq!(eval("0 ? 5"), 0.0);
        assert_eq!(eval("1 ? 5"), 5.0);
        assert_eq!(eval("0 ? 1 : 0 ? 2 : 3"), 3.0);
    }

    #[test]
    fn nan_and_inf_become_zero() {
        assert_eq!(eval("1 / 0"), 0.0);
        assert_eq!(eval("5 % 0"), 0.0);
        assert_eq!(eval("math.sqrt(-1)"), 0.0);
        assert_eq!(eval("math.ln(0)"), 0.0);
        assert_eq!(eval("math.pow(10, 100)"), 0.0);
        assert_eq!(eval("math.acos(2)"), 0.0);
    }

    #[test]
    fn math_functions_golden() {
        assert!((eval("math.sin(90)") - 1.0).abs() < 1e-6);
        assert!((eval("math.cos(180)") + 1.0).abs() < 1e-6);
        assert_eq!(eval("math.abs(-3)"), 3.0);
        assert_eq!(eval("math.clamp(5, 0, 2)"), 2.0);
        assert_eq!(eval("math.clamp(-5, 0, 2)"), 0.0);
        assert_eq!(eval("math.clamp(1, 3, 2)"), 2.0);
        assert_eq!(eval("math.lerp(10, 20, 0.25)"), 12.5);
        assert_eq!(eval("math.min(3, 4)"), 3.0);
        assert_eq!(eval("math.max(3, 4)"), 4.0);
        assert_eq!(eval("math.floor(-1.5)"), -2.0);
        assert_eq!(eval("math.ceil(1.2)"), 2.0);
        assert_eq!(eval("math.round(2.5)"), 3.0);
        assert_eq!(eval("math.trunc(-1.9)"), -1.0);
        assert_eq!(eval("math.sign(-9)"), -1.0);
        assert_eq!(eval("math.pow(2, 10)"), 1024.0);
        assert_eq!(eval("math.mod(7, 4)"), 3.0);
        assert!((eval("math.atan2(1, 1)") - 45.0).abs() < 1e-4);
        assert!((eval("math.pi") - std::f32::consts::PI).abs() < 1e-7);
    }

    #[test]
    fn enum_names_compare_against_synced_values() {
        let mut s = Symbols::new(&["state", "n"]);
        assert!(s.set_enum_values("state", &["idle", "sit", "pounce"]));
        assert!(!s.set_enum_values("nope", &["a"]));
        let mut inp = ExprInputs::default();
        inp.synced[0] = 1.0;
        let run = |src: &str, s: &mut Symbols, inp: &ExprInputs| compile(src, s).unwrap_or_else(|e| panic!("{src}: {e}")).eval(inp);
        assert_eq!(run("q.synced('state') == 'sit'", &mut s, &inp), 1.0);
        assert_eq!(run("'sit' == q.synced('state')", &mut s, &inp), 1.0);
        assert_eq!(run("query.synced('STATE') == 'SIT'", &mut s, &inp), 1.0);
        assert_eq!(run("q.synced('state') != 'sit'", &mut s, &inp), 0.0);
        assert_eq!(run("q.synced('state') == 'pounce'", &mut s, &inp), 0.0);
        assert_eq!(run("q.synced('state') == 'idle' || q.synced('state') == 'sit'", &mut s, &inp), 1.0);
        // the index form still works
        assert_eq!(run("q.synced('state') == 1", &mut s, &inp), 1.0);
        // the string is just the index: same bytecode length as the index form
        assert_eq!(
            compile("q.synced('state') == 'sit'", &mut s).unwrap().len(),
            compile("q.synced('state') == 1", &mut s).unwrap().len()
        );
        // errors carry a column
        let e = compile("q.synced('state') == 'nope'", &mut s).unwrap_err();
        assert_eq!(e.kind, ExprErrorKind::UnknownEnumValue);
        assert_eq!(e.column, 22);
        assert!(e.message.contains("nope") && e.message.contains("idle, sit, pounce"), "{e}");
        // not an enum
        let e = compile("q.synced('n') == 'sit'", &mut s).unwrap_err();
        assert_eq!(e.kind, ExprErrorKind::UnknownEnumValue);
        // strings anywhere else stay unsupported
        for bad in ["'sit'", "q.synced('state') + 1 == 'sit'", "q.synced('state') < 'sit'", "'sit' == 1", "1 == 'sit'", "q.health == 'sit'"] {
            let e = compile(bad, &mut s).unwrap_err();
            assert_eq!(e.kind, ExprErrorKind::Unsupported, "{bad}: {e}");
        }
    }

    #[test]
    fn inputs_synced_and_variables() {
        let mut s = Symbols::new(&["rage", "mode"]);
        let e = compile("q.health * 2 + query.synced('MODE') + v.boost", &mut s).unwrap();
        assert_eq!(s.variables(), &["boost".to_string()]);
        let mut inp = ExprInputs::default();
        inp.set(Query::Health, 5.0);
        inp.synced[1] = 3.0;
        inp.variables[0] = 0.5;
        assert_eq!(e.eval(&inp), 13.5);
        // aliases and case
        let e2 = compile("Variable.Boost + V.boost + m.abs(-1)", &mut s).unwrap();
        assert_eq!(e2.eval(&inp), 2.0);
        assert_eq!(s.variables().len(), 1);
    }

    #[test]
    fn non_finite_inputs_are_sanitised() {
        let mut s = Symbols::new(&[]);
        let e = compile("q.health + 1", &mut s).unwrap();
        let mut inp = ExprInputs::default();
        inp.set(Query::Health, f32::NAN);
        assert_eq!(e.eval(&inp), 1.0);
        inp.set(Query::Health, f32::INFINITY);
        assert_eq!(e.eval(&inp), 1.0);
    }

    #[test]
    fn constants_fold() {
        let mut s = Symbols::new(&[]);
        let e = compile("1 + 2 * 3 - math.sin(90) * 0", &mut s).unwrap();
        assert_eq!(e.as_constant(), Some(7.0));
        let e = compile("-5", &mut s).unwrap();
        assert_eq!(e.as_constant(), Some(-5.0));
        let e = compile("q.health + 1 + 2", &mut s).unwrap();
        assert_eq!(e.len(), 5); // folding is local: ((q + 1) + 2)
    }

    #[test]
    fn random_is_deterministic_and_in_range() {
        let mut s = Symbols::new(&[]);
        let e = compile("math.random(10, 20)", &mut s).unwrap();
        let mut inp = ExprInputs { seed: 7, ..Default::default() };
        let a = e.eval(&inp);
        assert_eq!(a, e.eval(&inp));
        assert!((10.0..20.0).contains(&a));
        inp.random_epoch = 1;
        assert_ne!(a, e.eval(&inp));
        inp.random_epoch = 0;
        inp.seed = 8;
        assert_ne!(a, e.eval(&inp));
        // two calls in one expression differ
        let e2 = compile("math.random(0, 1) - math.random(0, 1)", &mut s).unwrap();
        assert_ne!(e2.eval(&ExprInputs::default()), 0.0);
    }

    #[test]
    fn errors_have_columns() {
        let mut s = Symbols::new(&["x"]);
        let e = compile("1 + (2 * 3", &mut s).unwrap_err();
        assert_eq!((e.kind, e.column), (ExprErrorKind::Syntax, 11));
        let e = compile("1 + $", &mut s).unwrap_err();
        assert_eq!((e.kind, e.column), (ExprErrorKind::Syntax, 5));
        let e = compile("1 + * 2", &mut s).unwrap_err();
        assert_eq!((e.kind, e.column), (ExprErrorKind::Syntax, 5));
        let e = compile("query.synced('nope')", &mut s).unwrap_err();
        assert_eq!((e.kind, e.column), (ExprErrorKind::UnknownSynced, 14));
        let e = compile("math.sin(1, 2)", &mut s).unwrap_err();
        assert_eq!(e.kind, ExprErrorKind::Syntax);
        let e = compile("", &mut s).unwrap_err();
        assert_eq!(e.kind, ExprErrorKind::Syntax);
        let e = compile("1 2", &mut s).unwrap_err();
        assert_eq!((e.kind, e.column), (ExprErrorKind::Syntax, 3));
    }

    #[test]
    fn unsupported_constructs_are_classified() {
        let mut s = Symbols::new(&[]);
        for (src, col) in [
            ("math.hermite_blend(1)", 6),
            ("query.is_sneaking", 7),
            ("v.x = 1", 5),
            ("temp.x", 1),
            ("loop(3, 1)", 1),
            ("v.a; v.b", 6),
            ("'text'", 1),
            ("v.a.b", 4),
            ("a ?? b", 3),
        ] {
            let e = compile(src, &mut s).unwrap_err();
            assert_eq!(e.kind, ExprErrorKind::Unsupported, "{src}");
            assert_eq!(e.column, col, "{src}");
        }
        assert!(compile("v.a;", &mut s).is_ok());
    }

    #[test]
    fn limits() {
        let mut s = Symbols::new(&[]);
        let long = vec!["q.health"; 129].join(" + "); // 129 pushes + 128 adds = 257
        assert_eq!(compile(&long, &mut s).unwrap_err().kind, ExprErrorKind::Limit);
        let ok = vec!["q.health"; 128].join(" + "); // 255
        assert_eq!(compile(&ok, &mut s).unwrap().len(), 255);
        let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(compile(&deep, &mut s).unwrap_err().kind, ExprErrorKind::Limit);
        let neg = format!("{}q.health", "-".repeat(100));
        assert_eq!(compile(&neg, &mut s).unwrap_err().kind, ExprErrorKind::Limit);
        let src = "1".repeat(MAX_SOURCE_CHARS + 1);
        assert_eq!(compile(&src, &mut s).unwrap_err().kind, ExprErrorKind::Limit);
        for i in 0..MAX_VARIABLES {
            compile(&format!("v.n{i}"), &mut s).unwrap();
        }
        assert_eq!(compile("v.overflow", &mut s).unwrap_err().kind, ExprErrorKind::Limit);
        assert!(compile("v.n3", &mut s).is_ok());
    }

    #[test]
    fn deep_stack_expression_runs_on_the_big_stack() {
        let mut s = Symbols::new(&[]);
        // 60 operands on the stack at once.
        let src = format!("{}0{}", "q.health + (".repeat(40), ")".repeat(40));
        let e = compile(&src, &mut s).unwrap();
        assert!(e.max_depth > 24);
        let mut inp = ExprInputs::default();
        inp.set(Query::Health, 1.0);
        assert_eq!(e.eval(&inp), 40.0);
    }
}
