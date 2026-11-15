use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn yoake_test(attributes: TokenStream, item: TokenStream) -> TokenStream {
    if !attributes.is_empty() {
        return "compile_error!(\"yoake_test does not accept arguments\");"
            .parse()
            .expect("valid compile error");
    }
    item
}
