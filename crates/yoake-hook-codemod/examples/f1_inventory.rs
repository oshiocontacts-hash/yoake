//! FR-F1 spike: function inventory by eligibility class (syn only, follows `mod x;` declarations).
//! usage: f1_inventory <crate-name> <root-file>   (prints one JSON object per fn on stdout)
#[path = "../src/eligible.rs"]
#[allow(dead_code)]
mod eligible;

use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{ImplItem, Item, TraitItem};

#[derive(Default)]
struct Flags {
    generic_fn: bool,
    generic_impl: bool,
    impl_trait_arg: bool,
    impl_trait_ret: bool,
    nonstatic_ret: bool,
    nonstatic_arg: bool,
    is_async: bool,
    is_const: bool,
    is_unsafe: bool,
    is_extern: bool,
    by_value_self: bool,
    destructure: bool,
    track_caller: bool,
    dyn_arg: bool,
    fnptr_arg: bool,
}

struct Rec {
    module: String,
    file: String,
    name: String,
    kind: &'static str, // free | inherent | trait_impl | trait_default
    vis: &'static str,
    flags: Flags,
    reason: Option<&'static str>,
    closures: usize,
    body_stmts: usize,
}

fn has_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("test") {
            return true;
        }
        if a.path().is_ident("cfg") {
            let s = quote::ToTokens::to_token_stream(&a.meta).to_string();
            return s.contains("test") && !s.contains("not (test") && !s.contains("not(test");
        }
        false
    })
}

struct TyScan {
    impl_trait: bool,
    nonstatic: bool,
    dyn_: bool,
    fnptr: bool,
}
impl<'ast> Visit<'ast> for TyScan {
    fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
        self.impl_trait = true;
    }
    fn visit_type_trait_object(&mut self, n: &'ast syn::TypeTraitObject) {
        self.dyn_ = true;
        syn::visit::visit_type_trait_object(self, n);
    }
    fn visit_type_bare_fn(&mut self, n: &'ast syn::TypeBareFn) {
        self.fnptr = true;
        syn::visit::visit_type_bare_fn(self, n);
    }
    fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
        if r.lifetime.as_ref().is_none_or(|l| l.ident != "static") {
            self.nonstatic = true;
        }
        syn::visit::visit_type_reference(self, r);
    }
    fn visit_lifetime(&mut self, l: &'ast syn::Lifetime) {
        if l.ident != "static" && l.ident != "_" {
            self.nonstatic = true;
        }
    }
}
fn scan_ty(t: &syn::Type) -> TyScan {
    let mut s = TyScan { impl_trait: false, nonstatic: false, dyn_: false, fnptr: false };
    s.visit_type(t);
    s
}

struct Closures(usize);
impl<'ast> Visit<'ast> for Closures {
    fn visit_expr_closure(&mut self, n: &'ast syn::ExprClosure) {
        self.0 += 1;
        syn::visit::visit_expr_closure(self, n);
    }
}

struct Walker {
    out: Vec<Rec>,
    file: String,
    top: String,
}

fn vis_of(v: &syn::Visibility) -> &'static str {
    match v {
        syn::Visibility::Public(_) => "pub",
        syn::Visibility::Restricted(_) => "pub_crate",
        syn::Visibility::Inherited => "private",
    }
}

impl Walker {
    fn fn_rec(&mut self, kind: &'static str, vis: &syn::Visibility, attrs: &[syn::Attribute], sig: &syn::Signature, block: &syn::Block, impl_generic: bool) {
        let mut f = Flags::default();
        f.generic_fn = sig.generics.type_params().next().is_some() || sig.generics.const_params().next().is_some();
        f.generic_impl = impl_generic;
        f.is_async = sig.asyncness.is_some();
        f.is_const = sig.constness.is_some();
        f.is_unsafe = sig.unsafety.is_some();
        f.is_extern = sig.abi.is_some();
        f.track_caller = attrs.iter().any(|a| a.path().is_ident("track_caller"));
        if let syn::ReturnType::Type(_, t) = &sig.output {
            let s = scan_ty(t);
            f.impl_trait_ret = s.impl_trait;
            f.nonstatic_ret = s.nonstatic;
        }
        for a in &sig.inputs {
            match a {
                syn::FnArg::Receiver(r) => {
                    if r.reference.is_none() || r.colon_token.is_some() {
                        f.by_value_self = true;
                    }
                }
                syn::FnArg::Typed(p) => {
                    if !matches!(&*p.pat, syn::Pat::Ident(_)) {
                        f.destructure = true;
                    }
                    let s = scan_ty(&p.ty);
                    f.impl_trait_arg |= s.impl_trait;
                    f.nonstatic_arg |= s.nonstatic;
                    f.dyn_arg |= s.dyn_;
                    f.fnptr_arg |= s.fnptr;
                }
            }
        }
        let mut c = Closures(0);
        c.visit_block(block);
        let reason = eligible::check_fn(attrs, sig).err();
        self.out.push(Rec {
            module: self.top.clone(),
            file: self.file.clone(),
            name: sig.ident.to_string(),
            kind,
            vis: vis_of(vis),
            flags: f,
            reason,
            closures: c.0,
            body_stmts: block.stmts.len(),
        });
    }

