pub fn generate_inject_wit() -> String {
    String::from(
        r#"package yoake:mods@1.1.0;

interface inject {
    record registration {
        site: string,
        points: u8,
    }

    registrations: func() -> list<registration>;

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

pub fn generate_inject_config_wit() -> String {
    String::from(
        r#"package yoake:mods@1.2.0;

interface config {
    enum config-value-kind {
        %bool,
        int,
        float,
        %string,
        %enum,
        bool-list,
        int-list,
        float-list,
        string-list,
        enum-list,
    }

    record config-value {
        bool-list: list<bool>,
        bool-value: bool,
        float-list: list<f64>,
        float-value: f64,
        int-list: list<s64>,
        int-value: s64,
        kind: config-value-kind,
        string-list: list<string>,
        string-value: string,
    }

    config-get: func(key: string) -> result<config-value, string>;
}

interface config-events {
    use config.{config-value};
    config-changed: func(key: string, value: config-value);
}

world inject-config {
    import config;
    export config-events;
}

world inject-mod {
    include yoake:mods/inject-mod@1.1.0;
    include inject-config;
}
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::{generate_inject_config_wit, generate_inject_wit};
    use std::path::Path;

    #[test]
    fn generated_inject_wit_matches_committed_and_parses() {
        let generated = generate_inject_wit();
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-wasm/wit-inject/yoake-inject.wit");
        if std::env::var_os("YOAKE_UPDATE_WIT").is_some() {
            std::fs::write(&path, &generated).expect("write generated inject WIT");
        }
        let committed = std::fs::read_to_string(path).expect("read generated inject WIT");
        assert_eq!(generated, committed.replace("\r\n", "\n"));
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve
            .push_str("yoake-inject.wit", &generated)
            .expect("parse WIT");
        resolve
            .select_world(&[package], Some("inject-mod"))
            .expect("find world");
    }

    #[test]
    fn generated_inject_config_wit_matches_committed_and_parses() {
        let directory =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-wasm/wit-inject-config");
        let generated = generate_inject_config_wit();
        let dependency = generate_inject_wit();
        let path = directory.join("yoake-inject-config.wit");
        let dependency_path = directory.join("deps/yoake-inject.wit");
        if std::env::var_os("YOAKE_UPDATE_WIT").is_some() {
            std::fs::create_dir_all(directory.join("deps")).expect("create WIT directory");
            std::fs::write(&path, &generated).expect("write inject config WIT");
            std::fs::write(&dependency_path, &dependency).expect("write inject 1.1 dependency");
        }
        assert_eq!(
            generated,
            std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
        );
        assert_eq!(
            dependency,
            std::fs::read_to_string(dependency_path)
                .unwrap()
                .replace("\r\n", "\n")
        );
        let mut resolve = wit_parser::Resolve::default();
        let (package, _) = resolve
            .push_dir(directory)
            .expect("parse inject config WIT");
        resolve
            .select_world(&[package], Some("inject-mod"))
            .expect("find inject world");
        resolve
            .select_world(&[package], Some("inject-config"))
            .expect("find config world");
    }
}
