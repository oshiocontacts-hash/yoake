use proc_macro2::TokenStream;
use sha2::{Digest, Sha256};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Expr, FnArg, Lit, Meta, Pat, ReturnType, Signature, Token, Type};

#[derive(Default)]
pub struct Options {
    pub id: Option<String>,
    pub allowed: Option<u8>,
    pub side: Option<String>,
}

pub fn parse_options(attribute: TokenStream) -> syn::Result<Options> {
    let values = Punctuated::<Meta, Token![,]>::parse_terminated.parse2(attribute)?;
    let mut options = Options::default();
    for value in values {
        let Meta::NameValue(entry) = value else {
            return Err(syn::Error::new_spanned(value, "expected option name and string value"));
        };
        let Expr::Lit(expression) = &entry.value else {
            return Err(syn::Error::new_spanned(entry.value, "expected a string literal"));
        };
        let Lit::Str(text) = &expression.lit else {
            return Err(syn::Error::new_spanned(&expression.lit, "expected a string literal"));
        };
        if entry.path.is_ident("id") && options.id.is_none() {
            let id = text.value();
            if id.is_empty() || id.contains('#') {
                return Err(syn::Error::new_spanned(text, "id must be nonempty and cannot contain '#'"));
            }
            options.id = Some(id);
        } else if entry.path.is_ident("side") && options.side.is_none() {
            let side = text.value();
            if side != "client" && side != "server" {
                return Err(syn::Error::new_spanned(text, "side must be client or server"));
            }
            options.side = Some(side);
        } else if entry.path.is_ident("allow") && options.allowed.is_none() {
            let mut allowed = 0;
            for point in text.value().split(',').map(str::trim) {
                let bit = match point {
                    "head" => 1,
                    "return" => 2,
                    "overwrite" => 4,
                    _ => return Err(syn::Error::new_spanned(text, "allow accepts head, return, and overwrite")),
                };
                if allowed & bit != 0 {
                    return Err(syn::Error::new_spanned(text, "duplicate injection point"));
                }
                allowed |= bit;
            }
            options.allowed = Some(allowed);
        } else {
            return Err(syn::Error::new_spanned(entry.path, "unknown or duplicate yoake_inject option"));
        }
    }
    Ok(options)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shape {
    pub wire: &'static str,
    pub variant: &'static str,
}

pub fn shape(ty: &Type) -> syn::Result<Shape> {
    if let Type::Tuple(tuple) = ty {
        if tuple.elems.is_empty() {
            return Ok(Shape { wire: "unit", variant: "Unit" });
        }
    }
    if let Type::Reference(_) = ty {
        return Err(syn::Error::new_spanned(ty, "references require scoped engine handles, which are not implemented yet"));
    }
    if let Type::Ptr(_) = ty {
        return Err(syn::Error::new_spanned(ty, "raw pointers cannot be mapped to injection values"));
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
                "f32" => Some(("f32", "F32")),
                "f64" => Some(("f64", "F64")),
                "String" => Some(("string", "String")),
                _ => None,
            };
            if let Some((wire, variant)) = kind {
                if matches!(segment.arguments, syn::PathArguments::None) {
                    return Ok(Shape { wire, variant });
                }
            }
            if segment.ident == "Vec" {
                if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                    if arguments.args.len() == 1 {
                        if let Some(syn::GenericArgument::Type(Type::Path(element))) = arguments.args.first() {
                            if element.qself.is_none() && element.path.is_ident("u8") {
                                return Ok(Shape { wire: "list<u8>", variant: "Bytes" });
                            }
                        }
                    }
                }
            }
        }
    }
    Err(syn::Error::new_spanned(ty, "type has no supported injection value mapping; expected unit, a fixed-width scalar, String, or Vec<u8>"))
}

pub struct SignatureInfo {
    pub arguments: Vec<Shape>,
    pub result: Shape,
}

impl SignatureInfo {
    pub fn hash(&self) -> String {
        let names = self.arguments.iter().map(|shape| shape.wire).collect::<Vec<_>>().join(",");
        let canonical = format!("yoake-inject-v1:({names})->{}", self.result.wire);
        Sha256::digest(canonical.as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

pub fn inspect_signature(signature: &Signature) -> syn::Result<SignatureInfo> {
    if let Some(token) = signature.asyncness {
        return Err(syn::Error::new_spanned(token, "async functions cannot be injected"));
    }
    if let Some(token) = signature.constness {
        return Err(syn::Error::new_spanned(token, "const functions cannot be injected"));
    }
    if let Some(token) = signature.unsafety {
        return Err(syn::Error::new_spanned(token, "unsafe functions cannot be injected"));
    }
    if let Some(abi) = &signature.abi {
        return Err(syn::Error::new_spanned(abi, "extern functions cannot be injected"));
    }
    if signature.variadic.is_some() || !signature.generics.params.is_empty() || signature.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(signature, "generic or variadic functions cannot be injected"));
    }
    let mut arguments = Vec::new();
    for argument in &signature.inputs {
        let FnArg::Typed(argument) = argument else {
            return Err(syn::Error::new_spanned(argument, "methods require scoped engine handles, which are not implemented yet"));
        };
        let Pat::Ident(pattern) = argument.pat.as_ref() else {
            return Err(syn::Error::new_spanned(&argument.pat, "injected arguments must be named identifiers"));
        };
        if pattern.by_ref.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new_spanned(pattern, "unsupported injection argument pattern"));
        }
        arguments.push(shape(&argument.ty)?);
    }
    let result = match &signature.output {
        ReturnType::Default => Shape { wire: "unit", variant: "Unit" },
        ReturnType::Type(_, ty) => shape(ty)?,
    };
    Ok(SignatureInfo { arguments, result })
}
