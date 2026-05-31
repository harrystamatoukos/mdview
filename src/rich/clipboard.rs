//! System-clipboard write via the OSC 52 terminal escape sequence.
//!
//! Rich mode already requires a graphics-capable terminal (kitty, WezTerm,
//! Ghostty, Konsole), all of which honor OSC 52 — so this is sufficient on its
//! own, with no native-clipboard dependency. OSC 52 also works over SSH because
//! the *local* terminal emulator performs the write, not the remote process.
//!
//! The bytes are written straight to `/dev/tty` rather than through the ratatui
//! backend so they don't get interleaved with the kitty graphics transmit or
//! overwritten by the next frame.

use std::io::Write;

/// Copy `text` to the system clipboard. Best-effort: a terminal that doesn't
/// support OSC 52 simply ignores the sequence, so failure is silent.
pub fn copy(text: &str) {
    if text.is_empty() {
        return;
    }
    let payload = base64_encode(text.as_bytes());
    // OSC 52 ; c (clipboard) ; <base64> BEL
    let seq = format!("\x1b]52;c;{payload}\x07");
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
        let _ = tty.write_all(seq.as_bytes());
        let _ = tty.flush();
    }
}

/// Standard base64 (RFC 4648) encoding. Hand-rolled to keep the dependency tree
/// lean — the payloads here are short selections, so speed is irrelevant.
pub(super) fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}
