//! マイクラのキー名（`key.keyboard.enter`）と文字を、DevTools のキーに直す。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub key: &'static str,
    pub code: &'static str,
    pub vk: u32,
    /// keyDown で一緒に送る文字（Enter の "\r" など）。
    pub text: Option<&'static str>,
    /// US 配列で Shift を押して出す文字。
    pub shifted: bool,
}

const fn k(key: &'static str, code: &'static str, vk: u32) -> Key {
    Key { key, code, vk, text: None, shifted: false }
}

/// GLFW の修飾（shift 1, control 2, alt 4, super 8）を DevTools の（Alt 1, Ctrl 2, Meta 4, Shift 8）に。
pub fn cdp_modifiers(glfw: u32) -> u32 {
    let mut m = 0;
    if glfw & 1 != 0 {
        m |= 8;
    }
    if glfw & 2 != 0 {
        m |= 2;
    }
    if glfw & 4 != 0 {
        m |= 1;
    }
    if glfw & 8 != 0 {
        m |= 4;
    }
    m
}

const LETTER_CODES: [&str; 26] = [
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS",
    "KeyT", "KeyU", "KeyV", "KeyW", "KeyX", "KeyY", "KeyZ",
];
const LETTERS: [&str; 26] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z"];
const DIGIT_CODES: [&str; 10] = ["Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8", "Digit9"];
const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

/// ASCII の印字できる文字を、US 配列のキーとして。
pub fn for_char(c: char) -> Option<Key> {
    let plain = |code, vk| Some(Key { key: "", code, vk, text: None, shifted: false });
    let shift = |code, vk| Some(Key { key: "", code, vk, text: None, shifted: true });
    match c {
        'a'..='z' => plain(LETTER_CODES[(c as u8 - b'a') as usize], 0x41 + (c as u32 - 'a' as u32)),
        'A'..='Z' => shift(LETTER_CODES[(c as u8 - b'A') as usize], 0x41 + (c as u32 - 'A' as u32)),
        '0'..='9' => plain(DIGIT_CODES[(c as u8 - b'0') as usize], 0x30 + (c as u32 - '0' as u32)),
        ' ' => plain("Space", 0x20),
        '-' => plain("Minus", 0xBD),
        '=' => plain("Equal", 0xBB),
        '[' => plain("BracketLeft", 0xDB),
        ']' => plain("BracketRight", 0xDD),
        '\\' => plain("Backslash", 0xDC),
        ';' => plain("Semicolon", 0xBA),
        '\'' => plain("Quote", 0xDE),
        ',' => plain("Comma", 0xBC),
        '.' => plain("Period", 0xBE),
        '/' => plain("Slash", 0xBF),
        '`' => plain("Backquote", 0xC0),
        '!' => shift("Digit1", 0x31),
        '@' => shift("Digit2", 0x32),
        '#' => shift("Digit3", 0x33),
        '$' => shift("Digit4", 0x34),
        '%' => shift("Digit5", 0x35),
        '^' => shift("Digit6", 0x36),
        '&' => shift("Digit7", 0x37),
        '*' => shift("Digit8", 0x38),
        '(' => shift("Digit9", 0x39),
        ')' => shift("Digit0", 0x30),
        '_' => shift("Minus", 0xBD),
        '+' => shift("Equal", 0xBB),
        '{' => shift("BracketLeft", 0xDB),
        '}' => shift("BracketRight", 0xDD),
        '|' => shift("Backslash", 0xDC),
        ':' => shift("Semicolon", 0xBA),
        '"' => shift("Quote", 0xDE),
        '<' => shift("Comma", 0xBC),
        '>' => shift("Period", 0xBE),
        '?' => shift("Slash", 0xBF),
        '~' => shift("Backquote", 0xC0),
        _ => None,
    }
}

/// 文字を打たないキー（Enter・矢印など）。印字できるキーは None（文字の方で送る）。
pub fn special(name: &str) -> Option<Key> {
    let n = name.strip_prefix("key.keyboard.")?;
    let key = match n {
        "enter" | "keypad.enter" => Key { text: Some("\r"), ..k("Enter", "Enter", 0x0D) },
        "backspace" => k("Backspace", "Backspace", 0x08),
        "tab" => k("Tab", "Tab", 0x09),
        "delete" => k("Delete", "Delete", 0x2E),
        "insert" => k("Insert", "Insert", 0x2D),
        "left" => k("ArrowLeft", "ArrowLeft", 0x25),
        "up" => k("ArrowUp", "ArrowUp", 0x26),
        "right" => k("ArrowRight", "ArrowRight", 0x27),
        "down" => k("ArrowDown", "ArrowDown", 0x28),
        "home" => k("Home", "Home", 0x24),
        "end" => k("End", "End", 0x23),
        "page.up" => k("PageUp", "PageUp", 0x21),
        "page.down" => k("PageDown", "PageDown", 0x22),
        "f1" => k("F1", "F1", 0x70),
        "f2" => k("F2", "F2", 0x71),
        "f3" => k("F3", "F3", 0x72),
        "f4" => k("F4", "F4", 0x73),
        "f5" => k("F5", "F5", 0x74),
        "f6" => k("F6", "F6", 0x75),
        "f7" => k("F7", "F7", 0x76),
        "f8" => k("F8", "F8", 0x77),
        "f9" => k("F9", "F9", 0x78),
        "f10" => k("F10", "F10", 0x79),
        "f11" => k("F11", "F11", 0x7A),
        "f12" => k("F12", "F12", 0x7B),
        _ => return None,
    };
    Some(key)
}

/// Ctrl や Alt と一緒に押した文字のキー（Ctrl+A・Ctrl+C など）。
pub fn letter_or_digit(name: &str) -> Option<Key> {
    let n = name.strip_prefix("key.keyboard.")?;
    if let Some(i) = LETTERS.iter().position(|l| *l == n) {
        return Some(k(LETTERS[i], LETTER_CODES[i], 0x41 + i as u32));
    }
    if let Some(i) = DIGITS.iter().position(|d| *d == n) {
        return Some(k(DIGITS[i], DIGIT_CODES[i], 0x30 + i as u32));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps() {
        assert_eq!(for_char('a').unwrap().vk, 0x41);
        assert!(for_char('A').unwrap().shifted);
        assert_eq!(for_char('?').unwrap().code, "Slash");
        assert_eq!(for_char('あ'), None);
        assert_eq!(special("key.keyboard.enter").unwrap().text, Some("\r"));
        assert_eq!(special("key.keyboard.a"), None);
        assert_eq!(letter_or_digit("key.keyboard.v").unwrap().code, "KeyV");
        assert_eq!(cdp_modifiers(1 | 2), 8 | 2);
    }
}
