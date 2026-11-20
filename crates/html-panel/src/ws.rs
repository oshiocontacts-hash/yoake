//! 小さな WebSocket クライアント（Chrome の DevTools と話すだけ）。
//!
//! テキストの送受信と ping/pong・close だけ。TLS なし（127.0.0.1 だけ）。

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;

pub struct WsReader {
    stream: BufReader<TcpStream>,
}

pub struct WsWriter {
    stream: TcpStream,
    seed: u32,
}

pub enum Message {
    Text(String),
    Ping(Vec<u8>),
    Close,
}

fn seed() -> u32 {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    (n as u32) ^ ((n >> 32) as u32) ^ 0x9e37_79b9 ^ std::process::id()
}

/// `ws://127.0.0.1:port/path` に繋ぐ。
pub fn connect(port: u16, path: &str) -> io::Result<(WsReader, WsWriter)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut key = [0u8; 16];
    let mut s = seed();
    for b in &mut key {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        *b = s as u8;
    }
    let key = base64::engine::general_purpose::STANDARD.encode(key);
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )?;
    stream.flush()?;
    let mut reader = BufReader::with_capacity(1 << 20, stream.try_clone()?);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    if !status.contains(" 101") {
        return Err(io::Error::other(format!("websocket handshake refused: {}", status.trim())));
    }
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::Error::other("websocket handshake cut"));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }
    // 読みは止まらないように（ページが静かでも待ち続ける）。Windows では複製したソケットごとに期限があるので両方
    stream.set_read_timeout(None)?;
    reader.get_ref().set_read_timeout(None)?;
    Ok((WsReader { stream: reader }, WsWriter { stream, seed: seed() | 1 }))
}

impl WsReader {
    /// 読みの期限（None で無期限）。期限が切れると read は WouldBlock / TimedOut の Err を返す。
    /// フレームの途中で切れると続きが読めなくなるので、静かな時（返事を待つ間など）にだけ使う。
    pub fn set_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.stream.get_ref().set_read_timeout(timeout)
    }

    pub fn read(&mut self) -> io::Result<Message> {
        let mut text: Vec<u8> = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.stream.read_exact(&mut head)?;
            let fin = head[0] & 0x80 != 0;
            let opcode = head[0] & 0x0f;
            let masked = head[1] & 0x80 != 0;
            let mut len = (head[1] & 0x7f) as u64;
            if len == 126 {
                let mut b = [0u8; 2];
                self.stream.read_exact(&mut b)?;
                len = u16::from_be_bytes(b) as u64;
            } else if len == 127 {
                let mut b = [0u8; 8];
                self.stream.read_exact(&mut b)?;
                len = u64::from_be_bytes(b);
            }
            if len > 64 << 20 {
                return Err(io::Error::other("websocket frame too large"));
            }
            let mut mask = [0u8; 4];
            if masked {
                self.stream.read_exact(&mut mask)?;
            }
            let mut payload = vec![0u8; len as usize];
            self.stream.read_exact(&mut payload)?;
            if masked {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= mask[i & 3];
                }
            }
            match opcode {
                0 | 1 | 2 => {
                    text.extend_from_slice(&payload);
                    if fin {
                        return Ok(Message::Text(String::from_utf8_lossy(&text).into_owned()));
                    }
                }
                8 => return Ok(Message::Close),
                9 => return Ok(Message::Ping(payload)),
                _ => {}
            }
        }
    }
}

impl WsWriter {
    fn frame(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut out = Vec::with_capacity(payload.len() + 14);
        out.push(0x80 | opcode);
        let len = payload.len();
        if len < 126 {
            out.push(0x80 | len as u8);
        } else if len <= u16::MAX as usize {
            out.push(0x80 | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let mask = self.seed.to_le_bytes();
        out.extend_from_slice(&mask);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i & 3]));
        self.stream.write_all(&out)
    }

    pub fn text(&mut self, text: &str) -> io::Result<()> {
        self.frame(1, text.as_bytes())
    }

    pub fn pong(&mut self, payload: &[u8]) -> io::Result<()> {
        self.frame(10, payload)
    }

    pub fn close(&mut self) {
        let _ = self.frame(8, &[]);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}
