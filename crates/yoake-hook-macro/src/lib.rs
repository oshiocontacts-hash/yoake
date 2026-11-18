use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::visit_mut::{self, VisitMut};
use syn::{FnArg, GenericParam, Ident, ItemFn, Lifetime, LitInt, LitStr, Pat, ReturnType, Token, Type};

enum RawOption {
    Method,
    Id(LitInt),
    TestId(LitInt),
    Key(LitStr),
    Kind(Ident),
    SelfTy(Type),
    Skip(LitStr),
}

impl Parse for RawOption {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name: Ident = input.parse()?;
        if name == "method" {
            return Ok(Self::Method);
        }
        let kind = name.to_string();
        if !matches!(kind.as_str(), "id" | "test_id" | "key" | "kind" | "self_ty" | "skip") {
            return Err(syn::Error::new(
                name.span(),
                "expected method, id, key, kind, test_id, self_ty, or skip",
            ));
        }
        input.parse::<Token![=]>()?;
        match kind.as_str() {
            "id" => Ok(Self::Id(input.parse()?)),
            "test_id" => Ok(Self::TestId(input.parse()?)),
            "key" => Ok(Self::Key(input.parse()?)),
            "kind" => Ok(Self::Kind(input.parse()?)),
            "self_ty" => Ok(Self::SelfTy(input.parse()?)),
            "skip" => Ok(Self::Skip(input.parse()?)),
            _ => unreachable!(),
        }
    }
}

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
    let options = Punctuated::<RawOption, Token![,]>::parse_terminated.parse2(attributes)?;
    let mut associated = false;
    let mut skip = false;
    let mut slot_id: Option<usize> = None;
    let mut test_id: Option<usize> = None;
    let mut key: Option<LitStr> = None;
    let mut kind: Option<Ident> = None;
    let mut self_ty: Option<Type> = None;
    for option in options {
        match option {
            RawOption::Method => associated = true,
            RawOption::Id(value) => {
                let id = value.base10_parse()?;
                if slot_id.replace(id).is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate id"));
                }
            }
            RawOption::TestId(value) => {
                let id = value.base10_parse()?;
                if test_id.replace(id).is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate test_id"));
                }
            }
            RawOption::Key(value) => {
                if key.replace(value.clone()).is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate key"));
                }
            }
            RawOption::Kind(value) => {
                if kind.is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate kind"));
                }
                kind = Some(value);
            }
            RawOption::SelfTy(value) => {
                if self_ty.replace(value.clone()).is_some() {
                    return Err(syn::Error::new_spanned(value, "duplicate self_ty"));
                }
            }
            RawOption::Skip(reason) => {
                if reason.value().trim().is_empty() {
                    return Err(syn::Error::new_spanned(reason, "skip needs a nonempty reason"));
                }
                skip = true;
            }
        }
    }
    let callsite = match &kind {
        Some(value) if value == "callsite" => true,
        Some(value) if value == "fn" => false,
        Some(value) => return Err(syn::Error::new(value.span(), "kind must be fn or callsite")),
        None => false,
    };
    let mut function: ItemFn = syn::parse2(item)?;
    if skip {
        if slot_id.is_some()
            || test_id.is_some()
            || key.is_some()
            || kind.is_some()
            || self_ty.is_some()
        {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "skip cannot be combined with id, test_id, key, kind, or self_ty",
            ));
        }
        return Ok(quote!(#function));
    }
    if test_id.is_some() && (slot_id.is_some() || key.is_some()) {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "test_id cannot be combined with id or key",
        ));
    }
    if callsite && test_id.is_some() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "callsite requires an engine id and key",
        ));
    }
    if test_id.is_none() && slot_id.is_some() != key.is_some() {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "engine hooks require both id = <usize> and key = \"...\"",
        ));
    }
    if self_ty.is_some() && !associated {
        return Err(syn::Error::new_spanned(
            &function.sig,
            "self_ty requires method",
        ));
    }
    let (slot_id, ledger_assert, claim) = if let Some(test_id) = test_id {
        if test_id >= 64 {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "test_id must be less than 64",
            ));
        }
        let claim_ident = format_ident!("__YOAKE_HOOK_CLAIM_{test_id}");
        let export_name = format!("__yoake_hook_test_slot_{test_id}");
        let claim = quote! {
            #[used]
            #[unsafe(export_name = #export_name)]
            static #claim_ident: u8 = 0;
        };
        (
            quote!(::yoake_hook_rt::TEST_ID_BASE + #test_id),
            quote!(),
            claim,
        )
    } else {
        let Some(slot_id) = slot_id else {
            return Err(syn::Error::new_spanned(
                &function.sig,
                "yoake_hook requires id = <usize>, key = \"...\", or test_id = <usize>",
            ));
        };
        let key = key.unwrap();
        let function_name = &function.sig.ident;
        if callsite && function_name.to_string() != format!("__yoake_cs_{slot_id}") {
            return Err(syn::Error::new_spanned(
                function_name,
                format!("callsite shim must be named __yoake_cs_{slot_id}"),
            ));
        }
        let name_assert = if callsite {
            quote!()
        } else {
            quote!(const _: () = ::yoake_hook_rt::assert_hook_name(#key, stringify!(#function_name));)
        };
        let assert = quote! {
            const _: () = ::yoake_hook_rt::assert_ledger_key(
                crate::__yoake_hook_keys::KEYS,
                #slot_id,
                #key,
            );
            #name_assert
        };
        let claim_ident = format_ident!("__YOAKE_HOOK_CLAIM_{slot_id}");
        let export_name = format!("__yoake_hook_slot_{slot_id}");
        let claim = quote! {
            #[used]
            #[unsafe(export_name = #export_name)]
            static #claim_ident: u8 = 0;
        };
        (quote!(#slot_id), assert, claim)
    };
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
    if callsite {
        prepare_callsite_lifetimes(&mut function.sig)?;
    }
    let mut result_type: Type = match &function.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, result) => *result.clone(),
    };
    let mut result_check = Unsupported::default();
    result_check.visit_type(&result_type);
    if result_check.opaque || (!callsite && result_check.borrowed_result) {
        return Err(syn::Error::new_spanned(
            &result_type,
            "S2 cannot hook opaque, never, or non-static borrowed results; use skip",
        ));
    }

    let orig_sig = {
        let mut sig = function.sig.clone();
        sig.ident = format_ident!("__yoake_orig_{}", function.sig.ident);
        sig
    };

    let mut lifetime_counts = LifetimeCounts::default();
    let mut raw_input_types = Vec::new();
    let mut outer_arg_names = Vec::new();
    for argument in &mut function.sig.inputs {
        match argument {
            FnArg::Receiver(receiver) => {
                if !associated || receiver.reference.is_none() || receiver.colon_token.is_some() {
                    return Err(syn::Error::new_spanned(
                        receiver,
                        "S2 borrowed receivers require method; by-value/typed receivers need skip",
                    ));
                }
                let receiver_type = *receiver.ty.clone();
                lifetime_counts.visit_type(&receiver_type);
                raw_input_types.push(receiver_type);
                outer_arg_names.push(quote!(self));
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
                pattern.mutability = None;
                let name = pattern.ident.clone();
                let argument_type = *argument.ty.clone();
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
                lifetime_counts.visit_type(&argument_type);
                raw_input_types.push(argument_type);
                outer_arg_names.push(quote!(#name));
            }
        }
    }

    let mut self_ty_has_lifetimes = false;
    if let Some(replacement) = &mut self_ty {
        let mut lifetimes = NormalizeSelfLifetimes::default();
        lifetimes.visit_type_mut(replacement);
        self_ty_has_lifetimes = lifetimes.count != 0;
        let mut replace = ReplaceSelf { replacement };
        replace.visit_type_mut(&mut result_type);
        for input_type in &mut raw_input_types {
            replace.visit_type_mut(input_type);
        }
    }
    // A named lifetime is "shared" when it appears twice among the inputs, or in an input and
    // in the result (a call-site shim `fn s<'r>(recv: &'r T<'_>) -> &'r U`); eliding it would
    // leave the result's borrow ambiguous.
    lifetime_counts.visit_type(&result_type);
    let has_shared_named = self_ty_has_lifetimes
        || lifetime_counts.counts.values().any(|&count| count >= 2);
    let mut cb_lifetimes = CallbackLifetimes::with_prefix("'__yoake_cb_");
    let mut callback_arg_types = Vec::new();
    for input_type in &raw_input_types {
        let mut ty = input_type.clone();
        cb_lifetimes.visit_type_mut(&mut ty);
        callback_arg_types.push(ty);
    }
    let mut callback_result_type = result_type.clone();
    cb_lifetimes.visit_type_mut(&mut callback_result_type);

    let (
        dyn_lifetimes,
        dyn_arg_types,
        dyn_result_type,
        call_lifetimes,
        call_arg_types,
        call_result_type,
    ) = if has_shared_named {
        let mut dyn_lt = CallbackLifetimes::with_prefix("'__yoake_dyn_");
        let mut dyn_types = Vec::new();
        let mut fn_lt = CallbackLifetimes::with_prefix("'__yoake_fn_");
        let mut fn_types = Vec::new();
        for input_type in &raw_input_types {
            let mut d_ty = input_type.clone();
            dyn_lt.visit_type_mut(&mut d_ty);
            dyn_types.push(d_ty);
            let mut f_ty = input_type.clone();
            fn_lt.visit_type_mut(&mut f_ty);
            fn_types.push(f_ty);
        }
        let mut dyn_result = result_type.clone();
        dyn_lt.visit_type_mut(&mut dyn_result);
        let mut call_result = result_type.clone();
        fn_lt.visit_type_mut(&mut call_result);
        (
            dyn_lt.bound,
            dyn_types,
            dyn_result,
            fn_lt.bound,
            fn_types,
            call_result,
        )
    } else {
        let mut elider = ElideNamedLifetimes;
        let mut types = Vec::new();
        for input_type in &raw_input_types {
            let mut ty = input_type.clone();
            elider.visit_type_mut(&mut ty);
            types.push(ty);
        }
        let mut elided_result = result_type.clone();
        elider.visit_type_mut(&mut elided_result);
        (
            Vec::new(),
            types.clone(),
            elided_result.clone(),
            Vec::new(),
            types,
            elided_result,
        )
    };

    let dyn_trait_bound = if dyn_lifetimes.is_empty() {
        quote!(FnMut(#(#dyn_arg_types),*) -> #dyn_result_type)
    } else {
        quote!(for<#(#dyn_lifetimes),*> FnMut(#(#dyn_arg_types),*) -> #dyn_result_type)
    };

    let orig_lifetime = Lifetime::new("'__yoake_orig", Span::call_site());
    let cb_bound_lifetimes = &cb_lifetimes.bound;
    let callback_type = quote!(
        for<#orig_lifetime #(, #cb_bound_lifetimes)*> fn(
            &#orig_lifetime mut (dyn #dyn_trait_bound + #orig_lifetime)
            #(, #callback_arg_types)*
        ) -> #callback_result_type
    );

    let cfg_attrs: Vec<_> = function
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("cfg"))
        .cloned()
        .collect();
    let allow_attrs: Vec<_> = function
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("allow"))
        .cloned()
        .collect();

    let mut non_lifetime_generics = function.sig.generics.clone();
    non_lifetime_generics.params = non_lifetime_generics
        .params
        .into_iter()
        .filter(|parameter| !matches!(parameter, GenericParam::Lifetime(_)))
        .collect();
    let type_const_args: Vec<_> = non_lifetime_generics
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
    let turbofish = if type_const_args.is_empty() {
        quote!()
    } else {
        quote!(::<#(#type_const_args),*>)
    };
    let dispatch_turbofish = if type_const_args.is_empty() {
        quote!()
    } else {
        quote!(::<#(#type_const_args,)* _>)
    };

    let mut call_generics = non_lifetime_generics.clone();
    let mut call_params_with_lt: Punctuated<GenericParam, Token![,]> = Punctuated::new();
    for lt in &call_lifetimes {
        call_params_with_lt.push(GenericParam::Lifetime(syn::LifetimeParam::new(lt.clone())));
    }
    for param in call_generics.params {
        call_params_with_lt.push(param);
    }
    call_generics.params = call_params_with_lt;
    let (call_impl_generics, _, call_where_clause) = call_generics.split_for_impl();

    let mut dispatch_generics = call_generics.clone();
    dispatch_generics.params.push(syn::parse_quote!(__YoakeF));
    let mut dispatch_where = dispatch_generics
        .where_clause
        .take()
        .unwrap_or_else(|| syn::WhereClause {
            where_token: Default::default(),
            predicates: Punctuated::new(),
        });
    dispatch_where.predicates.push(syn::parse_quote!(__YoakeF: #dyn_trait_bound));
    dispatch_generics.where_clause = Some(dispatch_where);
    let (dispatch_impl_generics, _, dispatch_where_clause) = dispatch_generics.split_for_impl();

    let arg_idents: Vec<_> = (0..call_arg_types.len())
        .map(|index| format_ident!("__yoake_arg_{index}"))
        .collect();
    let closure_idents: Vec<_> = (0..call_arg_types.len())
        .map(|index| format_ident!("__yoake_a_{index}"))
        .collect();

    let id_const = format_ident!("__YOAKE_HOOK_ID_{}", function.sig.ident);
    let orig_ident = orig_sig.ident.clone();
    let call_ident = format_ident!("__yoake_call_{}", function.sig.ident);
    let dispatch_ident = format_ident!("__yoake_dispatch_{}", function.sig.ident);
    let prefix = if associated { quote!(Self::) } else { quote!() };
    let orig_prefix = if self_ty.is_some()
        && matches!(function.sig.inputs.first(), Some(FnArg::Receiver(_)))
    {
        match call_arg_types.first().unwrap() {
            Type::Reference(receiver) => {
                let self_type = &receiver.elem;
                quote!(<#self_type>::)
            }
            self_type => quote!(<#self_type>::),
        }
    } else {
        prefix.clone()
    };

    let register_ident = format_ident!("__yoake_register_{}", function.sig.ident);
    let (register_impl_generics, register_ty_generics, register_where_clause) =
        non_lifetime_generics.split_for_impl();
    let registration = if associated {
        quote! {
            #(#cfg_attrs)*
            #[doc(hidden)]
            #[allow(dead_code, clippy::type_complexity)]
            pub(crate) fn #register_ident #register_impl_generics (
                callback: #callback_type,
                order: ::yoake_hook_rt::Order,
            ) -> Result<(), &'static str> #register_where_clause {
                ::yoake_hook_rt::__register_typed(
                    Self::#id_const,
                    ::std::any::TypeId::of::<#callback_type>(),
                    callback,
                    order,
                )
            }
        }
    } else {
        let alias_ident = format_ident!("__YoakeHook_{}", function.sig.ident);
        let alias_is_usable = non_lifetime_generics.params.iter().all(|parameter| {
            let ident = match parameter {
                GenericParam::Type(parameter) => &parameter.ident,
                GenericParam::Const(parameter) => &parameter.ident,
                GenericParam::Lifetime(_) => unreachable!(),
            };
            token_stream_uses_ident(&callback_type, ident)
        });
        let alias = alias_is_usable.then(|| {
            quote! {
                #(#cfg_attrs)*
                #[doc(hidden)]
                #[allow(non_camel_case_types, dead_code, clippy::type_complexity)]
                pub type #alias_ident #register_impl_generics =
                    #callback_type #register_where_clause;
            }
        });
        let callback_parameter = if alias_is_usable {
            quote!(#alias_ident #register_ty_generics)
        } else {
            callback_type.clone()
        };
        quote! {
            #alias
            #(#cfg_attrs)*
            #[doc(hidden)]
            #[allow(dead_code, clippy::type_complexity)]
            pub(crate) fn #register_ident #register_impl_generics (
                callback: #callback_parameter,
                order: ::yoake_hook_rt::Order,
            ) -> Result<(), &'static str> #register_where_clause {
                ::yoake_hook_rt::__register_typed(
                    #id_const,
                    ::std::any::TypeId::of::<#callback_parameter>(),
                    callback,
                    order,
                )
            }
        }
    };

    let original_body = function.block;
    function.block = syn::parse_quote!({
        #claim
        #ledger_assert
        if let Some(__yoake_chain) = ::yoake_hook_rt::SLOTS[#slot_id].load() {
            return #prefix #call_ident #turbofish (__yoake_chain #(, #outer_arg_names)*);
        }
        #prefix #orig_ident #turbofish (#(#outer_arg_names),*)
    });

    Ok(quote! {
        #(#cfg_attrs)*
        #[doc(hidden)]
        #[allow(non_upper_case_globals, dead_code)]
        pub const #id_const: usize = #slot_id;

        #registration

        #(#cfg_attrs)*
        #(#allow_attrs)*
        #[doc(hidden)]
        #[inline(always)]
        #[allow(
            dead_code,
            clippy::too_many_arguments,
            clippy::type_complexity,
            clippy::needless_lifetimes
        )]
        #orig_sig #original_body

        #(#cfg_attrs)*
        #(#allow_attrs)*
        #[doc(hidden)]
        #[cold]
        #[inline(never)]
        #[allow(
            dead_code,
            clippy::too_many_arguments,
            clippy::type_complexity,
            clippy::needless_lifetimes
        )]
        fn #call_ident #call_impl_generics (
            __yoake_chain: ::yoake_hook_rt::Snapshot
            #(, #arg_idents: #call_arg_types)*
        ) -> #call_result_type #call_where_clause {
            if let Some((__yoake_cb, __yoake_rest)) = __yoake_chain.next::<#callback_type>() {
                #prefix #dispatch_ident #dispatch_turbofish (
                    __yoake_cb,
                    |#(#closure_idents),*| #prefix #call_ident #turbofish (
                        __yoake_rest #(, #closure_idents)*
                    )
                    #(, #arg_idents)*
                )
            } else {
                #orig_prefix #orig_ident #turbofish (#(#arg_idents),*)
            }
        }

        #(#cfg_attrs)*
        #(#allow_attrs)*
        #[doc(hidden)]
        #[inline(always)]
        #[allow(
            dead_code,
            clippy::too_many_arguments,
            clippy::type_complexity,
            clippy::needless_lifetimes
        )]
        fn #dispatch_ident #dispatch_impl_generics (
            __yoake_cb: #callback_type,
            mut __yoake_next: __YoakeF
            #(, #arg_idents: #call_arg_types)*
        ) -> #call_result_type #dispatch_where_clause {
            __yoake_cb(&mut __yoake_next #(, #arg_idents)*)
        }

        #function
    })
}

