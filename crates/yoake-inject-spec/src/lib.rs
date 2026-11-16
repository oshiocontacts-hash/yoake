use proc_macro2::TokenStream;
use quote::ToTokens;
use sha2::{Digest, Sha256};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, FnArg, Lit, Meta, Pat, ReturnType, Signature, Token, Type};

pub use yoake_value::policy::{Policy, Rule, site_policy};

#[derive(Default)]
pub struct Options {
    pub id: Option<String>,
    pub allowed: Option<u8>,
    pub side: Option<String>,
    pub entities: Vec<String>,
}

pub fn parse_options(attribute: TokenStream) -> syn::Result<Options> {
    let values = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attribute)?;
    let mut options = Options::default();
    for value in values {
        let Meta::NameValue(entry) = value else {
            return Err(syn::Error::new_spanned(
                value,
                "expected option name and string value",
            ));
        };
        let Expr::Lit(expression) = &entry.value else {
            return Err(syn::Error::new_spanned(
                entry.value,
                "expected a string literal",
            ));
        };
        let Lit::Str(text) = &expression.lit else {
            return Err(syn::Error::new_spanned(
                &expression.lit,
                "expected a string literal",
            ));
        };
        if entry.path.is_ident("id") && options.id.is_none() {
            let id = text.value();
            if id.is_empty() || id.contains('#') {
                return Err(syn::Error::new_spanned(
                    text,
                    "id must be nonempty and cannot contain '#'",
                ));
            }
            options.id = Some(id);
        } else if entry.path.is_ident("side") && options.side.is_none() {
            let side = text.value();
            if side != "client" && side != "server" && side != "both" {
                return Err(syn::Error::new_spanned(
                    text,
                    "side must be client, server, or both",
                ));
            }
            options.side = Some(side);
        } else if entry.path.is_ident("entities") && options.entities.is_empty() {
            for name in text.value().split(',').map(str::trim) {
                if syn::parse_str::<syn::Ident>(name).is_err()
                    || options.entities.iter().any(|previous| previous == name)
                {
                    return Err(syn::Error::new_spanned(
                        text,
                        "entities must name distinct u32 arguments",
                    ));
                }
                options.entities.push(name.to_owned());
            }
        } else if entry.path.is_ident("allow") && options.allowed.is_none() {
            let mut allowed = 0;
            for point in text.value().split(',').map(str::trim) {
                let bit = match point {
                    "head" => 1,
                    "return" => 2,
                    "overwrite" => 4,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            text,
                            "allow accepts head, return, and overwrite",
                        ));
                    }
                };
                if allowed & bit != 0 {
                    return Err(syn::Error::new_spanned(text, "duplicate injection point"));
                }
                allowed |= bit;
            }
            options.allowed = Some(allowed);
        } else {
            return Err(syn::Error::new_spanned(
                entry.path,
                "unknown or duplicate yoake_inject option",
            ));
        }
    }
    Ok(options)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shape {
    pub wire: String,
    pub variant: &'static str,
}

