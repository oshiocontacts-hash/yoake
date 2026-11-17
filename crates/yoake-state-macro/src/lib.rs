use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse::Parser, spanned::Spanned, Expr, Fields, ItemConst, ItemStruct, LitStr, Meta, PathArguments, Type};

#[derive(Default)]
struct Options {
    read_only: bool,
    skip: bool,
    write: bool,
    min: Option<Expr>,
    max: Option<Expr>,
}

fn parse_options(tokens: proc_macro2::TokenStream) -> syn::Result<Options> {
    let mut options = Options::default();
    syn::meta::parser(|meta| {
        if meta.path.is_ident("read_only") {
            if options.read_only {
                return Err(meta.error("duplicate read_only option"));
            }
            options.read_only = true;
        } else if meta.path.is_ident("skip") {
            if options.skip {
                return Err(meta.error("duplicate skip option"));
            }
            options.skip = true;
        } else if meta.path.is_ident("write") {
            if options.write {
                return Err(meta.error("duplicate write option"));
            }
            options.write = true;
        } else if meta.path.is_ident("min") {
            if options.min.is_some() {
                return Err(meta.error("duplicate min option"));
            }
            options.min = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("max") {
            if options.max.is_some() {
                return Err(meta.error("duplicate max option"));
            }
            options.max = Some(meta.value()?.parse()?);
        } else {
            return Err(meta.error("unknown yoake_state option"));
        }
        Ok(())
    })
    .parse2(tokens)?;
    Ok(options)
}

fn expand_state(args: proc_macro2::TokenStream, mut item: ItemStruct) -> syn::Result<proc_macro2::TokenStream> {
    let options = parse_options(args)?;
    if options.write || options.min.is_some() || options.max.is_some() || (options.read_only && options.skip) {
        return Err(syn::Error::new(item.span(), "invalid struct-level yoake_state options"));
    }
    if options.skip {
        for field in &mut item.fields {
            field.attrs.retain(|attribute| !attribute.path().is_ident("yoake_state"));
        }
        return Ok(quote! { #item });
    }
    if !item.generics.params.is_empty() {
        return Err(syn::Error::new(item.generics.span(), "generic state structs are not supported"));
    }
    if item.attrs.iter().any(|attribute| attribute.path().is_ident("cfg_attr")) {
        return Err(syn::Error::new(item.span(), "cfg_attr on state structs is not supported"));
    }
    let conditional_item_attrs: Vec<_> = item.attrs.iter()
        .filter(|attribute| attribute.path().is_ident("cfg"))
        .cloned()
        .collect();
    let item_span = item.span();
    let fields = match &mut item.fields {
        Fields::Named(fields) => &mut fields.named,
        _ => return Err(syn::Error::new(item_span, "yoake_state requires named fields")),
    };
    let mut descriptors = Vec::new();
    let mut snapshots = Vec::new();
    let mut validators = Vec::new();
    let mut setters = Vec::new();
    let mut getters = Vec::new();

    for field in fields {
        let mut state_options = None;
        let mut remaining = Vec::new();
        for attribute in std::mem::take(&mut field.attrs) {
            if attribute.path().is_ident("yoake_state") {
                if state_options.is_some() {
                    return Err(syn::Error::new(attribute.span(), "duplicate yoake_state attribute"));
                }
                state_options = Some(match &attribute.meta {
                    Meta::Path(_) => Options::default(),
                    Meta::List(list) => parse_options(list.tokens.clone())?,
                    Meta::NameValue(_) => return Err(syn::Error::new(attribute.span(), "invalid yoake_state attribute")),
                });
            } else {
                remaining.push(attribute);
            }
        }
        field.attrs = remaining;
        let Some(field_options) = state_options else { continue };
        if field_options.skip {
            if field_options.read_only || field_options.write || field_options.min.is_some() || field_options.max.is_some() {
                return Err(syn::Error::new(field.span(), "skip cannot be combined with other options"));
            }
            continue;
        }
        if field.attrs.iter().any(|attribute| attribute.path().is_ident("cfg_attr")) {
            return Err(syn::Error::new(field.span(), "cfg_attr on exposed state fields is not supported"));
        }
        let conditional_attrs: Vec<_> = field.attrs.iter()
            .filter(|attribute| attribute.path().is_ident("cfg"))
            .collect();
        if field_options.read_only && field_options.write {
            return Err(syn::Error::new(field.span(), "a field cannot be read-only and writable"));
        }
        if options.read_only && field_options.write {
            return Err(syn::Error::new(field.span(), "a field of a read-only struct cannot be writable"));
        }
        if !field_options.write && (field_options.min.is_some() || field_options.max.is_some()) {
            return Err(syn::Error::new(field.span(), "bounds require a writable field"));
        }
        let field_ident = field.ident.as_ref().ok_or_else(|| syn::Error::new(field.span(), "expected named field"))?;
        let identifier_name = field_ident.to_string();
        let field_name = identifier_name.strip_prefix("r#").unwrap_or(&identifier_name);
        let type_ident = match &field.ty {
            Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 && matches!(&path.path.segments[0].arguments, PathArguments::None) => &path.path.segments[0].ident,
            _ => return Err(syn::Error::new(field.ty.span(), "unsupported state field type")),
        };
        let type_name = type_ident.to_string();
        if !matches!(type_name.as_str(), "bool" | "i32" | "f32" | "f64") {
            return Err(syn::Error::new(field.ty.span(), "only bool, i32, f32 and f64 state fields are supported"));
        }
        let writable = field_options.write;
        let ty = &field.ty;
        let getter = format_ident!("yoake_state_{}", field_name);
        descriptors.push(quote! { #(#conditional_attrs)* ::yoake_state::Field { name: #field_name, value_type: #type_name, writable: #writable } });
        snapshots.push(quote! { #(#conditional_attrs)* fields.insert(#field_name.to_owned(), ::yoake_state::Scalar::into_state_value(self.#field_ident)?); });
        getters.push(quote! { #(#conditional_attrs)* pub fn #getter(&self) -> #ty { self.#field_ident } });
        if writable {
            let check_bounds = if type_name == "bool" {
                if field_options.min.is_some() || field_options.max.is_some() {
                    return Err(syn::Error::new(field.span(), "boolean fields cannot have bounds"));
                }
                quote! {}
            } else {
                let (Some(min), Some(max)) = (field_options.min, field_options.max) else {
                    return Err(syn::Error::new(field.span(), "writable numeric fields require min and max"));
                };
                if type_name == "i32" {
                    quote! {
                        let minimum: #ty = #min;
                        let maximum: #ty = #max;
                        if minimum > maximum || candidate < minimum || candidate > maximum {
                            return ::core::result::Result::Err(::std::format!("value for {} is out of range", #field_name));
                        }
                    }
                } else {
                    quote! {
                        let minimum: #ty = #min;
                        let maximum: #ty = #max;
                        let raw = match value {
                            ::yoake_state::Value::Float(raw) => *raw,
                            _ => return ::core::result::Result::Err("expected a floating-point state value".to_owned()),
                        };
                        if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum
                            || raw < f64::from(minimum) || raw > f64::from(maximum)
                        {
                            return ::core::result::Result::Err(::std::format!("value for {} is out of range", #field_name));
                        }
                    }
                }
            };
            validators.push(quote! {
                #(#conditional_attrs)*
                #field_name => {
                    let candidate = <#ty as ::yoake_state::Scalar>::from_state_value(value)?;
                    #check_bounds
                    let _ = candidate;
                    ::core::result::Result::Ok(())
                }
            });
            setters.push(quote! {
                #(#conditional_attrs)*
                #field_name => {
                    self.#field_ident = <#ty as ::yoake_state::Scalar>::from_state_value(value)?;
                    ::core::result::Result::Ok(())
                }
            });
        } else {
            validators.push(quote! { #(#conditional_attrs)* #field_name => ::core::result::Result::Err(::std::format!("field {} is read-only", #field_name)) });
        }
    }

    let fields_init = if snapshots.is_empty() {
        quote! { let fields = ::std::collections::BTreeMap::new(); }
    } else {
        quote! { #[allow(unused_mut)] let mut fields = ::std::collections::BTreeMap::new(); }
    };
    let name = &item.ident;
    Ok(quote! {
        #item

        #(#conditional_item_attrs)*
        impl ::yoake_state::State for #name {
            fn type_name() -> &'static str {
                concat!(module_path!(), "::", stringify!(#name))
            }

            fn fields() -> &'static [::yoake_state::Field] {
                &[#(#descriptors),*]
            }

            fn snapshot(&self) -> ::core::result::Result<::yoake_state::Value, ::std::string::String> {
                #fields_init
                #(#snapshots)*
                ::core::result::Result::Ok(::yoake_state::Value::Record(fields))
            }

            fn validate_set(&self, path: &[::std::string::String], value: &::yoake_state::Value) -> ::core::result::Result<(), ::std::string::String> {
                ::yoake_state::validate_path(path)?;
                if path.len() != 1 {
                    return ::core::result::Result::Err("state write path must name exactly one field".to_owned());
                }
                let _ = value;
                match path[0].as_str() {
                    #(#validators,)*
                    _ => ::core::result::Result::Err("unknown state field".to_owned()),
                }
            }

            fn set(&mut self, path: &[::std::string::String], value: &::yoake_state::Value) -> ::core::result::Result<(), ::std::string::String> {
                <Self as ::yoake_state::State>::validate_set(self, path, value)?;
                match path[0].as_str() {
                    #(#setters,)*
                    _ => ::core::result::Result::Err("state field is not writable".to_owned()),
                }
            }
        }

        #(#conditional_item_attrs)*
        impl #name {
            #(#getters)*
        }
    })
}

#[proc_macro_attribute]
pub fn yoake_state(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse::<ItemStruct>(input);
    match item.and_then(|item| expand_state(args.into(), item)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_const(args: proc_macro2::TokenStream, item: ItemConst) -> syn::Result<proc_macro2::TokenStream> {
    let mut id: Option<LitStr> = None;
    let mut minimum: Option<Expr> = None;
    let mut maximum: Option<Expr> = None;
    syn::meta::parser(|meta| {
        if meta.path.is_ident("id") {
            if id.is_some() {
                return Err(meta.error("duplicate id option"));
            }
            id = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("min") {
            if minimum.is_some() {
                return Err(meta.error("duplicate min option"));
            }
            minimum = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("max") {
            if maximum.is_some() {
                return Err(meta.error("duplicate max option"));
            }
            maximum = Some(meta.value()?.parse()?);
        } else {
            return Err(meta.error("unknown yoake_const option"));
        }
        Ok(())
    }).parse2(args)?;
    let id = id.ok_or_else(|| syn::Error::new(item.span(), "yoake_const requires an id"))?;
    let type_name = match item.ty.as_ref() {
        Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1
            && matches!(&path.path.segments[0].arguments, PathArguments::None) =>
        {
            path.path.segments[0].ident.to_string()
        }
        _ => return Err(syn::Error::new(item.ty.span(), "unsupported constant type")),
    };
    let kind = match type_name.as_str() {
        "bool" => quote! { ::yoake_state::ConstantKind::Boolean },
        "i32" => quote! { ::yoake_state::ConstantKind::I32 },
        "f32" => quote! { ::yoake_state::ConstantKind::F32 },
        "f64" => quote! { ::yoake_state::ConstantKind::F64 },
        _ => return Err(syn::Error::new(item.ty.span(), "only bool, i32, f32 and f64 constants are supported")),
    };
    let ty = &item.ty;
    let bounds = match (type_name.as_str(), minimum, maximum) {
        ("bool", None, None) => quote! { (::core::option::Option::None, ::core::option::Option::None) },
        ("bool", _, _) => return Err(syn::Error::new(item.span(), "boolean constants cannot have bounds")),
        (_, Some(minimum), Some(maximum)) => quote! {{
            let minimum: #ty = #minimum;
            let maximum: #ty = #maximum;
            (
                ::core::option::Option::Some(::yoake_state::Scalar::into_state_value(minimum)?),
                ::core::option::Option::Some(::yoake_state::Scalar::into_state_value(maximum)?),
            )
        }},
        _ => return Err(syn::Error::new(item.span(), "numeric constants require min and max")),
    };
    let name = &item.ident;
    let getter = format_ident!("yoake_const_{}", name.to_string().trim_start_matches("r#").to_lowercase());
    let visibility = &item.vis;
    let conditional_attrs: Vec<_> = item.attrs.iter().filter(|attribute| {
        attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
    }).collect();
    Ok(quote! {
        #item

        #(#conditional_attrs)*
        #visibility fn #getter() -> ::core::result::Result<::yoake_state::ConstantDefinition, ::std::string::String> {
            let (minimum, maximum) = #bounds;
            ::yoake_state::ConstantDefinition::new(
                #id,
                #kind,
                ::yoake_state::Scalar::into_state_value(#name)?,
                minimum,
                maximum,
            )
        }
    })
}

#[proc_macro_attribute]
pub fn yoake_const(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = syn::parse::<ItemConst>(input);
    match item.and_then(|item| expand_const(args.into(), item)) {
        Ok(output) => output.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
