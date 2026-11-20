//! 通しの試し：ページを開き、状態を送り、クリックし、ページからの便りと絵を受け取る。
//!
//! cargo run -p html-panel --example smoke -- <出力 PNG>

use std::time::{Duration, Instant};

use html_panel::{Input, Options, Panel, Status};
use serde_json::json;

const PAGE: &str = r#"<!doctype html><meta charset="utf-8">
<style>body{margin:0;font:28px sans-serif;color:#fff}#box{position:absolute;inset:20px;border-radius:16px;background:rgba(20,30,60,.85);padding:20px}
button{font:inherit;padding:8px 20px}</style>
<div id="box"><div id="msg">waiting</div><button id="b" style="position:absolute;left:20px;top:120px">Click</button></div>
<script>
let clicks = 0;
yoake.onState = (s) => { document.getElementById('msg').textContent = 'friends: ' + s.friends; yoake.post({ got: s.friends }); };
document.getElementById('b').onclick = () => { clicks++; yoake.post({ click: clicks }); };
window.yoakePost('raw text');
</script>"#;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "smoke.png".to_owned());
    let dir = std::env::temp_dir().join(format!("html-panel-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("page.html");
    std::fs::write(&page, PAGE).unwrap();
    let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));

    let t = Instant::now();
    let panel = Panel::open_with(&url, 480, 240, 2.0, Options { transparent: true, ..Default::default() }).expect("browser");
    println!("open returned in {:?}", t.elapsed());
    // つながる前の send は、つながったあとに届く
    panel.send(&json!({ "friends": 3 }));

    let mut posts = Vec::new();
    let mut last = None;
    let mut clicked = false;
    while t.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(50));
        if let Status::Failed(e) = panel.status() {
            panic!("failed: {e}");
        }
        posts.extend(panel.poll());
        if let Some(f) = panel.frame() {
            last = Some(f);
        }
        if !clicked && posts.iter().any(|p| p.get("got").is_some()) {
            clicked = true;
            // ボタンは CSS (40..., 140...) あたり
            panel.input(&Input::Down { x: 70.0, y: 160.0, button: "left".into(), clicks: 1, mods: 0 });
            panel.input(&Input::Up { x: 70.0, y: 160.0, button: "left".into(), clicks: 1, mods: 0 });
            panel.send(&json!({ "friends": 7 }));
        }
        if posts.iter().any(|p| p.get("click").is_some()) && posts.iter().any(|p| p.get("got") == Some(&json!(7))) && last.is_some() {
            std::thread::sleep(Duration::from_millis(300));
            if let Some(f) = panel.frame() {
                last = Some(f);
            }
            break;
        }
    }
    println!("ready after {:?}, posts: {posts:?}", t.elapsed());
    let f = last.expect("no frame");
    println!("frame {}x{} seq {} (frames {})", f.width, f.height, f.seq, panel.frames());
    let corner = &f.rgba[..4];
    println!("top-left pixel rgba {corner:?} (transparent background → alpha 0)");
    let file = std::fs::File::create(&out).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), f.width, f.height);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&f.rgba).unwrap();

    let pid_alive_check = Instant::now();
    drop(panel);
    println!("closed in {:?}", pid_alive_check.elapsed());
    let _ = std::fs::remove_dir_all(&dir);
}