fn token_stream_uses_ident(tokens: &proc_macro2::TokenStream, ident: &Ident) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(candidate) => candidate == *ident,
        proc_macro2::TokenTree::Group(group) => token_stream_uses_ident(&group.stream(), ident),
        _ => false,
    })
}

fn prepare_callsite_lifetimes(signature: &mut syn::Signature) -> syn::Result<()> {
    let ReturnType::Type(_, result) = &signature.output else { return Ok(()) };
    let mut elided_result = ElidedReferences::default();
    elided_result.visit_type(result);
    if !elided_result.0 {
        return Ok(());
    }
    let borrowed: Vec<usize> = signature
        .inputs
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let ty = match argument {
                FnArg::Receiver(receiver) => &receiver.ty,
                FnArg::Typed(argument) => &argument.ty,
            };
            let mut check = Unsupported::default();
            check.visit_type(ty);
            check.borrowed_result.then_some(index)
        })
        .collect();
    if borrowed.len() != 1 {
        return Err(syn::Error::new_spanned(
            &signature.output,
            "borrowed return with several borrowed inputs",
        ));
    }
    let input_ty = match &signature.inputs[borrowed[0]] {
        FnArg::Receiver(receiver) => &receiver.ty,
        FnArg::Typed(argument) => &argument.ty,
    };
    let mut input_lifetime = InputLifetime::default();
    input_lifetime.visit_type(input_ty);
    let has_named_lifetime = input_lifetime.named.is_some();
    let lifetime = input_lifetime
        .named
        .unwrap_or_else(|| Lifetime::new("'__yoake_site", Span::call_site()));
    if !has_named_lifetime {
        let mut parameters = Punctuated::new();
        parameters.push(GenericParam::Lifetime(syn::LifetimeParam::new(lifetime.clone())));
        parameters.extend(std::mem::take(&mut signature.generics.params));
        signature.generics.params = parameters;
    }
    let input_ty = match &mut signature.inputs[borrowed[0]] {
        FnArg::Receiver(receiver) => &mut receiver.ty,
        FnArg::Typed(argument) => &mut argument.ty,
    };
    let mut bind_input = BindElided {
        lifetime: lifetime.clone(),
        references: 0,
    };
    bind_input.visit_type_mut(input_ty);
    if bind_input.references == 0 {
        let mut bind_placeholder = BindPlaceholder(lifetime.clone());
        bind_placeholder.visit_type_mut(input_ty);
    }
    let ReturnType::Type(_, result) = &mut signature.output else { unreachable!() };
    BindElided {
        lifetime,
        references: 0,
    }
    .visit_type_mut(result);
    Ok(())
}

