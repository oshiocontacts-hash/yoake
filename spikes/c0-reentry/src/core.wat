(module
  (import "env" "original_f" (func $of (param i32) (result i32)))
  (import "env" "original_g" (func $og (param i32) (result i32)))
  (import "env" "original_h" (func $oh (param i32) (result i32)))
  (import "env" "original_p" (func $op (param i32 i32) (result i32)))
  (import "env" "log" (func $log (param i32 i32)))
  (memory (export "memory") 2)
  (global $sp (export "sp") (mut i32) (i32.const 65536))   ;; shadow stack top (grows down), like LLVM's __stack_pointer
  (global $heap (mut i32) (i32.const 1024))
  (global $burn_f (mut i32) (i32.const 0))
  (global $burn_g (mut i32) (i32.const 0))
  (global $burn_h (mut i32) (i32.const 0))
  (global $strict (mut i32) (i32.const 0))
  (func (export "set_burn") (param i32 i32 i32)
    (global.set $burn_f (local.get 0)) (global.set $burn_g (local.get 1)) (global.set $burn_h (local.get 2)))
  ;; strict=1: hook_f also asserts that the GLOBAL sp is unchanged after the nested call
  ;; (LLVM-generated code does not do that: it works from a local copy of sp and restores the global in its epilogue)
  (func (export "set_strict") (param i32) (global.set $strict (local.get 0)))
  (func $burn (param $n i32)
    (loop $l
      (if (local.get $n) (then
        (local.set $n (i32.sub (local.get $n) (i32.const 1)))
        (br $l)))))
  (func (export "alloc") (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (local.get $n)))
    (local.get $p))
  (func (export "free") (param $p i32) (global.set $heap (local.get $p)))

  (func (export "hook_f") (param $x i32) (result i32)
    (local $r i32) (local $fp i32)
    (local.set $fp (i32.sub (global.get $sp) (i32.const 16)))
    (global.set $sp (local.get $fp))
    (i32.store (local.get $fp) (local.get $x))
    (call $log (i32.const 1) (local.get $x))
    (call $burn (global.get $burn_f))
    (local.set $r (call $of (local.get $x)))
    ;; after re-entry: our frame must be intact
    (if (i32.ne (i32.load (local.get $fp)) (local.get $x)) (then (unreachable)))
    (if (global.get $strict) (then
      (if (i32.ne (global.get $sp) (local.get $fp)) (then (unreachable)))))
    (call $log (i32.const 2) (local.get $r))
    (global.set $sp (i32.add (local.get $fp) (i32.const 16)))   ;; epilogue restores from the local
    (i32.add (local.get $r) (i32.const 1000)))

  (func (export "hook_g") (param $x i32) (result i32)
    (local $r i32) (local $fp i32)
    (local.set $fp (i32.sub (global.get $sp) (i32.const 16)))
    (global.set $sp (local.get $fp))
    (i32.store (local.get $fp) (local.get $x))
    (call $log (i32.const 3) (local.get $x))
    (call $burn (global.get $burn_g))
    (if (i32.eq (local.get $x) (i32.const 100)) (then (unreachable)))  ;; trap trigger
    (local.set $r (call $og (local.get $x)))
    (if (i32.ne (i32.load (local.get $fp)) (local.get $x)) (then (unreachable)))
    (call $log (i32.const 4) (local.get $r))
    (global.set $sp (i32.add (local.get $fp) (i32.const 16)))
    (i32.add (local.get $r) (i32.const 1)))

  ;; trivial hooks for the benchmark / epoch test
  (func (export "hook_h") (param $x i32) (result i32)
    (local $r i32)
    (local.set $r (call $oh (local.get $x)))
    (if (global.get $burn_h) (then (call $burn (global.get $burn_h))))
    (local.get $r))
  (func (export "hook_p") (param $ptr i32) (param $len i32) (result i32)
    (call $op (local.get $ptr) (local.get $len)))
)
