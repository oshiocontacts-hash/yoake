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

/// The additive 1.3 interfaces of a mod's worker (FR-F2): `crates/yoake-wasm/wit-worker/yoake-worker.wit`.
pub fn generate_worker_wit() -> String {
    String::from(
        r#"package yoake:mods@1.6.0;

/// The mod's side: talk to the worker declared by `[wasm] worker`. Neither call blocks.
interface worker {
    /// Queues a message for the worker. The error is "queue-full" (1024 messages or 16 MiB
    /// waiting), "too-large", "closed" (the worker stopped) or "no-worker" (the mod declares none).
    worker-send: func(message: list<u8>) -> result<_, string>;
    /// The next message the worker sent, or none.
    worker-recv: func() -> option<list<u8>>;
}

/// The worker's side: what the worker component imports.
interface worker-host {
    /// Queues a message for the mod. Never blocks; errors as `worker-send`.
    host-send: func(message: list<u8>) -> result<_, string>;
    /// Waits for the next message from the mod. none: the worker is being stopped.
    host-recv: func() -> option<list<u8>>;
    /// The next message from the mod, or none at once.
    host-try-recv: func() -> option<list<u8>>;
}

/// The worker's entry point: called once on the worker's own thread. When it returns, the worker
/// ends.
interface worker-run {
    run: func();
}

/// A mod that talks to a worker imports `worker` next to its usual world.
world worker-user {
    import worker;
}

/// What a worker component implements. It may also import any WASI 0.2 interface (files under
/// /data and /game, sockets, name lookup, http, blocking clocks).
world worker-mod {
    import worker-host;
    export worker-run;
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

#[cfg(test)]
mod worker_tests {
    use super::generate_worker_wit;
    use std::path::Path;

    #[test]
    fn generated_worker_wit_matches_committed_and_parses() {
        let generated = generate_worker_wit() ;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-wasm/wit-worker/yoake-worker.wit");
        if std::env::var_os("YOAKE_UPDATE_WIT").is_some() {
            std::fs::write(&path, &generated).expect("write worker WIT");
        }
        let committed = std::fs::read_to_string(path).expect("read worker WIT");
        assert_eq!(generated.trim_end(), committed.replace("\r\n", "\n").trim_end());
        let mut resolve = wit_parser::Resolve::default();
        let package = resolve.push_str("yoake-worker.wit", &generated).expect("parse WIT");
        resolve.select_world(&[package], Some("worker-mod")).expect("worker-mod");
        resolve.select_world(&[package], Some("worker-user")).expect("worker-user");
    }
}
