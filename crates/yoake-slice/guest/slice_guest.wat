;; MX3V1 slice guest (hand-written: only wasm32-wasip2 is installed, no wasm32-unknown-unknown).
;; Every hook is `(handle i64) -> i64`. Field ids belong to the shim's FieldTable:
;; 0 = a counter (fire_ticks), 1 = position.x (f64 bits). Scratch buffer: bytes 64..80.
(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "ticks" (func $ticks (result i64)))
  (memory (export "memory") 1)
  (global $outer (mut i64) (i64.const 0))
  (global $saved (mut i64) (i64.const 0))
  (global $depth (mut i32) (i32.const 0))

  ;; (a) the guest changes self through the handle around `original`:
  ;; counter = 5; original; read the counter back; counter += 10; position.x = 12.5
  (func (export "hook_basic") (param $h i64) (result i64)
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 5)))
    (call $report (i32.const 10) (i64.extend_i32_s (call $orig (local.get $h) (i32.const 72))))
    (call $report (i32.const 11) (i64.extend_i32_s (call $get (local.get $h) (i32.const 0) (i32.const 64))))
    (call $report (i32.const 12) (i64.load (i32.const 64)))
    (drop (call $set (local.get $h) (i32.const 0) (i64.add (i64.load (i32.const 64)) (i64.const 10))))
    (drop (call $set (local.get $h) (i32.const 1) (i64.reinterpret_f64 (f64.const 12.5))))
    (i64.const 0))

  ;; (b)+(c) re-entrancy and the lending lock. The activation that finds $outer set is nested
  ;; (it runs inside the outer one's `original`); it tries the outer handle, which is lent.
  (func (export "hook_reenter") (param $h i64) (result i64)
    (local $o i64) (local $d i32)
    (global.set $depth (i32.add (global.get $depth) (i32.const 1)))
    (local.set $d (global.get $depth))
    (call $report (i32.const 40) (i64.extend_i32_s (local.get $d)))
    (local.set $o (global.get $outer))
    (if (i64.ne (local.get $o) (i64.const 0)) (then
      (call $report (i32.const 20) (i64.extend_i32_s (call $get (local.get $o) (i32.const 0) (i32.const 64))))
      (call $report (i32.const 21) (i64.extend_i32_s (call $set (local.get $o) (i32.const 0) (i64.const 999))))
      (call $report (i32.const 22) (i64.extend_i32_s (call $orig (local.get $o) (i32.const 72))))
      (call $report (i32.const 23) (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.const 7))))))
    (global.set $outer (local.get $h))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (global.set $outer (local.get $o))
    ;; own handle usable again after original; leave a per-depth mark in the counter
    (call $report (i32.const 30)
      (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.add (i64.const 100) (i64.extend_i32_s (local.get $d))))))
    (global.set $depth (i32.sub (global.get $depth) (i32.const 1)))
    (i64.const 0))

  ;; stale handles: the second activation tries the first one's handle, and a forged arg index
  (func (export "hook_stale") (param $h i64) (result i64)
    (if (i64.ne (global.get $saved) (i64.const 0)) (then
      (call $report (i32.const 50) (i64.extend_i32_s (call $get (global.get $saved) (i32.const 0) (i32.const 64))))
      (call $report (i32.const 51) (i64.extend_i32_s (call $get (i64.or (local.get $h) (i64.const 1)) (i32.const 0) (i32.const 64))))
      (call $report (i32.const 52) (i64.extend_i32_s (call $get (local.get $h) (i32.const 77) (i32.const 64))))))
    (global.set $saved (local.get $h))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (i64.const 0))

  ;; traps
  (func (export "hook_trap_before") (param $h i64) (result i64)
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 31)))
    (unreachable))
  (func (export "hook_trap_after") (param $h i64) (result i64)
    (drop (call $orig (local.get $h) (i32.const 72)))
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 55)))
    (unreachable))
  ;; nested activation traps while the outer one is waiting in `original`
  (func (export "hook_trap_nested") (param $h i64) (result i64)
    (if (i64.ne (global.get $outer) (i64.const 0)) (then (unreachable)))
    (global.set $outer (local.get $h))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (global.set $outer (i64.const 0))
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 31)))
    (i64.const 0))
  ;; fuel
  (func (export "hook_spin") (param $h i64) (result i64)
    (loop $l (br $l))
    (i64.const 0))
  ;; epoch: original (the host body is slow), then a modest guest loop
  (func (export "hook_slow") (param $h i64) (result i64)
    (local $n i32)
    (drop (call $orig (local.get $h) (i32.const 72)))
    (local.set $n (i32.const 2000))
    (loop $l
      (if (local.get $n) (then
        (local.set $n (i32.sub (local.get $n) (i32.const 1)))
        (br $l))))
    (i64.const 0))
  ;; a trap after nesting happened, for the fuel-restore check: burns fuel in a nested-capable hook
  (func (export "hook_burn") (param $h i64) (result i64)
    (local $n i32)
    (local.set $n (i32.const 4000))
    (loop $l
      (if (local.get $n) (then
        (local.set $n (i32.sub (local.get $n) (i32.const 1)))
        (br $l))))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (i64.const 0))

  ;; original twice from WASM; the second call must run the body again
  (func (export "hook_twice") (param $h i64) (result i64)
    (call $report (i32.const 80) (i64.extend_i32_s (call $orig (local.get $h) (i32.const 72))))
    (call $report (i32.const 81) (i64.extend_i32_s (call $orig (local.get $h) (i32.const 72))))
    (i64.const 0))
  ;; `out` past the end of the 1-page memory: the import must trap the guest, not corrupt anything
  (func (export "hook_oob_get") (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 65530)))
    (i64.const 0))
  (func (export "hook_oob_original") (param $h i64) (result i64)
    (drop (call $orig (local.get $h) (i32.const 65530)))
    (i64.const 0))
  ;; a nested activation spins until its own fuel is gone; the outer one must survive
  (func (export "hook_nested_spin") (param $h i64) (result i64)
    (if (i64.ne (global.get $outer) (i64.const 0)) (then (loop $l (br $l))))
    (global.set $outer (local.get $h))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (global.set $outer (i64.const 0))
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 31)))
    (i64.const 0))
  ;; field 2's accessor calls a hooked engine function (nested activation on the same object)
  (func (export "hook_accessor") (param $h i64) (result i64)
    (global.set $depth (i32.add (global.get $depth) (i32.const 1)))
    (if (i32.eq (global.get $depth) (i32.const 1)) (then
      (call $report (i32.const 60) (i64.extend_i32_s (call $get (local.get $h) (i32.const 2) (i32.const 64))))))
    (call $report (i32.const 61) (i64.extend_i32_s (call $orig (local.get $h) (i32.const 72))))
    (global.set $depth (i32.sub (global.get $depth) (i32.const 1)))
    (i64.const 0))
  ;; two &mut arguments: handle h is argument 0, h+1 argument 1
  (func (export "hook_pair") (param $h i64) (result i64)
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 11)))
    (drop (call $set (i64.add (local.get $h) (i64.const 1)) (i32.const 0) (i64.const 22)))
    (call $report (i32.const 70) (i64.extend_i32_s (call $orig (local.get $h) (i32.const 72))))
    (call $report (i32.const 71) (i64.extend_i32_s (call $get (local.get $h) (i32.const 0) (i32.const 64))))
    (call $report (i32.const 72) (i64.load (i32.const 64)))
    (call $report (i32.const 73) (i64.extend_i32_s (call $get (i64.add (local.get $h) (i64.const 1)) (i32.const 0) (i32.const 64))))
    (call $report (i32.const 74) (i64.load (i32.const 64)))
    (i64.const 0))

  ;; recurses forever through field 2's accessor (which ticks the object, hooking this guest again)
  (func (export "hook_recurse") (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 2) (i32.const 64)))
    (i64.const 0))
  ;; epoch: spin ~25 ticks, run a nested hook under `get` (field 2), then spin to tick 60 or until trapped.
  ;; A nested hook must not extend the outer deadline: with a 30-tick budget the second spin traps.
  (func (export "hook_deadline") (param $h i64) (result i64)
    (local $t0 i64)
    (global.set $depth (i32.add (global.get $depth) (i32.const 1)))
    (if (i32.eq (global.get $depth) (i32.const 1)) (then
      (local.set $t0 (call $ticks))
      (loop $a (br_if $a (i64.lt_u (call $ticks) (i64.add (local.get $t0) (i64.const 25)))))
      (drop (call $get (local.get $h) (i32.const 2) (i32.const 64)))
      (loop $b (br_if $b (i64.lt_u (call $ticks) (i64.add (local.get $t0) (i64.const 60)))))
      (call $report (i32.const 90) (i64.const 1))))
    (global.set $depth (i32.sub (global.get $depth) (i32.const 1)))
    (i64.const 0))

  ;; benchmark hooks
  (func (export "hook_nop") (param $h i64) (result i64) (i64.const 0))
  (func (export "hook_min") (param $h i64) (result i64)
    (drop (call $orig (local.get $h) (i32.const 72)))
    (i64.const 0))
  (func (export "hook_rw") (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 64)))
    (drop (call $set (local.get $h) (i32.const 0) (i64.add (i64.load (i32.const 64)) (i64.const 1))))
    (drop (call $orig (local.get $h) (i32.const 72)))
    (i64.const 0))
)
