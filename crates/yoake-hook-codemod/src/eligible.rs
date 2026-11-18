use std::collections::BTreeSet;

use proc_macro2::TokenTree;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Block, Expr, ExprMacro, ExprMethodCall, FnArg, Generics, ImplItem, ItemImpl,
    Lifetime, Pat, Path, Signature, StmtMacro, Type,
};

pub fn check_fn(attrs: &[Attribute], sig: &Signature) -> Result<(), &'static str> {
    if sig.asyncness.is_some() {
        return Err("async fn");
    }
    if sig.constness.is_some() {
        return Err("const fn");
    }
    if sig.unsafety.is_some() {
        return Err("unsafe fn");
    }
    if sig.abi.is_some() {
        return Err("extern ABI fn");
    }
    if sig.variadic.is_some() {
        return Err("variadic fn");
    }
    if sig.generics.type_params().next().is_some() {
        return Err("generic type params");
    }
    if sig.generics.const_params().next().is_some() {
        return Err("const generics");
    }
    if sig.generics.lifetimes().any(|lt| !lt.bounds.is_empty()) {
        return Err("bounded lifetime parameters");
    }
    if attrs.iter().any(|attr| attr.path().is_ident("track_caller")) {
        return Err("track_caller");
    }
    if let Some(clause) = &sig.generics.where_clause {
        let mut check = Unsupported::default();
        check.visit_where_clause(clause);
        if check.borrowed_result {
            return Err("lifetime-dependent where bounds");
        }
    }

    if let syn::ReturnType::Type(_, result_ty) = &sig.output {
        let mut result_check = Unsupported::default();
        result_check.visit_type(result_ty);
        if result_check.impl_trait {
            return Err("impl Trait return");
        }
        if result_check.never {
            return Err("never return");
        }
        if result_check.borrowed_result {
            return Err("borrowed return type");
        }
    }

    for arg in &sig.inputs {
        match arg {
            FnArg::Receiver(receiver) => {
                if receiver.reference.is_none() || receiver.colon_token.is_some() {
                    return Err("by-value or typed self");
                }
            }
            FnArg::Typed(pat_type) => {
                let Pat::Ident(pattern) = &*pat_type.pat else {
                    return Err("destructuring argument");
                };
                if pattern.by_ref.is_some() || pattern.subpat.is_some() {
                    return Err("ref or @ argument pattern");
                }
                let mut input_check = Unsupported::default();
                input_check.visit_type(&pat_type.ty);
                if input_check.impl_trait {
                    return Err("impl Trait argument");
                }
                if input_check.never {
                    return Err("never argument");
                }
                if input_check.nested_callable_borrows {
                    return Err("nested callable borrow");
                }
            }
        }
    }

    Ok(())
}

pub fn check_impl_generics(generics: &Generics) -> Result<(), &'static str> {
    if generics.type_params().next().is_some() || generics.const_params().next().is_some() {
        return Err("generic impl");
    }
    if generics.lifetimes().any(|lt| !lt.bounds.is_empty()) {
        return Err("bounded impl lifetimes");
    }
    if let Some(clause) = &generics.where_clause {
        let mut check = Unsupported::default();
        check.visit_where_clause(clause);
        if check.borrowed_result {
            return Err("lifetime-dependent impl where bounds");
        }
    }
    Ok(())
}

pub fn check_trait_method_body_attrs(attrs: &[Attribute]) -> Result<(), &'static str> {
    for attr in attrs {
        let path = attr.path();
        let allowed = path.is_ident("doc")
            || path.is_ident("inline")
            || path.is_ident("allow")
            || path.is_ident("expect")
            || path.is_ident("warn")
            || path.is_ident("deny")
            || path.is_ident("must_use")
            || path.is_ident("cfg")
            || path.is_ident("cfg_attr")
            || path.is_ident("track_caller")
            || path
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "yoake_hook");
        if !allowed {
            return Err("attribute may change the body");
        }
    }
    Ok(())
}

pub struct TraitImplMeta {
    pub generics_check: Result<(), &'static str>,
    pub trait_self_items: BTreeSet<String>,
    pub trait_methods: BTreeSet<String>,
    pub trait_in_scope: bool,
}

impl TraitImplMeta {
    pub fn from_impl(trait_impl: &ItemImpl, trait_in_scope: bool) -> Self {
        let generics_check = check_impl_generics(&trait_impl.generics);
        let mut trait_self_items = BTreeSet::new();
        let mut trait_methods = BTreeSet::new();
        for item in &trait_impl.items {
            match item {
                ImplItem::Type(ty) => {
                    trait_self_items.insert(ty.ident.to_string());
                }
                ImplItem::Const(c) => {
                    trait_self_items.insert(c.ident.to_string());
                }
                ImplItem::Fn(f) => {
                    let name = f.sig.ident.to_string();
                    trait_self_items.insert(name.clone());
                    trait_methods.insert(name);
                }
                _ => {}
            }
        }
        Self {
            generics_check,
            trait_self_items,
            trait_methods,
            trait_in_scope,
        }
    }
}