pub fn shape(ty: &Type) -> syn::Result<Shape> {
    if let Type::Tuple(tuple) = ty {
        if tuple.elems.is_empty() {
            return Ok(Shape {
                wire: "unit".into(),
                variant: "Unit",
            });
        }
        if tuple.elems.len() == 3 {
            let first = &tuple.elems[0];
            if tuple.elems.iter().all(|element| {
                element.to_token_stream().to_string() == first.to_token_stream().to_string()
            }) {
                if let Some(shape) = vector_shape(first) {
                    return Ok(shape);
                }
            }
        }
    }
    if let Type::Array(array) = ty {
        if array.len.to_token_stream().to_string() == "3" {
            if let Some(shape) = vector_shape(&array.elem) {
                return Ok(shape);
            }
        }
    }
    if let Type::Reference(reference) = ty {
        if reference.mutability.is_none()
            && matches!(reference.elem.as_ref(), Type::Path(path) if path.path.is_ident("str"))
        {
            return Ok(Shape {
                wire: "string".into(),
                variant: "String",
            });
        }
        return Ok(handle_shape(ty));
    }
    if let Type::Ptr(_) = ty {
        return Err(syn::Error::new_spanned(
            ty,
            "raw pointers cannot be mapped to injection values",
        ));
    }
    if let Type::Path(path) = ty {
        if path.qself.is_none() && path.path.segments.len() == 1 {
            let segment = &path.path.segments[0];
            let kind = match segment.ident.to_string().as_str() {
                "bool" => Some(("bool", "Bool")),
                "i8" => Some(("s8", "S8")),
                "i16" => Some(("s16", "S16")),
                "i32" => Some(("s32", "S32")),
                "i64" => Some(("s64", "S64")),
                "u8" => Some(("u8", "U8")),
                "u16" => Some(("u16", "U16")),
                "u32" => Some(("u32", "U32")),
                "u64" => Some(("u64", "U64")),
                "usize" => Some(("usize", "Usize")),
                "isize" => Some(("isize", "Ssize")),
                "f32" => Some(("f32", "F32")),
                "f64" => Some(("f64", "F64")),
                "String" => Some(("string", "String")),
                "BlockPos" => Some(("block-pos", "BlockPos")),
                "BlockStateId" => Some(("block-state-id", "U32")),
                "BlockId" => Some(("block-id", "U16")),
                "Vec3" => Some(("vec3", "Vec3")),
                "Pos" => Some(("block-pos", "BlockPos")),
                _ => None,
            };
            if let Some((wire, variant)) = kind {
                if matches!(segment.arguments, syn::PathArguments::None) {
                    return Ok(Shape {
                        wire: wire.into(),
                        variant,
                    });
                }
            }
            if segment.ident == "Vec" {
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    if arguments.args.len() == 1 {
                        if let Some(syn::GenericArgument::Type(Type::Path(element))) =
                            arguments.args.first()
                        {
                            if element.qself.is_none() && element.path.is_ident("u8") {
                                return Ok(Shape {
                                    wire: "list<u8>".into(),
                                    variant: "Bytes",
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(handle_shape(ty))
}

fn vector_shape(ty: &Type) -> Option<Shape> {
    let (wire, variant) = match ty.to_token_stream().to_string().as_str() {
        "i32" => ("block-pos", "BlockPos"),
        "f32" => ("vec3-f32", "Vec3F32"),
        "f64" => ("vec3", "Vec3"),
        _ => return None,
    };
    Some(Shape {
        wire: wire.into(),
        variant,
    })
}

pub fn handle_shape(ty: &Type) -> Shape {
    let native = ty.to_token_stream().to_string().replace(' ', "");
    Shape {
        wire: format!("scoped<{native}>"),
        variant: "Handle",
    }
}

pub fn success_type(output: &ReturnType) -> Option<Type> {
    let ReturnType::Type(_, ty) = output else {
        return None;
    };
    let Type::Path(path) = ty.as_ref() else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" && segment.ident != "R" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(success) = arguments.args.first()? else {
        return None;
    };
    Some(success.clone())
}

pub struct SignatureInfo {
    pub arguments: Vec<Shape>,
    pub result: Shape,
    pub fallible: bool,
}

impl SignatureInfo {
    pub fn hash(&self) -> String {
        let names = self
            .arguments
            .iter()
            .map(|shape| shape.wire.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let result = if self.fallible {
            format!("result<{}>", self.result.wire)
        } else {
            self.result.wire.clone()
        };
        let canonical = format!("yoake-inject-v1:({names})->{result}");
        Sha256::digest(canonical.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

pub fn inspect_signature(signature: &Signature) -> syn::Result<SignatureInfo> {
    inspect_signature_with_options(signature, &Options::default())
}

pub fn inspect_signature_with_options(
    signature: &Signature,
    options: &Options,
) -> syn::Result<SignatureInfo> {
    if let Some(token) = signature.asyncness {
        return Err(syn::Error::new_spanned(
            token,
            "async functions cannot be injected",
        ));
    }
    if let Some(token) = signature.constness {
        return Err(syn::Error::new_spanned(
            token,
            "const functions cannot be injected",
        ));
    }
    if let Some(token) = signature.unsafety {
        return Err(syn::Error::new_spanned(
            token,
            "unsafe functions cannot be injected",
        ));
    }
    if let Some(abi) = &signature.abi {
        return Err(syn::Error::new_spanned(
            abi,
            "extern functions cannot be injected",
        ));
    }
    if signature.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            signature,
            "variadic functions cannot be injected",
        ));
    }
    let mut arguments = Vec::new();
    for argument in &signature.inputs {
        let FnArg::Typed(argument) = argument else {
            let FnArg::Receiver(receiver) = argument else {
                unreachable!()
            };
            arguments.push(handle_shape(&receiver.ty));
            continue;
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(syn::Error::new_spanned(
                &argument.pat,
                "injected arguments must be named identifiers",
            ));
        };
        if pattern.by_ref.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                pattern,
                "unsupported injection argument pattern",
            ));
        }
        let mut argument_shape = shape(&argument.ty)?;
        if options
            .entities
            .iter()
            .any(|name| pattern.ident == name.as_str())
        {
            if argument_shape.wire != "u32" {
                return Err(syn::Error::new_spanned(
                    &argument.ty,
                    "entity arguments must be u32 scoped identifiers",
                ));
            }
            argument_shape = Shape {
                wire: "scoped<entity>".into(),
                variant: "Handle",
            };
        }
        arguments.push(argument_shape);
    }
    for name in &options.entities {
        if !signature.inputs.iter().any(|argument| matches!(argument, FnArg::Typed(argument) if matches!(argument.pat.as_ref(), Pat::Ident(pattern) if pattern.ident == name.as_str()))) {
            return Err(syn::Error::new_spanned(signature, format!("unknown entity argument: {name}")));
        }
    }
    let success = success_type(&signature.output);
    let result = if let Some(success) = &success {
        shape(success)?
    } else {
        match &signature.output {
            ReturnType::Default => Shape {
                wire: "unit".into(),
                variant: "Unit",
            },
            ReturnType::Type(_, ty) => shape(ty)?,
        }
    };
    if result.variant == "Handle" {
        return Err(syn::Error::new_spanned(
            &signature.output,
            "injection returns must be owned wire values",
        ));
    }
    Ok(SignatureInfo {
        arguments,
        result,
        fallible: success.is_some(),
    })
}
