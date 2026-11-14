pub fn generate_inject_wit() -> String {
    String::from(
        r#"package yoake:mods@1.1.0;

interface inject {
    record record-entry {
        key: string,
        node: u32,
    }

    variant value-node {
        unit,
        boolean(bool),
        signed(s64),
        unsigned(u64),
        float(f64),
        text(string),
        bytes(list<u8>),
        sequence(list<u32>),
        object(list<record-entry>),
    }

    record value-tree {
        root: u32,
        nodes: list<value-node>,
    }

    variant value {
        unit,
        boolean(bool),
        signed(s64),
        unsigned(u64),
        float(f64),
        text(string),
        bytes(list<u8>),
        sequence(value-tree),
        object(value-tree),
    }

    variant head-outcome {
        proceed,
        replace-arguments(list<value>),
        cancel(value),
    }

    inject-head: func(site: string, args: list<value>) -> head-outcome;
    inject-return: func(site: string, ret: value) -> option<value>;
    inject-overwrite: func(site: string, args: list<value>) -> value;
}

world inject-mod {
    export inject;
}
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::generate_inject_wit;
    use std::path::Path;

    #[test]
    fn generated_inject_wit_matches_committed_and_parses() {
        let generated = generate_inject_wit();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../yoake-wasm/wit-inject/yoake-inject.wit");
        let committed = std::fs::read_to_string(path).expect("read generated inject WIT");
        assert_eq!(generated, committed.replace("\r\n", "\n"));
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve.push_str("yoake-inject.wit", &generated).expect("parse WIT");
        resolve.select_world(&[package], Some("inject-mod")).expect("find world");
    }
}
