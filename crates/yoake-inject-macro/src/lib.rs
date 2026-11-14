use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, ReturnType, Type};
use yoake_inject_spec::{inspect_signature, parse_options};

#[proc_macro_attribute]
pub fn yoake_inject(attribute: TokenStream, item: TokenStream) -> TokenStream {
    let result = syn::parse::<ItemFn>(item)
        .and_then(|function| expand(attribute.into(), function));
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

fn expand(attribute: TokenStream2, function: ItemFn) -> syn::Result<TokenStream2> {
    let options = parse_options(attribute)?;
    let signature = &function.sig;
    let info = inspect_signature(signature)?;

    let mut names = Vec::new();
    let mut patterns = Vec::new();
    let mut types = Vec::new();
    let mut kinds = Vec::new();
    for (argument, shape) in signature.inputs.iter().zip(&info.arguments) {
        let FnArg::Typed(argument) = argument else {
            unreachable!("signature validation rejects receivers");
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            unreachable!("signature validation rejects unnamed patterns");
        };
        let kind = format_ident!("{}", shape.variant);
        names.push(&pattern.ident);
        patterns.push(pattern);
        types.push(&argument.ty);
        kinds.push(quote!(::yoake_inject_rt::ValueType::#kind));
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
        _ => quote!(::yoake_inject_rt::Side::Unspecified),
    };
    let allowed = options.allowed.unwrap_or(7);
    let visibility = &function.vis;
    let attributes = &function.attrs;
    let cfg_attributes = attributes.iter().filter(|attribute| attribute.path().is_ident("cfg"));
    let body = &function.block;
    let arguments = format_ident!("__yoake_inject_arguments", span = Span::mixed_site());
    let iter = format_ident!("__yoake_inject_iter", span = Span::mixed_site());
    let native = format_ident!("__yoake_inject_native", span = Span::mixed_site());
    let result = format_ident!("__yoake_inject_result", span = Span::mixed_site());
    let decode_arguments = if names.is_empty() {
        quote!(let _ = #arguments;)
    } else {
        quote! {
            let mut #iter = #arguments.into_iter();
            #(let #patterns: #types = <#types as ::yoake_inject_rt::InjectValue>::from_inject_value(
                #iter.next().expect("validated injection argument count"),
            ).expect("validated injection argument type");)*
        }
    };
    Ok(quote! {
        #(#cfg_attributes)*
        #[doc(hidden)]
        #[inline(always)]
        #visibility fn #site() -> &'static ::yoake_inject_rt::Site {
            static SITE: ::yoake_inject_rt::Site = ::yoake_inject_rt::Site::new(
                #path, #hash, #side, &[#(#kinds),*], #result_kind, #allowed,
            );
            &SITE
        }

        #(#attributes)*
        #visibility #signature {
            if #site().mask() == 0 {
                #body
            } else {
                let #result = #site().invoke(
                    vec![#(::yoake_inject_rt::InjectValue::into_inject_value(#names)),*],
                    |#arguments| {
                        #decode_arguments
                        let #native: #result_type = (|| -> #result_type #body)();
                        ::yoake_inject_rt::InjectValue::into_inject_value(#native)
                    },
                );
                <#result_type as ::yoake_inject_rt::InjectValue>::from_inject_value(#result)
                    .expect("validated injection return type")
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_scalar_function_with_a_mask_gate_and_stable_hash() {
        let function: ItemFn = syn::parse_quote!(fn calculate(value: i32) -> i32 { return value + 1; });
        let expanded = expand(quote!(id = "test::calculate", side = "server"), function).unwrap().to_string();
        assert!(expanded.contains("mask"));
        assert!(expanded.contains("yoake_inject_site_calculate"));
        assert!(expanded.contains("test::calculate"));
        assert!(expanded.contains("Side :: Server"));
    }

    #[test]
    fn rejects_unmapped_references_and_unsafe_functions() {
        let reference: ItemFn = syn::parse_quote!(fn test(value: &i32) {});
        assert!(expand(quote!(), reference).unwrap_err().to_string().contains("scoped engine handles"));
        let unsafe_function: ItemFn = syn::parse_quote!(unsafe fn test() {});
        assert!(expand(quote!(), unsafe_function).unwrap_err().to_string().contains("unsafe"));
    }

    #[test]
    fn rejects_duplicate_or_unknown_injection_points() {
        assert!(parse_options(quote!(allow = "head,head")).is_err());
        assert!(parse_options(quote!(allow = "middle")).is_err());
        assert!(parse_options(quote!(allow = "head", allow = "return")).is_err());
    }
}
