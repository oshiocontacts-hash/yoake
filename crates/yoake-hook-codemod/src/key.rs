use quote::quote;
use syn::visit_mut::{self, VisitMut};
use syn::{
    AngleBracketedGenericArguments, Expr, FnArg, GenericParam, Ident, Member,
    ParenthesizedGenericArguments, PatTuple, Path, Signature, Type, TypeBareFn, TypeTuple,
};
use yoake_hook_ledger::Ledger;

pub fn normalize_crate_name(name: &str) -> String {
    name.replace('-', "_")
}

pub fn module_prefix(crate_ident: &str, mod_path: &[String]) -> String {
    if mod_path.is_empty() {
        crate_ident.to_string()
    } else {
        format!("{crate_ident}::{}", mod_path.join("::"))
    }
}

pub fn type_last_segment_name(mut ty: &Type) -> String {
    loop {
        match ty {
            Type::Group(group) => ty = &group.elem,
            Type::Paren(paren) => ty = &paren.elem,
            Type::Path(type_path) => {
                if let Some(last) = type_path.path.segments.last() {
                    return last.ident.to_string();
                }
                break;
            }
            _ => break,
        }
    }
    quote!(#ty)
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn trait_last_segment_name(path: &Path) -> String {
    path.segments
        .last()
        .map(|seg| seg.ident.to_string())
        .unwrap_or_default()
}

pub fn free_fn_key(prefix: &str, ident: &Ident) -> String {
    format!("{prefix}::{ident}")
}

pub fn inherent_method_key(prefix: &str, self_ty: &Type, ident: &Ident) -> String {
    let self_name = type_last_segment_name(self_ty);
    format!("{prefix}::{self_name}::{ident}")
}

pub fn trait_method_key(
    prefix: &str,
    self_ty: &Type,
    trait_path: &Path,
    ident: &Ident,
) -> String {
    let self_name = type_last_segment_name(self_ty);
    let trait_name = trait_last_segment_name(trait_path);
    format!("{prefix}::<{self_name} as {trait_name}>::{ident}")
}

pub fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_uppercase() {
            let prev_is_lower_or_digit =
                i > 0 && (chars[i - 1].is_lowercase() || chars[i - 1].is_ascii_digit());
            let next_is_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
            let prev_is_upper = i > 0 && chars[i - 1].is_uppercase();
            if i > 0 && (prev_is_lower_or_digit || (prev_is_upper && next_is_lower)) {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn trait_helper_name(trait_path: &Path, method_ident: &Ident) -> String {
    let trait_snake = to_snake_case(&trait_last_segment_name(trait_path));
    format!("__yoake_trait_{trait_snake}_{method_ident}")
}

#[derive(Default)]
struct CanonicalSigCleanup;

impl VisitMut for CanonicalSigCleanup {
    fn visit_type_tuple_mut(&mut self, node: &mut TypeTuple) {
        if node.elems.len() != 1 && node.elems.trailing_punct() {
            node.elems.pop_punct();
        }
        visit_mut::visit_type_tuple_mut(self, node);
    }

    fn visit_pat_tuple_mut(&mut self, node: &mut PatTuple) {
        if node.elems.len() != 1 && node.elems.trailing_punct() {
            node.elems.pop_punct();
        }
        visit_mut::visit_pat_tuple_mut(self, node);
    }

    fn visit_type_bare_fn_mut(&mut self, node: &mut TypeBareFn) {
        if node.inputs.trailing_punct() {
            node.inputs.pop_punct();
        }
        visit_mut::visit_type_bare_fn_mut(self, node);
    }

    fn visit_angle_bracketed_generic_arguments_mut(
        &mut self,
        node: &mut AngleBracketedGenericArguments,
    ) {
        if node.args.trailing_punct() {
            node.args.pop_punct();
        }
        visit_mut::visit_angle_bracketed_generic_arguments_mut(self, node);
    }

    fn visit_parenthesized_generic_arguments_mut(
        &mut self,
        node: &mut ParenthesizedGenericArguments,
    ) {
        if node.inputs.trailing_punct() {
            node.inputs.pop_punct();
        }
        visit_mut::visit_parenthesized_generic_arguments_mut(self, node);
    }
}

pub fn canonical_signature_tokens(sig: &Signature) -> String {
    let mut clean = sig.clone();
    CanonicalSigCleanup.visit_signature_mut(&mut clean);

    let mut out = String::new();
    if clean.constness.is_some() {
        out.push_str("const ");
    }
    if clean.asyncness.is_some() {
        out.push_str("async ");
    }
    if clean.unsafety.is_some() {
        out.push_str("unsafe ");
    }
    if let Some(abi) = &clean.abi {
        out.push_str(&quote!(#abi).to_string());
        out.push(' ');
    }
    out.push_str("fn ");
    out.push_str(&clean.ident.to_string());

    if !clean.generics.params.is_empty() {
        let params: Vec<String> = clean
            .generics
            .params
            .iter_mut()
            .map(|param| {
                match param {
                    GenericParam::Lifetime(p) => p.attrs.clear(),
                    GenericParam::Type(p) => p.attrs.clear(),
                    GenericParam::Const(p) => p.attrs.clear(),
                }
                quote!(#param).to_string()
            })
            .collect();
        out.push('<');
        out.push_str(&params.join(", "));
        out.push('>');
    }

    let mut inputs: Vec<String> = clean
        .inputs
        .iter_mut()
        .map(|arg| match arg {
            FnArg::Receiver(receiver) => {
                receiver.attrs.clear();
                quote!(#receiver).to_string()
            }
            FnArg::Typed(typed) => {
                let pat = &typed.pat;
                let ty = &typed.ty;
                format!("{}: {}", quote!(#pat), quote!(#ty))
            }
        })
        .collect();
    if let Some(variadic) = &mut clean.variadic {
        variadic.attrs.clear();
        variadic.comma = None;
        inputs.push(quote!(#variadic).to_string());
    }
    out.push('(');
    out.push_str(&inputs.join(", "));
    out.push(')');

    if let syn::ReturnType::Type(_, ty) = &clean.output {
        out.push_str(" -> ");
        out.push_str(&quote!(#ty).to_string());
    }

    if let Some(where_clause) = &clean.generics.where_clause
        && !where_clause.predicates.is_empty()
    {
        let preds: Vec<String> = where_clause
            .predicates
            .iter()
            .map(|pred| quote!(#pred).to_string())
            .collect();
        out.push_str(" where ");
        out.push_str(&preds.join(", "));
    }

    out
}

pub fn signature_hash(sig: &Signature) -> String {
    Ledger::sig_hash(&canonical_signature_tokens(sig))
}

pub fn callsite_key(caller_key: &str, callee_path: &str, ordinal: usize) -> String {
    format!("{caller_key}@call:{callee_path}#{ordinal}")
}

pub fn store_key(caller_key: &str, field_path: &str, ordinal: usize) -> String {
    format!("{caller_key}@store:{field_path}#{ordinal}")
}

fn unwrap_expr_parens(mut expr: &Expr) -> &Expr {
    loop {
        match expr {
            Expr::Paren(p) => expr = &p.expr,
            Expr::Group(g) => expr = &g.expr,
            _ => return expr,
        }
    }
}

pub fn call_expr_callee(func: &Expr) -> (String, bool) {
    match unwrap_expr_parens(func) {
        Expr::Path(ep) => {
            let segs: Vec<String> = ep
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            if let Some(qself) = &ep.qself {
                let self_name = type_last_segment_name(&qself.ty);
                if qself.position == 0 {
                    (format!("{self_name}::{}", segs.join("::")), false)
                } else if qself.position <= segs.len() {
                    let trait_name = &segs[qself.position - 1];
                    let rest = &segs[qself.position..];
                    if rest.is_empty() {
                        (format!("<{self_name} as {trait_name}>"), false)
                    } else {
                        (
                            format!("<{self_name} as {trait_name}>::{}", rest.join("::")),
                            false,
                        )
                    }
                } else {
                    (segs.join("::"), false)
                }
            } else {
                (segs.join("::"), false)
            }
        }
        Expr::Field(ef) => {
            let member = match &ef.member {
                Member::Named(id) => id.to_string(),
                Member::Unnamed(idx) => idx.index.to_string(),
            };
            (format!("?::{member}"), true)
        }
        _ => ("?::<expr>".to_string(), true),
    }
}

pub fn assign_field_path(left: &Expr) -> Option<String> {
    match unwrap_expr_parens(left) {
        Expr::Field(ef) => {
            let member = match &ef.member {
                Member::Named(id) => id.to_string(),
                Member::Unnamed(idx) => idx.index.to_string(),
            };
            let base = match unwrap_expr_parens(&ef.base) {
                Expr::Path(ep) if ep.qself.is_none() => ep
                    .path
                    .segments
                    .iter()
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::"),
                Expr::Field(_) => assign_field_path(&ef.base)?,
                _ => "?".to_string(),
            };
            Some(format!("{base}.{member}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_handles_pascal_and_acronyms() {
        assert_eq!(to_snake_case("EntityRules"), "entity_rules");
        assert_eq!(to_snake_case("ClimbingWorld"), "climbing_world");
        assert_eq!(to_snake_case("IOStream"), "io_stream");
    }

    #[test]
    fn type_last_segment_drops_lifetimes_and_generics() {
        let ty: Type = syn::parse_str("crate::game::Game<'_>").unwrap();
        assert_eq!(type_last_segment_name(&ty), "Game");
        let ty2: Type = syn::parse_str("StandClimbing<'a>").unwrap();
        assert_eq!(type_last_segment_name(&ty2), "StandClimbing");
    }

    #[test]
    fn signature_hash_is_invariant_under_rustfmt_trailing_commas_and_comments() {
        let one_line: syn::ItemFn = syn::parse_str(
            "pub fn f<'a, T: Clone>(/* c */ &mut self, x: (u32, u64), y: Vec<T>) -> Result<u32, ()> where T: Send {}",
        )
        .unwrap();
        let multi_line: syn::ItemFn = syn::parse_str(
            "pub fn f<
                'a,
                // lifetime and type comment
                T: Clone,
            >(
                &mut self,
                // param comment
                #[allow(unused)]
                x: (
                    u32,
                    u64,
                ),
                y: Vec<
                    T,
                >,
            ) -> Result<u32, ()>
            where
                T: Send,
            {}",
        )
        .unwrap();
        assert_eq!(
            canonical_signature_tokens(&one_line.sig),
            canonical_signature_tokens(&multi_line.sig)
        );
        assert_eq!(signature_hash(&one_line.sig), signature_hash(&multi_line.sig));
    }

    #[test]
    fn one_tuple_preserves_trailing_comma_in_sig_hash() {
        let one_tuple: syn::ItemFn = syn::parse_str("fn f(x: (u32,)) {}").unwrap();
        let parens: syn::ItemFn = syn::parse_str("fn f(x: (u32)) {}").unwrap();
        assert_ne!(signature_hash(&one_tuple.sig), signature_hash(&parens.sig));
    }

    #[test]
    fn callsite_and_store_helpers_format_expected_keys() {
        assert_eq!(
            callsite_key(
                "horizon_client::entities::ArmorStand::tick_settle",
                "horizon_core::geometry::Vec3::add_vector",
                0
            ),
            "horizon_client::entities::ArmorStand::tick_settle@call:horizon_core::geometry::Vec3::add_vector#0"
        );
        assert_eq!(
            store_key("demo::m::Foo::bar", "self.velocity", 2),
            "demo::m::Foo::bar@store:self.velocity#2"
        );

        let call: syn::ExprCall = syn::parse_str("Vec3::add_vector::<f64>(a, b)").unwrap();
        assert_eq!(call_expr_callee(&call.func), ("Vec3::add_vector".to_string(), false));

        let assign: syn::ExprAssign = syn::parse_str("self.pos.x = 1.0").unwrap();
        assert_eq!(assign_field_path(&assign.left).as_deref(), Some("self.pos.x"));

        let local_assign: syn::ExprAssign = syn::parse_str("step = step + 1").unwrap();
        assert_eq!(assign_field_path(&local_assign.left), None);
    }
}