pub fn check_trait_method_movable(
    meta: &TraitImplMeta,
    sig: &Signature,
    block: &Block,
) -> Result<(), &'static str> {
    meta.generics_check?;

    let mut scanner = TraitDependencyScan {
        trait_self_items: &meta.trait_self_items,
        trait_methods: &meta.trait_methods,
        trait_in_scope: meta.trait_in_scope,
        uses_trait_self_item: false,
        calls_unimported_trait_method: false,
    };
    scanner.visit_signature(sig);
    scanner.visit_block(block);

    if scanner.uses_trait_self_item {
        return Err("uses trait Self:: item");
    }
    if scanner.calls_unimported_trait_method {
        return Err("calls trait method without trait in scope");
    }
    Ok(())
}

pub fn type_has_non_static_lifetime(ty: &Type) -> bool {
    let mut finder = NonStaticLifetimeFinder::default();
    finder.visit_type(ty);
    finder.found
}

#[derive(Default)]
struct NonStaticLifetimeFinder {
    found: bool,
}

impl<'ast> Visit<'ast> for NonStaticLifetimeFinder {
    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if reference
            .lifetime
            .as_ref()
            .is_none_or(|lt| lt.ident != "static")
        {
            self.found = true;
        }
        visit::visit_type_reference(self, reference);
    }

    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        if lifetime.ident != "static" {
            self.found = true;
        }
    }
}

#[derive(Default)]
struct Unsupported {
    impl_trait: bool,
    never: bool,
    borrowed_result: bool,
    nested_callable_borrows: bool,
}

impl<'ast> Visit<'ast> for Unsupported {
    fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
        self.impl_trait = true;
    }

    fn visit_type_never(&mut self, _: &'ast syn::TypeNever) {
        self.never = true;
    }

    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if reference
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident != "static")
        {
            self.borrowed_result = true;
        }
        visit::visit_type_reference(self, reference);
    }

    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        if lifetime.ident != "static" {
            self.borrowed_result = true;
        }
    }

    fn visit_type_bare_fn(&mut self, function: &'ast syn::TypeBareFn) {
        let mut nested = Self::default();
        visit::visit_type_bare_fn(&mut nested, function);
        self.nested_callable_borrows |= nested.borrowed_result || function.lifetimes.is_some();
    }

    fn visit_parenthesized_generic_arguments(
        &mut self,
        arguments: &'ast syn::ParenthesizedGenericArguments,
    ) {
        let mut nested = Self::default();
        visit::visit_parenthesized_generic_arguments(&mut nested, arguments);
        self.nested_callable_borrows |= nested.borrowed_result;
    }

    fn visit_bound_lifetimes(&mut self, lifetimes: &'ast syn::BoundLifetimes) {
        self.nested_callable_borrows = true;
        visit::visit_bound_lifetimes(self, lifetimes);
    }
}

struct TraitDependencyScan<'a> {
    trait_self_items: &'a BTreeSet<String>,
    trait_methods: &'a BTreeSet<String>,
    trait_in_scope: bool,
    uses_trait_self_item: bool,
    calls_unimported_trait_method: bool,
}

impl<'ast> TraitDependencyScan<'ast> {
    fn check_path(&mut self, path: &Path) {
        if path.segments.len() >= 2 && path.segments[0].ident == "Self" {
            let member = path.segments[1].ident.to_string();
            if self.trait_self_items.contains(&member) {
                self.uses_trait_self_item = true;
            }
        }
    }

    fn scan_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        let list: Vec<TokenTree> = tokens.into_iter().collect();
        let mut i = 0;
        while i < list.len() {
            match &list[i] {
                TokenTree::Group(group) => self.scan_tokens(group.stream()),
                TokenTree::Ident(ident) if *ident == "Self" => {
                    if i + 3 < list.len()
                        && matches!(&list[i + 1], TokenTree::Punct(p) if p.as_char() == ':')
                        && matches!(&list[i + 2], TokenTree::Punct(p) if p.as_char() == ':')
                        && let TokenTree::Ident(member) = &list[i + 3]
                        && self.trait_self_items.contains(&member.to_string())
                    {
                        self.uses_trait_self_item = true;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

impl<'ast> Visit<'ast> for TraitDependencyScan<'ast> {
    fn visit_path(&mut self, path: &'ast Path) {
        self.check_path(path);
        visit::visit_path(self, path);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        if !self.trait_in_scope
            && matches!(&*node.receiver, Expr::Path(p) if p.path.is_ident("self"))
            && self.trait_methods.contains(&node.method.to_string())
        {
            self.calls_unimported_trait_method = true;
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast ExprMacro) {
        self.scan_tokens(node.mac.tokens.clone());
        visit::visit_expr_macro(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &'ast StmtMacro) {
        self.scan_tokens(node.mac.tokens.clone());
        visit::visit_stmt_macro(self, node);
    }
}