#[derive(Default)]
struct ElidedReferences(bool);

impl<'ast> Visit<'ast> for ElidedReferences {
    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if reference
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident == "_")
        {
            self.0 = true;
        }
        visit::visit_type_reference(self, reference);
    }
}

#[derive(Default)]
struct InputLifetime {
    seen: bool,
    named: Option<Lifetime>,
}

impl<'ast> Visit<'ast> for InputLifetime {
    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
        if !self.seen {
            self.seen = true;
            self.named = reference
                .lifetime
                .as_ref()
                .filter(|lifetime| lifetime.ident != "_")
                .cloned();
        }
        visit::visit_type_reference(self, reference);
    }
}

struct BindElided {
    lifetime: Lifetime,
    references: usize,
}

impl VisitMut for BindElided {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        self.references += 1;
        if reference
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident == "_")
        {
            reference.lifetime = Some(self.lifetime.clone());
        }
        visit_mut::visit_type_reference_mut(self, reference);
    }
}

struct BindPlaceholder(Lifetime);

impl VisitMut for BindPlaceholder {
    fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = self.0.clone();
        }
    }
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
struct NormalizeSelfLifetimes {
    named: BTreeMap<String, Lifetime>,
    count: usize,
}

impl NormalizeSelfLifetimes {
    fn fresh(&mut self) -> Lifetime {
        let lifetime = Lifetime::new(&format!("'__yoake_self_{}", self.count), Span::call_site());
        self.count += 1;
        lifetime
    }
}

