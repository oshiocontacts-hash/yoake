//! THIR fixture for the call-site parser: method calls with autoref, autoderef through Box and Vec,
//! overloaded operators, trait calls, a borrowed return, a closure argument and a macro argument.
use std::ops::Add;

pub struct Stack {
    pub items: Vec<u32>,
}

pub struct Game<'a> {
    pub name: &'a str,
    pub stack: Box<Stack>,
}

#[derive(Clone, Copy)]
pub struct V(pub f64);

impl Add for V {
    type Output = V;
    fn add(self, other: V) -> V {
        V(self.0 + other.0)
    }
}

pub trait Rules {
    fn gravity(&self) -> f64;
}

impl Rules for Stack {
    fn gravity(&self) -> f64 {
        0.08
    }
}

impl Stack {
    pub fn first(&self) -> Option<&u32> {
        self.items.first()
    }
    pub fn push_twice(&mut self, value: u32) {
        self.items.push(value);
        self.items.push(value);
    }
}

impl Game<'_> {
    pub fn held(&self) -> Option<&u32> {
        self.stack.first()
    }
    pub fn tick(&mut self, extra: u32) -> f64 {
        self.stack.push_twice(extra);
        let n = self.stack.items.len();
        let held = self.held().cloned();
        let sum = V(1.0) + V(n as f64);
        let shown = self.name.to_uppercase();
        assert!(self.stack.items.contains(&extra));
        let doubled: Vec<u32> = self.stack.items.iter().map(|x| x * 2).collect();
        sum.0 + self.stack.gravity() + held.unwrap_or(0) as f64 + shown.len() as f64 + doubled.len() as f64
    }
}
