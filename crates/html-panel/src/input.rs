//! ページへの操作（クリック・ホイール・キー・文字・ページ移動）と、それを DevTools の命令に直すところ。
//!
//! 座標はページの CSS ピクセル（`Panel::open` の幅・高さの中）。修飾キーは DevTools の値
//! （Alt 1, Ctrl 2, Meta 4, Shift 8。GLFW の値からは `keys::cdp_modifiers` で直す）。

use serde_json::{Value, json};

use crate::keys;

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// マウスを動かす。`buttons` は押している最中のボタン（左 1・右 2・中 4）。
    Move { x: f64, y: f64, buttons: u32 },
    /// `button` は "left" / "right" / "middle"。`clicks` はダブルクリックなら 2。
    Down { x: f64, y: f64, button: String, clicks: u32, mods: u32 },
    Up { x: f64, y: f64, button: String, clicks: u32, mods: u32 },
    /// ホイール（ピクセル。下へは dy が正）。
    Wheel { x: f64, y: f64, dx: f64, dy: f64, mods: u32 },
    /// 打った文字（ASCII は本物のキーとして、それ以外は確定した文字として）。
    Type(String),
    /// 貼り付けなど（そのまま入れる）。
    Insert(String),
    /// 文字にならないキー（vanilla のキー名 `key.keyboard.enter` など、または esc・space）。
    Key { name: String, down: bool, mods: u32 },
    Nav(String),
    Back,
    Forward,
    Reload,
}

/// キー名から DevTools のキーへ（Enter・矢印・Ctrl+英字・Esc・Space）。
pub fn key_by_name(name: &str) -> Option<keys::Key> {
    match name {
        "escape" | "key.keyboard.escape" => Some(keys::Key { key: "Escape", code: "Escape", vk: 0x1B, text: None, shifted: false }),
        "space" | "key.keyboard.space" => Some(keys::Key { key: " ", code: "Space", vk: 0x20, text: Some(" "), shifted: false }),
        _ => keys::special(name).or_else(|| keys::letter_or_digit(name)),
    }
}

fn mouse(kind: &str, x: f64, y: f64, button: &str, buttons: u32, clicks: u32, modifiers: u32) -> (&'static str, Value) {
    (
        "Input.dispatchMouseEvent",
        json!({ "type": kind, "x": x, "y": y, "button": button, "buttons": buttons, "clickCount": clicks, "modifiers": modifiers }),
    )
}

/// 文字を打つ命令（ASCII は本物のキーとして、それ以外は IME の確定文字として）。
pub fn type_text(text: &str) -> Vec<(&'static str, Value)> {
    let mut out = Vec::new();
    for c in text.chars() {
        if let Some(key) = keys::for_char(c) {
            let mut down = json!({ "type": "keyDown", "key": c.to_string(), "code": key.code, "windowsVirtualKeyCode": key.vk, "text": c.to_string(), "unmodifiedText": c.to_string() });
            if c.is_ascii_uppercase() || key.shifted {
                down["modifiers"] = json!(8);
            }
            out.push(("Input.dispatchKeyEvent", down));
            out.push(("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": c.to_string(), "code": key.code, "windowsVirtualKeyCode": key.vk })));
        } else {
            out.push(("Input.insertText", json!({ "text": c.to_string() })));
        }
    }
    out
}

/// 文字にならないキー（Enter・矢印・Backspace・Ctrl+A など）の命令。
pub fn key(key: &keys::Key, down: bool, modifiers: u32) -> (&'static str, Value) {
    let mut event = json!({
        "type": if down { if key.text.is_some() { "keyDown" } else { "rawKeyDown" } } else { "keyUp" },
        "key": key.key,
        "code": key.code,
        "windowsVirtualKeyCode": key.vk,
        "nativeVirtualKeyCode": key.vk,
        "modifiers": modifiers,
    });
    if down {
        if let Some(t) = key.text {
            event["text"] = json!(t);
            event["unmodifiedText"] = json!(t);
        }
    }
    ("Input.dispatchKeyEvent", event)
}

impl Input {
    /// DevTools の命令（method, params）に直す。
    pub fn to_cdp(&self) -> Vec<(&'static str, Value)> {
        match self {
            Input::Move { x, y, buttons } => vec![mouse("mouseMoved", *x, *y, if *buttons & 1 != 0 { "left" } else { "none" }, *buttons, 0, 0)],
            Input::Down { x, y, button, clicks, mods } => {
                let mask = match button.as_str() {
                    "left" => 1,
                    "right" => 2,
                    "middle" => 4,
                    _ => 0,
                };
                vec![mouse("mouseMoved", *x, *y, "none", 0, 0, *mods), mouse("mousePressed", *x, *y, button, mask, *clicks, *mods)]
            }
            Input::Up { x, y, button, clicks, mods } => vec![mouse("mouseReleased", *x, *y, button, 0, *clicks, *mods)],
            Input::Wheel { x, y, dx, dy, mods } => {
                vec![("Input.dispatchMouseEvent", json!({ "type": "mouseWheel", "x": x, "y": y, "deltaX": dx, "deltaY": dy, "modifiers": mods }))]
            }
            Input::Type(t) => type_text(t),
            Input::Insert(t) => vec![("Input.insertText", json!({ "text": t }))],
            Input::Key { name, down, mods } => key_by_name(name).map(|k| vec![key(&k, *down, *mods)]).unwrap_or_default(),
            Input::Nav(u) => vec![("Page.navigate", json!({ "url": u }))],
            Input::Back => vec![("Runtime.evaluate", json!({ "expression": "history.back()" }))],
            Input::Forward => vec![("Runtime.evaluate", json!({ "expression": "history.forward()" }))],
            Input::Reload => vec![("Page.reload", json!({}))],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_is_move_then_press() {
        let c = Input::Down { x: 3.0, y: 4.0, button: "left".into(), clicks: 1, mods: 0 }.to_cdp();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].1["type"], "mousePressed");
        assert_eq!(c[1].1["buttons"], 1);
    }

    #[test]
    fn typing_mixes_keys_and_text() {
        let c = Input::Type("aあ".into()).to_cdp();
        assert_eq!(c.iter().map(|(m, _)| *m).collect::<Vec<_>>(), ["Input.dispatchKeyEvent", "Input.dispatchKeyEvent", "Input.insertText"]);
        assert!(Input::Key { name: "key.keyboard.nope".into(), down: true, mods: 0 }.to_cdp().is_empty());
        assert_eq!(Input::Key { name: "key.keyboard.enter".into(), down: true, mods: 0 }.to_cdp().len(), 1);
    }
}