impl VisitMut for NormalizeSelfLifetimes {
    fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
        if lifetime.ident == "static" {
            return;
        }
        if lifetime.ident == "_" {
            *lifetime = self.fresh();
            return;
        }
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

struct ReplaceSelf<'a> {
    replacement: &'a Type,
}

impl VisitMut for ReplaceSelf<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if matches!(ty, Type::Path(path) if path.qself.is_none() && path.path.is_ident("Self")) {
            *ty = self.replacement.clone();
        } else {
            visit_mut::visit_type_mut(self, ty);
        }
    }
}

struct CallbackLifetimes {
    prefix: &'static str,
    named: BTreeMap<String, Lifetime>,
    bound: Vec<Lifetime>,
}

impl CallbackLifetimes {
    fn with_prefix(prefix: &'static str) -> Self {
        Self {
            prefix,
            named: BTreeMap::new(),
            bound: Vec::new(),
        }
    }

    fn fresh(&mut self) -> Lifetime {
        let lifetime = Lifetime::new(
            &format!("{}{}", self.prefix, self.bound.len()),
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

#[derive(Default)]
struct LifetimeCounts {
    counts: BTreeMap<String, usize>,
}

impl<'ast> Visit<'ast> for LifetimeCounts {
    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        if lifetime.ident != "static" && lifetime.ident != "_" {
            *self.counts.entry(lifetime.ident.to_string()).or_insert(0) += 1;
        }
    }
}

struct ElideNamedLifetimes;

impl VisitMut for ElideNamedLifetimes {
    fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
        if lifetime.ident != "static" && lifetime.ident != "_" {
            *lifetime = Lifetime::new("'_", lifetime.span());
        }
    }

    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if let Some(lifetime) = &reference.lifetime
            && lifetime.ident != "static"
        {
            reference.lifetime = None;
        }
        visit_mut::visit_type_mut(self, &mut reference.elem);
        if let Type::TraitObject(object) = &mut *reference.elem {
            object.bounds = std::mem::take(&mut object.bounds)
                .into_iter()
                .filter(|bound| {
                    !matches!(bound, syn::TypeParamBound::Lifetime(lt) if lt.ident != "static")
                })
                .collect();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::expand;
    use quote::quote;

    fn rejected(function: proc_macro2::TokenStream, diagnostic: &str) {
        let error = expand(quote!(id = 1, key = "test::function"), function).unwrap_err();
        assert!(error.to_string().contains(diagnostic), "{error}");
    }

    #[test]
    fn engine_and_test_ids_validate_their_arguments() {
        let function = quote!(
            fn simple(x: u32) -> u32 {
                x
            }
        );
        let missing = expand(quote!(), function.clone()).unwrap_err();
        assert!(missing.to_string().contains("requires id"), "{missing}");
        let duplicate = expand(quote!(id = 1, id = 2), function.clone()).unwrap_err();
        assert!(duplicate.to_string().contains("duplicate id"), "{duplicate}");
        let combined = expand(quote!(skip = "no", id = 1), function.clone()).unwrap_err();
        assert!(combined.to_string().contains("skip"), "{combined}");
        let missing_key = expand(quote!(id = 1), function.clone()).unwrap_err();
        assert!(missing_key.to_string().contains("both id"), "{missing_key}");
        let key_without_id = expand(quote!(key = "test::simple"), function.clone()).unwrap_err();
        assert!(key_without_id.to_string().contains("both id"), "{key_without_id}");
        let expanded = expand(quote!(id = 17, key = "test::simple"), function.clone())
            .unwrap()
            .to_string();
        assert!(expanded.contains("__YOAKE_HOOK_ID_simple : usize = 17usize"));
        let compact: String = expanded.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            compact.contains("assert_ledger_key(crate::__yoake_hook_keys::KEYS,17usize,\"test::simple\","),
            "{expanded}"
        );
        assert!(expanded.contains("assert_hook_name (\"test::simple\" , stringify ! (simple))"));
        assert!(expanded.contains("SLOTS [17usize]"));
        assert!(expanded.contains("pub type __YoakeHook_simple"));
        assert!(expanded.contains("pub (crate) fn __yoake_register_simple"));
        assert!(expanded.contains("export_name = \"__yoake_hook_slot_17\""));

        let test = expand(quote!(test_id = 3), function.clone()).unwrap().to_string();
        assert!(test.contains("TEST_ID_BASE + 3usize"));
        assert!(!test.contains("assert_ledger_key"));
        assert!(test.contains("export_name = \"__yoake_hook_test_slot_3\""));
        assert!(expand(quote!(test_id = 64), function.clone()).is_err());
        assert!(expand(quote!(test_id = 1, key = "no"), function).is_err());
    }

    #[test]
    fn callsite_shims_validate_names_and_support_borrowed_returns() {
        let function = quote!(
            fn __yoake_cs_17(recv: &Game) -> Option<&ItemStack> {
                recv.held()
            }
        );
        let expanded = expand(
            quote!(id = 17, key = "fn:103@call:fn:2207", kind = callsite),
            function.clone(),
        )
        .unwrap()
        .to_string();
        assert!(expanded.contains("assert_ledger_key"));
        assert!(!expanded.contains("assert_hook_name"));
        assert!(expanded.contains("'__yoake_site"));
        assert!(expanded.contains("pub (crate) fn __yoake_register___yoake_cs_17"));

        let wrong = expand(
            quote!(id = 17, key = "fn:103@call:fn:2207", kind = callsite),
            quote!(fn held(recv: &Game) -> Option<&ItemStack> { recv.held() }),
        )
        .unwrap_err();
        assert!(wrong.to_string().contains("__yoake_cs_17"), "{wrong}");
        let ambiguous = expand(
            quote!(id = 17, key = "fn:103@call:fn:2207", kind = callsite),
            quote!(fn __yoake_cs_17(left: &str, right: &str) -> &str { left }),
        )
        .unwrap_err();
        assert!(ambiguous.to_string().contains("several borrowed inputs"), "{ambiguous}");
        assert!(expand(
            quote!(id = 17, key = "x", kind = callsite, kind = callsite),
            function,
        )
        .unwrap_err()
        .to_string()
        .contains("duplicate kind"));
    }

    #[test]
    fn self_type_is_accepted_for_borrowed_impl_lifetimes() {
        let function = quote!(
            fn bump(&mut self, amount: u32) -> u32 {
                amount
            }
        );
        let expanded = expand(
            quote!(method, id = 1, key = "test::Holder::bump", self_ty = Holder<'_>),
            function.clone(),
        );
        assert!(expanded.is_ok(), "{expanded:?}");
        let expanded = expanded.unwrap().to_string();
        assert!(expanded.contains("pub (crate) fn __yoake_register_bump"));
        let error = expand(quote!(id = 1, key = "no", self_ty = Holder<'_>), function)
            .unwrap_err();
        assert!(error.to_string().contains("requires method"), "{error}");
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
