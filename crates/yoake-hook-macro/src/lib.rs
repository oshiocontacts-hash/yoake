use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};
use syn::{FnArg, GenericParam, ItemFn, Lifetime, Meta, Pat, ReturnType, Token, Type};

#[proc_macro_attribute]
pub fn yoake_hook(attributes: TokenStream, item: TokenStream) -> TokenStream {
    match expand(attributes.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand(
    attributes: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> syn::Result<proc_macro2::TokenStream> {
    let options = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attributes)?;
    let mut associated = false;
    let mut skip = false;
    for option in options {
        match option {
            Meta::Path(path) if path.is_ident("method") => associated = true,
            Meta::NameValue(value) if value.path.is_ident("skip") => {
                if let syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(reason),
                    ..
                }) = &value.value
                    && !reason.value().trim().is_empty()
                {
                    skip = true;
                } else {
                    return Err(syn::Error::new_spanned(
                        value,
                        "skip needs a nonempty reason",
                    ));
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected method or skip = \"reason\"",
                ));
            }
        }
    }
    let mut function: ItemFn = syn::parse2(item)?;
    if skip {
        return Ok(quote!(#function));
    }
    let signature = &function.sig;
    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || signature.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            signature,
            "S2 requires a safe synchronous non-const Rust function; use skip with a reason",
        ));
    }
    for parameter in signature.generics.type_params() {
        let inline_static = parameter.bounds.iter().any(is_static_bound);
        let where_static = signature.generics.where_clause.as_ref().is_some_and(|clause| {
            clause.predicates.iter().any(|predicate| {
                matches!(predicate, syn::WherePredicate::Type(bound)
                    if matches!(&bound.bounded_ty, Type::Path(path) if path.path.is_ident(&parameter.ident))
                        && bound.bounds.iter().any(is_static_bound))
            })
        });
        if !inline_static && !where_static {
            return Err(syn::Error::new_spanned(
                parameter,
                "S2 generic types require an existing explicit 'static bound; use skip rather than restricting vanilla callers",
            ));
        }
    }
    if signature
        .generics
        .lifetimes()
        .any(|parameter| !parameter.bounds.is_empty())
    {
        return Err(syn::Error::new_spanned(
            signature,
            "S2 does not model lifetime bounds; use skip",
        ));
    }
    if function
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("track_caller"))
    {
        return Err(syn::Error::new_spanned(
            signature,
            "S2 body wrapping does not preserve track_caller; use skip",
        ));
    }
    if let Some(clause) = &signature.generics.where_clause {
        let mut bounds_check = Unsupported::default();
        bounds_check.visit_where_clause(clause);
        if bounds_check.borrowed_result {
            return Err(syn::Error::new_spanned(
                clause,
                "S2 does not model lifetime-dependent where bounds; use skip",
            ));
        }
    }
    let result_type: Type = match &signature.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, result) => *result.clone(),
    };
    let mut result_check = Unsupported::default();
    result_check.visit_type(&result_type);
    if result_check.opaque || result_check.borrowed_result {
        return Err(syn::Error::new_spanned(
            &result_type,
            "S2 cannot hook opaque, never, or non-static borrowed results; use skip",
        ));
    }
    let mut lifetimes = CallbackLifetimes::default();
    let mut callback_arguments = Vec::new();
    let mut invoke_arguments = Vec::new();
    for argument in &mut function.sig.inputs {
        match argument {
            FnArg::Receiver(receiver) => {
                if !associated || receiver.reference.is_none() || receiver.colon_token.is_some() {
                    return Err(syn::Error::new_spanned(
                        receiver,
                        "S2 borrowed receivers require method; by-value/typed receivers need skip",
                    ));
                }
                let mut receiver_type = *receiver.ty.clone();
                lifetimes.visit_type_mut(&mut receiver_type);
                callback_arguments.push(quote!(#receiver_type));
                invoke_arguments.push(quote!(self));
            }
            FnArg::Typed(argument) => {
                let Pat::Ident(pattern) = &mut *argument.pat else {
                    return Err(syn::Error::new_spanned(
                        &argument.pat,
                        "S2 requires identifier arguments; destructuring needs skip",
                    ));
                };
                if pattern.by_ref.is_some() || pattern.subpat.is_some() {
                    return Err(syn::Error::new_spanned(
                        pattern,
                        "S2 does not support ref/subpatterns",
                    ));
                }
                pattern.mutability = Some(syn::parse_quote!(mut));
                let name = &pattern.ident;
                let mut argument_type = *argument.ty.clone();
                let mut input_check = Unsupported::default();
                input_check.visit_type(&argument_type);
                if input_check.opaque {
                    return Err(syn::Error::new_spanned(
                        argument_type,
                        "S2 impl Trait inputs need skip",
                    ));
                }
                if input_check.nested_callable_borrows {
                    return Err(syn::Error::new_spanned(
                        argument_type,
                        "S2 nested callable borrows need independent lifetime binders; use skip",
                    ));
                }
                lifetimes.visit_type_mut(&mut argument_type);
                callback_arguments.push(quote!(&mut #argument_type));
                invoke_arguments.push(quote!(&mut #name));
            }
        }
    }
    let bound_lifetimes = &lifetimes.bound;
    let binder = if bound_lifetimes.is_empty() {
        quote!()
    } else {
        quote!(for<#(#bound_lifetimes),*>)
    };
    let head_type =
        quote!(#binder fn(#(#callback_arguments),*) -> ::std::ops::ControlFlow<#result_type>);
    let returned_type = quote!(fn(&mut #result_type));
    let getter = format_ident!("__yoake_hook_{}", function.sig.ident);
    let visibility = &function.vis;
    let cfg: Vec<_> = function
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("cfg"))
        .cloned()
        .collect();
    let mut getter_generics = function.sig.generics.clone();
    getter_generics.params = getter_generics
        .params
        .into_iter()
        .filter(|parameter| !matches!(parameter, GenericParam::Lifetime(_)))
        .collect();
    let (getter_parameters, _, where_clause) = getter_generics.split_for_impl();
    let type_arguments: Vec<_> = getter_generics
        .params
        .iter()
        .map(|parameter| match parameter {
            GenericParam::Type(parameter) => {
                let name = &parameter.ident;
                quote!(#name)
            }
            GenericParam::Const(parameter) => {
                let name = &parameter.ident;
                quote!(#name)
            }
            GenericParam::Lifetime(_) => unreachable!(),
        })
        .collect();
    let turbofish = if type_arguments.is_empty() {
        quote!()
    } else {
        quote!(::<#(#type_arguments),*>)
    };
    let call_getter = if associated {
        quote!(Self::#getter #turbofish ())
    } else {
        quote!(#getter #turbofish ())
    };
    let original_body = function.block;
    function.attrs.push(syn::parse_quote!(#[allow(unused_mut)]));
    function.block = syn::parse_quote!({
        let __yoake_site = #call_getter;
        let __yoake_head = if let Some(__yoake_chain) = __yoake_site.load() {
            __yoake_site.head(&__yoake_chain, |__yoake_callback| __yoake_callback(#(#invoke_arguments),*))
        } else {
            ::std::ops::ControlFlow::Continue(())
        };
        let mut __yoake_result = match __yoake_head {
            ::std::ops::ControlFlow::Break(__yoake_replacement) => __yoake_replacement,
            ::std::ops::ControlFlow::Continue(()) => (|| #original_body)(),
        };
        if let Some(__yoake_chain) = __yoake_site.load() {
            __yoake_site.returned(&__yoake_chain, |__yoake_callback| __yoake_callback(&mut __yoake_result));
        }
        __yoake_result
    });
    Ok(quote! {
        #(#cfg)*
        #[doc(hidden)]
        #[inline(always)]
        #visibility fn #getter #getter_parameters () -> ::yoake_hook_rt::Site<#head_type, #returned_type>
        #where_clause
        {
            static SLOT: ::yoake_hook_rt::Slot = ::yoake_hook_rt::Slot::new();
            ::yoake_hook_rt::Site::new(&SLOT)
        }
        #function
    })
}

fn is_static_bound(bound: &syn::TypeParamBound) -> bool {
    matches!(bound, syn::TypeParamBound::Lifetime(lifetime) if lifetime.ident == "static")
}

#[derive(Default)]
struct Unsupported {
    opaque: bool,
    borrowed_result: bool,
    nested_callable_borrows: bool,
}

impl<'ast> Visit<'ast> for Unsupported {
    fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
        self.opaque = true;
    }

    fn visit_type_never(&mut self, _: &'ast syn::TypeNever) {
        self.opaque = true;
    }

    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if !reference
            .lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.ident == "static")
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

#[derive(Default)]
struct CallbackLifetimes {
    named: BTreeMap<String, Lifetime>,
    bound: Vec<Lifetime>,
}

impl CallbackLifetimes {
    fn fresh(&mut self) -> Lifetime {
        let lifetime = Lifetime::new(
            &format!("'__yoake_borrow_{}", self.bound.len()),
            Span::call_site(),
        );
        self.bound.push(lifetime.clone());
        lifetime
    }
}

impl VisitMut for CallbackLifetimes {
    fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
        if lifetime.ident == "static" {
            return;
        }
        if lifetime.ident == "_" {
            *lifetime = self.fresh();
        } else {
            let key = lifetime.ident.to_string();
            let replacement = if let Some(existing) = self.named.get(&key) {
                existing.clone()
            } else {
                let fresh = self.fresh();
                self.named.insert(key, fresh.clone());
                fresh
            };
            *lifetime = replacement;
        }
    }

    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(self.fresh());
        } else if let Some(lifetime) = &mut reference.lifetime {
            self.visit_lifetime_mut(lifetime);
        }
        visit_mut::visit_type_mut(self, &mut reference.elem);
        if let Type::TraitObject(object) = &mut *reference.elem
            && !object
                .bounds
                .iter()
                .any(|bound| matches!(bound, syn::TypeParamBound::Lifetime(_)))
        {
            object.bounds.push(syn::TypeParamBound::Lifetime(
                reference.lifetime.clone().unwrap(),
            ));
            let element = &reference.elem;
            reference.elem = Box::new(syn::parse_quote!((#element)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    use quote::quote;

    fn rejected(function: proc_macro2::TokenStream, diagnostic: &str) {
        let error = expand(quote!(), function).unwrap_err();
        assert!(error.to_string().contains(diagnostic), "{error}");
    }

    #[test]
    fn unsupported_calling_conventions_are_rejected() {
        for function in [
            quote!(
                async fn task() {}
            ),
            quote!(
                const fn constant() {}
            ),
            quote!(
                unsafe fn unchecked() {}
            ),
            quote!(
                extern "C" fn foreign() {}
            ),
        ] {
            rejected(function, "safe synchronous non-const Rust function");
        }
    }

    #[test]
    fn unconstrained_generics_do_not_gain_restrictive_bounds() {
        rejected(
            quote!(
                fn identity<Value>(value: Value) -> Value {
                    value
                }
            ),
            "existing explicit 'static bound",
        );
    }

    #[test]
    fn borrowed_results_are_rejected() {
        rejected(
            quote!(
                fn borrow(value: &str) -> &str {
                    value
                }
            ),
            "non-static borrowed results",
        );
        rejected(
            quote!(
                fn opaque() -> impl Copy {
                    1_u32
                }
            ),
            "opaque",
        );
        rejected(
            quote!(
                fn never() -> ! {
                    panic!()
                }
            ),
            "never",
        );
    }

    #[test]
    fn lifetime_relationships_are_not_silently_erased() {
        rejected(
            quote!(
                fn bounded<'long: 'short, 'short>(value: &'long str) {}
            ),
            "lifetime bounds",
        );
        rejected(
            quote!(
                fn bounded<'input, Value: 'static>(value: &'input Value)
                where
                    Value: 'input,
                {
                }
            ),
            "lifetime-dependent where bounds",
        );
    }

    #[test]
    fn higher_order_borrows_are_rejected() {
        rejected(
            quote!(
                fn apply(callback: &dyn Fn(&str)) {}
            ),
            "independent lifetime binders",
        );
        rejected(
            quote!(
                fn apply(callback: for<'input> fn(&'input str)) {}
            ),
            "independent lifetime binders",
        );
    }

    #[test]
    fn destructured_arguments_require_an_explicit_exclusion() {
        rejected(
            quote!(
                fn pair((first, second): (u32, u32)) {}
            ),
            "destructuring needs skip",
        );
    }

    #[test]
    fn caller_location_is_not_silently_changed() {
        rejected(
            quote!(
                #[track_caller]
                fn located() {}
            ),
            "does not preserve track_caller",
        );
    }

    #[test]
    fn skip_preserves_the_unhookable_function() {
        let function = quote!(
            async fn task() -> impl Copy {
                1_u32
            }
        );
        let expanded = expand(
            quote!(skip = "future state has no synchronous RETURN"),
            function.clone(),
        )
        .unwrap();
        assert_eq!(expanded.to_string(), function.to_string());
        assert!(expand(quote!(skip = ""), function).is_err());
    }
}
