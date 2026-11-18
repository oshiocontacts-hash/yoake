// Explicit paths so this file resolves the same from lib.rs and from build.rs's #[path] include.
#[path = "render/guest.rs"]
mod guest;
#[path = "render/host.rs"]
mod host;

pub use guest::guest_stubs;
pub use host::host_glue;

use crate::model::Function;

pub fn classification_md(functions: &[Function]) -> String {
    let mut output = String::from(
        "# Binding classification\n\n| fn | visibility | receiver | category | reason |\n|---|---|---|---|---|\n",
    );
    for function in functions {
        let reason = if function.reasons.is_empty() {
            "-".to_string()
        } else {
            function.reasons.join("; ")
        };
        output.push_str(&format!(
            "| `{}` | {} | {} | ({}) | {} |\n",
            escape(&function.full_path),
            escape(&function.visibility),
            escape(&function.receiver),
            function.category.label(),
            escape(&reason),
        ));
    }
    output
}

fn escape(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}
