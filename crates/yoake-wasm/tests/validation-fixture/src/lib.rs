#[cfg(any(feature = "typo", feature = "trap"))]
mod frozen {
    use yoake_wasm_sdk::Mod;

    #[derive(Default)]
    struct Fixture;

    impl Mod for Fixture {
        fn subscriptions() -> &'static [&'static str] {
            #[cfg(feature = "typo")]
            { &["tock"] }
            #[cfg(not(feature = "typo"))]
            { &[] }
        }

        fn init(&mut self) {
            #[cfg(feature = "trap")]
            panic!("intentional init trap");
        }
    }

    yoake_wasm_sdk::export_mod!(Fixture);
}

#[cfg(any(feature = "unwired-client", feature = "wrong-client"))]
mod client {
    use yoake_wasm_sdk::full::client::ClientMod;

    #[derive(Default)]
    struct Fixture;

    impl ClientMod for Fixture {
        fn subscriptions() -> &'static [&'static str] {
            #[cfg(feature = "unwired-client")]
            { &["input_key"] }
            #[cfg(feature = "wrong-client")]
            { &["server_about_to_start"] }
        }
    }

    yoake_wasm_sdk::export_client_full!(Fixture);
}

#[cfg(feature = "wrong-server")]
mod server {
    use yoake_wasm_sdk::full::server::ServerMod;

    #[derive(Default)]
    struct Fixture;

    impl ServerMod for Fixture {
        fn subscriptions() -> &'static [&'static str] { &["hud"] }
    }

    yoake_wasm_sdk::export_server_full!(Fixture);
}
