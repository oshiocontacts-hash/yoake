use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{ToTokens, format_ident, quote};
use syn::{FnArg, ItemFn, Pat, ReturnType, Type};
use yoake_inject_spec::{inspect_signature_with_options, parse_options, success_type};

#[proc_macro_attribute]
pub fn yoake_inject(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let result = syn::parse::<ItemFn>(item).and_then(|function| expand(attribute.into(), function));
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

fn expand(attribute: TokenStream2, function: ItemFn) -> syn::Result<TokenStream2> {
    let options = parse_options(attribute)?;
    let signature = &function.sig;
    let info = inspect_signature_with_options(signature, &options)?;

    let mut encode = Vec::new();
    let mut decode = Vec::new();
    let mut kinds = Vec::new();
    let mut scoped = false;
    let iterator = format_ident!("__yoake_inject_iter", span = Span::mixed_site());
    let scope_name = format_ident!("__yoake_inject_scope", span = Span::mixed_site());
    let receiver = signature.receiver().is_some();
    for (argument, shape) in signature.inputs.iter().zip(&info.arguments) {
        let kind = format_ident!("{}", shape.variant);
        kinds.push(quote!(::yoake_inject_rt::ValueType::#kind));
        let FnArg::Typed(argument) = argument else {
            scoped = true;
            encode.push(quote!(#scope_name.handle("self", None)));
            decode.push(quote!(let _ = #iterator.next();));
            continue;
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            unreachable!()
        };
        let name = &pattern.ident;
        let ty = &argument.ty;
        if shape.variant == "Handle" {
            scoped = true;
            let entity = options
                .entities
                .iter()
                .any(|entity| name == entity.as_str());
            let native_type = if entity {
                "entity".to_owned()
            } else {
                ty.to_token_stream().to_string()
            };
            let snapshot_type = match ty.as_ref() {
                Type::Reference(reference) => reference.elem.as_ref(),
                native => native,
            };
            let snapshot = if entity {
                quote!(Some(::yoake_inject_rt::Value::Unsigned(u64::from(#name))))
            } else if matches!(snapshot_type, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "ItemStack"))
            {
                let reference = if matches!(ty.as_ref(), Type::Reference(_)) {
                    quote!(&*#name)
                } else {
                    quote!(&#name)
                };
                quote!(Some(::yoake_inject_rt::InjectSnapshot::inject_snapshot(#reference)))
            } else {
                quote!(None)
            };
            encode.push(quote!(#scope_name.handle(#native_type, #snapshot)));
            decode.push(quote!(let _ = #iterator.next();));
        } else if matches!(ty.as_ref(), Type::Reference(_)) {
            let storage = format_ident!("__yoake_inject_text_{}", name);
            encode.push(quote!(::yoake_inject_rt::Value::Text(#name.to_owned())));
            decode.push(quote! {
                let #storage = <String as ::yoake_inject_rt::InjectValue>::from_inject_value(
                    #iterator.next().expect("validated injection argument count"),
                ).expect("validated injection argument type");
                let #pattern: #ty = #storage.as_str();
            });
        } else {
            encode.push(quote!(::yoake_inject_rt::InjectValue::into_inject_value(#name)));
            decode.push(quote! {
                let #pattern: #ty = <#ty as ::yoake_inject_rt::InjectValue>::from_inject_value(
                    #iterator.next().expect("validated injection argument count"),
                ).expect("validated injection argument type");
            });
        }
    }
    let result_type: Type = match &signature.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => *ty.clone(),
    };
    let result_variant = format_ident!("{}", info.result.variant);
    let result_kind = quote!(::yoake_inject_rt::ValueType::#result_variant);
    let hash = syn::LitStr::new(&info.hash(), Span::call_site());
    let ident = &signature.ident;
    let site = format_ident!("__yoake_inject_site_{}", ident);
    let registry_bound = match options.id.as_deref() {
        Some(
            "world::level::on_place"
            | "world::level::tick::crop_random_tick"
            | "world::level::bonemeal::crop_bonemeal_increase",
        ) => quote!(Some(self.b.table.state_count())),
        Some("game::drop_falling_block") => quote!(Some(self.res.facts().states.len())),
        _ => quote!(None),
    };
    let path = match options.id {
        Some(id) => {
            let id = syn::LitStr::new(&id, Span::call_site());
            quote!(#id)
        }
        None => quote!(concat!(module_path!(), "::", stringify!(#ident))),
    };
    let side = match options.side.as_deref() {
        Some("client") => quote!(::yoake_inject_rt::Side::Client),
        Some("server") => quote!(::yoake_inject_rt::Side::Server),
        Some("both") => quote!(::yoake_inject_rt::Side::Both),
        _ => quote!(::yoake_inject_rt::Side::Unspecified),
    };
    let allowed = options.allowed.unwrap_or(7);
    let visibility = &function.vis;
    let attributes = &function.attrs;
    let cfg_attributes = attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("cfg"));
    let body = &function.block;
    let arguments = format_ident!("__yoake_inject_arguments", span = Span::mixed_site());
    let native = format_ident!("__yoake_inject_native", span = Span::mixed_site());
    let result = format_ident!("__yoake_inject_result", span = Span::mixed_site());
    let scope = scoped.then(|| quote!(let mut #scope_name = ::yoake_inject_rt::Scope::default();));
    let accessor = if receiver {
        quote!(Self::#site())
    } else {
        quote!(#site())
    };
    let wire_type = success_type(&signature.output).unwrap_or_else(|| result_type.clone());
    let invoke = if info.fallible {
        quote!(invoke_fallible_validated)
    } else {
        quote!(invoke_validated)
    };
    let encode_result = if info.fallible {
        quote!(#native.map(::yoake_inject_rt::InjectValue::into_inject_value))
    } else {
        quote!(::yoake_inject_rt::InjectValue::into_inject_value(#native))
    };
    let decoded = quote!(<#wire_type as ::yoake_inject_rt::InjectValue>::from_inject_value(#result)
        .expect("validated injection return type"));
    let decode_result = if info.fallible {
        quote!(#result.map(|#result| #decoded))
    } else {
        decoded
    };
    let iterator_setup = if signature.inputs.is_empty() {
        quote!(let _ = #arguments;)
    } else {
        quote!(let mut #iterator = #arguments.into_iter();)
    };
    Ok(quote! {
            #(#cfg_attributes)*
            #[doc(hidden)]
            #[inline(always)]
            #visibility fn #site() -> &'static ::yoake_inject_rt::Site {
                const INDEX: Option<usize> = ::yoake_inject_rt::catalog_index(#path, #hash);
                static SITE: ::yoake_inject_rt::Site = ::yoake_inject_rt::Site::new(
                    #path, #hash, #side, &[#(#kinds),*], #result_kind, #allowed,
                );
                match INDEX {
                    Some(index) => &::yoake_inject_rt::catalog::SITES[index],
                    None => &SITE,
                }
            }

            #(#attributes)*
            #visibility #signature {
                if !#accessor.has_handlers() {
                    #body
                } else {
                    ::yoake_inject_rt::dispatch_slow(move || {
                        if #accessor.mask() == 0 {
                            return (|| -> #result_type #body)();
                        }
                        #scope
                        let #result = #accessor.#invoke(
                            vec![#(#encode),*],
                            #registry_bound,
                            |#arguments| {
                                #iterator_setup
                                #(#decode)*
                                let #native: #result_type = (|| -> #result_type #body)();
                                #encode_result
                            },
                        );
                        #decode_result
                    })
                }
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_scalar_function_with_a_mask_gate_and_stable_hash() {
        let function: ItemFn = syn::parse_quote!(
            fn calculate(value: i32) -> i32 {
                return value + 1;
            }
        );
        let expanded = expand(quote!(id = "test::calculate", side = "server"), function)
            .unwrap()
            .to_string();
        assert!(expanded.contains("mask"));
        assert!(expanded.contains("yoake_inject_site_calculate"));
        assert!(expanded.contains("test::calculate"));
        assert!(expanded.contains("Side :: Server"));
    }

    #[test]
    fn accepts_scoped_references_and_rejects_unsafe_functions() {
        let reference: ItemFn = syn::parse_quote!(
            fn test(value: &i32) {}
        );
        assert!(expand(quote!(), reference).is_ok());
        let unsafe_function: ItemFn = syn::parse_quote!(
            unsafe fn test() {}
        );
        assert!(
            expand(quote!(), unsafe_function)
                .unwrap_err()
                .to_string()
                .contains("unsafe")
        );
    }

    #[test]
    fn rejects_duplicate_or_unknown_injection_points() {
        assert!(parse_options(quote!(allow = "head,head")).is_err());
        assert!(parse_options(quote!(allow = "middle")).is_err());
        assert!(parse_options(quote!(allow = "head", allow = "return")).is_err());
    }
}