    fn items(&mut self, items: &[Item], dir: &Path, files: &mut Vec<PathBuf>) {
        for it in items {
            match it {
                Item::Fn(f) if !has_cfg_test(&f.attrs) => self.fn_rec("free", &f.vis, &f.attrs, &f.sig, &f.block, false),
                Item::Impl(i) if !has_cfg_test(&i.attrs) => {
                    let gen_ = i.generics.type_params().next().is_some() || i.generics.const_params().next().is_some();
                    let kind = if i.trait_.is_some() { "trait_impl" } else { "inherent" };
                    for ii in &i.items {
                        if let ImplItem::Fn(m) = ii {
                            if !has_cfg_test(&m.attrs) {
                                self.fn_rec(kind, &m.vis, &m.attrs, &m.sig, &m.block, gen_);
                            }
                        }
                    }
                }
                Item::Trait(t) if !has_cfg_test(&t.attrs) => {
                    for ti in &t.items {
                        if let TraitItem::Fn(m) = ti {
                            if let Some(b) = &m.default {
                                self.fn_rec("trait_default", &t.vis, &m.attrs, &m.sig, b, true);
                            }
                        }
                    }
                }
                Item::Mod(m) if !has_cfg_test(&m.attrs) => {
                    let name = m.ident.to_string();
                    if let Some((_, inner)) = &m.content {
                        let d = dir.join(&name);
                        self.items(inner, &d, files);
                    } else {
                        let path_attr = m.attrs.iter().find(|a| a.path().is_ident("path")).and_then(|a| {
                            if let syn::Meta::NameValue(nv) = &a.meta {
                                if let syn::Expr::Lit(l) = &nv.value {
                                    if let syn::Lit::Str(s) = &l.lit {
                                        return Some(s.value());
                                    }
                                }
                            }
                            None
                        });
                        let cand: Vec<PathBuf> = if let Some(p) = path_attr {
                            let mut v = vec![dir.join(&p)];
                            if let Some(par) = dir.parent() {
                                v.push(par.join(&p));
                            }
                            v
                        } else {
                            vec![dir.join(format!("{name}.rs")), dir.join(&name).join("mod.rs")]
                        };
                        if let Some(p) = cand.into_iter().find(|p| p.is_file()) {
                            files.push(p);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let krate = args[0].clone();
    let root = PathBuf::from(&args[1]);
    let mut w = Walker { out: Vec::new(), file: String::new(), top: String::new() };
    let mut queue: Vec<(PathBuf, String, bool)> = vec![(root.clone(), "(root)".to_string(), true)];
    let mut seen = std::collections::BTreeSet::new();
    while let Some((p, top, is_root)) = queue.pop() {
        let canon = p.canonicalize().unwrap_or(p.clone());
        if !seen.insert(canon) {
            continue;
        }
        let Ok(src) = std::fs::read_to_string(&p) else { continue };
        let Ok(parsed) = syn::parse_file(&src) else {
            eprintln!("parse fail {}", p.display());
            continue;
        };
        let stem = p.file_stem().unwrap().to_string_lossy().to_string();
        let is_mod_rs = is_root || stem == "mod";
        let dir0 = p.parent().unwrap().to_path_buf();
        let dir = if is_mod_rs { dir0 } else { dir0.join(&stem) };
        w.file = p.to_string_lossy().replace('\\', "/");
        w.top = top.clone();
        let mut children = Vec::new();
        w.items(&parsed.items, &dir, &mut children);
        for c in children {
            let t = if is_root {
                let s = c.file_stem().unwrap().to_string_lossy().to_string();
                if s == "mod" { c.parent().unwrap().file_name().unwrap().to_string_lossy().to_string() } else { s }
            } else {
                top.clone()
            };
            queue.push((c, t, false));
        }
    }
    for r in &w.out {
        let f = &r.flags;
        println!(
            "{}",
            serde_json::json!({
                "crate": krate, "module": r.module, "file": r.file, "name": r.name, "kind": r.kind, "vis": r.vis,
                "reason": r.reason, "generic_fn": f.generic_fn, "generic_impl": f.generic_impl, "impl_trait_arg": f.impl_trait_arg,
                "impl_trait_ret": f.impl_trait_ret, "nonstatic_ret": f.nonstatic_ret, "nonstatic_arg": f.nonstatic_arg,
                "async": f.is_async, "const": f.is_const, "unsafe": f.is_unsafe, "extern": f.is_extern, "by_value_self": f.by_value_self,
                "destructure": f.destructure, "track_caller": f.track_caller, "dyn_arg": f.dyn_arg, "fnptr_arg": f.fnptr_arg,
                "closures": r.closures, "stmts": r.body_stmts,
            })
        );
    }
}
