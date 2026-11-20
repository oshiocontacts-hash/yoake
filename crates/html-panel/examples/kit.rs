//! キットの通しの試し：ローカルの元・許可リスト・やり取り（両向き）・上限・自動の読み直し・止める/戻す。
//!
//! cargo run -p html-panel --example kit

use std::time::{Duration, Instant};

use html_panel::{Options, Panel, Source, Status};
use serde_json::{Value, json};

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>kit</title>
<body style="margin:0;font:20px sans-serif"><div id="v">VERSION</div>
<script>
yoake.onMessage = (m) => { yoake.send({ echo: m }); };
yoake.send({ loaded: "VERSION" });
fetch("https://example.com/").then(() => yoake.send({ net: "reached" })).catch(() => yoake.send({ net: "blocked" }));
try { yoake.send("x".repeat(5000)); } catch (e) { yoake.send({ big: "refused" }); }
</script>"#;

fn wait(panel: &Panel, secs: u64, mut done: impl FnMut(&[Value]) -> bool, got: &mut Vec<Value>) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if let Status::Failed(e) = panel.status() {
            panic!("failed: {e}");
        }
        got.extend(panel.poll());
        if done(got) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn main() {
    let dir = std::env::temp_dir().join(format!("html-panel-kit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("hub.html");
    std::fs::write(&file, PAGE.replace("VERSION", "v1")).unwrap();

    let panel = Panel::open_source(Source::File(file.clone()), [400, 200], 1.0, Options { watch: true, max_message: 4096, ..Default::default() }).expect("browser");
    let mut got = Vec::new();
    let ok = wait(&panel, 20, |g| g.iter().any(|v| v.get("net").is_some()) && g.iter().any(|v| v.get("big").is_some()), &mut got);
    println!("1. local page: {ok} {got:?}");
    assert!(got.contains(&json!({ "loaded": "v1" })));
    assert!(got.contains(&json!({ "net": "blocked" })), "the net must be blocked for a local source");
    assert!(got.contains(&json!({ "big": "refused" })), "over-limit sends must throw");

    got.clear();
    panel.post(&json!({ "hello": 1 })).unwrap();
    let ok = wait(&panel, 5, |g| g.contains(&json!({ "echo": { "hello": 1 } })), &mut got);
    println!("2. post -> onMessage -> send: {ok}");
    assert!(ok);
    assert!(panel.post(&json!("y".repeat(5000))).is_err(), "over-limit posts must be refused");

    got.clear();
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&file, PAGE.replace("VERSION", "v2")).unwrap();
    let ok = wait(&panel, 6, |g| g.contains(&json!({ "loaded": "v2" })), &mut got);
    println!("3. watch reload: {ok}");
    assert!(ok);

    panel.set_visible(false);
    std::thread::sleep(Duration::from_millis(500));
    let before = panel.frames();
    std::thread::sleep(Duration::from_millis(800));
    panel.set_visible(true);
    got.clear();
    panel.post(&json!({ "after": "resume" })).unwrap();
    let ok = wait(&panel, 5, |g| g.contains(&json!({ "echo": { "after": "resume" } })), &mut got);
    println!("4. hide/show: frames while hidden {} → resumed and answering {ok}", panel.frames() - before);
    assert!(ok);

    let html = Panel::open_source(Source::Html("<!doctype html><script>yoake.send({inline:true})</script>".into()), [400, 200], 1.0, Options::default()).unwrap();
    let mut g2 = Vec::new();
    let ok = wait(&html, 15, |g| g.contains(&json!({ "inline": true })), &mut g2);
    println!("5. Source::Html: {ok}");
    assert!(ok);
    drop(html);
    drop(panel);
    let _ = std::fs::remove_dir_all(&dir);
    println!("all good");
}
