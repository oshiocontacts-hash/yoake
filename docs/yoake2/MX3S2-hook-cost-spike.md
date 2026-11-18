# S2 spike: cost and mechanics of a hook slot on nearly every engine function (you can build and test; do not run git commands that write)

`-C` root: clone `horizon-integ`, branch `spike/s2-hooks` (= yoake/integ-1 @0626a35ee). Throwaway experiment: measure and report.

Read first: C:\Users\USER\NestgateHorizon\YOAKE_MIXIN2.md, section 3.3. The existing Mixin machinery is yoake-inject-macro and yoake-inject-rt; reuse ideas from it.

Do:
1. Write a proc-macro attribute `#[yoake_hook]` in a new crate, `crates/yoake-hook-macro`, plus a small runtime. For a function it creates a static slot, a null-checked `AtomicPtr` to a callback chain, checked at entry (HEAD: may replace args or return early) and at exit (RETURN: may replace the result). Work out how to handle:
   - methods (`&self` / `&mut self`);
   - borrowed arguments with lifetimes: the typed callback signature, e.g. `for<'a> fn(&'a mut Game, ...)`;
   - generic functions: one slot per function shared by all monomorphizations, or per type? Document the choice and why;
   - `impl Trait` returns, closures and async fns: skip with a reason, or support.
2. Apply it to every fn in one large, hot module, by a codemod or by hand with a script: for example the entity tick code (`crates/horizon-client/src/entities.rs` and the item/mob tick functions it calls), or a horizon-world module that runs every tick.
3. Measure with no hooks registered, release build:
   - the module's tick cost before and after, using existing benches (`horizon-bench`, `crates/horizon-client/src/benchmark.rs`) or a focused criterion-style loop you add;
   - binary size growth;
   - compile-time growth.
   Then measure with one no-op hook registered on a hot function.
4. Show that a hook on a function that LLVM inlines still fires: prove it with a test.
5. Run `cargo test --offline -j 4 -p horizon-client --lib` on the instrumented build to show behaviour is unchanged.

Write the report to reports/MX3S2.md:
- the numbers (ns per call, % tick time, size, compile time);
- the macro's limits (which fn shapes can't be hooked);
- a recommendation for the exclusion policy.

Don't run the game. Don't touch anything outside horizon-integ.
