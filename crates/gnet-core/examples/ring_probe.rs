//! Rust twin of reference/gnet-go/cmd/ringprobe — same op sequence, same
//! JSON-lines output, diffed byte-for-byte against the Go probe.
//!
//! Go's `encoding/json` marshals maps with keys in sorted order and escapes
//! `<`, `>`, `&` as <, >, &; every line below emits keys in
//! that same alphabetical order and uses NIL_ESC for the escaped "<nil>" so
//! the two outputs diff cleanly.

use gnet_core::ring::RingBuffer;
use std::io::{Error, ErrorKind, Read, Result};

/// The exact bytes Go's json.Marshal emits for the string "<nil>".
const NIL_ESC: &str = "\\u003cnil\\u003e";

/// Mimics Go's `bytes.Reader` for the `read_from` probe: at exhaustion it
/// returns `Ok(0)` on a non-empty buffer (Rust EOF) and
/// `UnexpectedEof` on an empty buffer — the signal `read_from` maps to Go's
/// `io.EOF`-on-empty-read. Mid-stream an empty read returns `Ok(0)`.
struct GoBytes<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> Read for GoBytes<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let remaining = self.data.len().saturating_sub(self.pos);
        if remaining == 0 {
            return if buf.is_empty() {
                Err(Error::new(ErrorKind::UnexpectedEof, "eof"))
            } else {
                Ok(0)
            };
        }
        let n = remaining.min(buf.len());
        if let Some(dst) = buf.get_mut(..n) {
            if let Some(src) = self.data.get(self.pos..self.pos + n) {
                dst.copy_from_slice(src);
            }
        }
        self.pos += n;
        Ok(n)
    }
}

/// xorshift64* — must match the Go probe's generator exactly.
struct Xs(u64);
impl Xs {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let v = self.next().to_le_bytes();
            for &b in v.iter() {
                if out.len() < n {
                    out.push(b);
                }
            }
        }
        out
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn main() {
    let mut g = Xs(0x9E3779B97F4A7C15 | 1);

    // Scenario 1: sized buffer, wrap-around.
    let mut rb = RingBuffer::new(64);
    println!(
        "{{\"available\":{},\"buffered\":{},\"cap\":{},\"is_empty\":{},\"is_full\":{},\"len\":{},\"op\":\"new\",\"size\":64}}",
        rb.available(), rb.buffered(), rb.cap(), rb.is_empty(), rb.is_full(), rb.len()
    );

    let data = g.bytes(64);
    let n = rb.write(&data);
    println!(
        "{{\"available\":{},\"buffered\":{},\"cap\":{},\"is_empty\":{},\"is_full\":{},\"len\":{},\"n\":{},\"op\":\"write\"}}",
        rb.available(), rb.buffered(), rb.cap(), rb.is_empty(), rb.is_full(), rb.len(), n
    );

    let d = rb.discard(32);
    println!(
        "{{\"available\":{},\"buffered\":{},\"discarded\":{},\"op\":\"discard\"}}",
        rb.available(),
        rb.buffered(),
        d
    );
    let more = g.bytes(16);
    let n = rb.write(&more);
    println!(
        "{{\"available\":{},\"buffered\":{},\"n\":{},\"op\":\"write\"}}",
        rb.available(),
        rb.buffered(),
        n
    );

    let (head, tail) = rb.peek(-1);
    println!(
        "{{\"head\":\"{}\",\"head_len\":{},\"n\":-1,\"op\":\"peek\",\"tail\":\"{}\",\"tail_len\":{}}}",
        hex(&head), head.len(), hex(&tail), tail.len()
    );
    let (head, tail) = rb.peek(10);
    println!(
        "{{\"head\":\"{}\",\"head_len\":{},\"n\":10,\"op\":\"peek\",\"tail\":\"{}\",\"tail_len\":{}}}",
        hex(&head), head.len(), hex(&tail), tail.len()
    );
    let (head, tail) = rb.peek(40);
    println!(
        "{{\"head\":\"{}\",\"head_len\":{},\"n\":40,\"op\":\"peek\",\"tail\":\"{}\",\"tail_len\":{}}}",
        hex(&head), head.len(), hex(&tail), tail.len()
    );

    let b = rb.bytes();
    println!(
        "{{\"hex\":\"{}\",\"len\":{},\"op\":\"bytes\"}}",
        hex(&b),
        b.len()
    );

    let mut p = vec![0u8; 20];
    let rn = rb.read(&mut p).unwrap_or(usize::MAX);
    let rd: Vec<u8> = if rn == usize::MAX {
        Vec::new()
    } else {
        p.iter().take(rn).copied().collect()
    };
    println!(
        "{{\"buffered\":{},\"data\":\"{}\",\"err\":\"{}\",\"n\":{},\"op\":\"read\"}}",
        rb.buffered(),
        hex(&rd),
        NIL_ESC,
        rn
    );

    let c1 = rb.read_byte();
    let c2 = rb.read_byte();
    println!(
        "{{\"b1\":{},\"b2\":{},\"buffered\":{},\"e1\":\"{}\",\"e2\":\"{}\",\"op\":\"readbyte\"}}",
        c1.unwrap_or(u8::MAX),
        c2.unwrap_or(u8::MAX),
        rb.buffered(),
        NIL_ESC,
        NIL_ESC
    );

    let d = rb.discard(1000);
    println!(
        "{{\"available\":{},\"buffered\":{},\"discarded\":{},\"is_empty\":{},\"op\":\"discard\"}}",
        rb.available(),
        rb.buffered(),
        d,
        rb.is_empty()
    );

    // Scenario 2: zero-size buffer growth.
    let mut rb2 = RingBuffer::new(0);
    println!(
        "{{\"available\":{},\"buffered\":{},\"cap\":{},\"len\":{},\"op\":\"new\",\"size\":0}}",
        rb2.available(),
        rb2.buffered(),
        rb2.cap(),
        rb2.len()
    );
    let big = g.bytes(1500);
    let n = rb2.write(&big);
    println!(
        "{{\"available\":{},\"buffered\":{},\"cap\":{},\"len\":{},\"n\":{},\"op\":\"write\"}}",
        rb2.available(),
        rb2.buffered(),
        rb2.cap(),
        rb2.len(),
        n
    );

    // Scenario 3: ReadFrom / WriteTo.
    let mut rb3 = RingBuffer::new(0);
    let src = g.bytes(4096);
    let mut reader = GoBytes { data: &src, pos: 0 };
    let rn64 = rb3.read_from(&mut reader).unwrap_or(u64::MAX);
    println!(
        "{{\"buffered\":{},\"cap\":{},\"err\":\"{}\",\"len\":{},\"n\":{},\"op\":\"readfrom\"}}",
        rb3.buffered(),
        rb3.cap(),
        NIL_ESC,
        rb3.len(),
        rn64
    );
    let (head, tail) = rb3.peek(-1);
    let mut all = head.clone();
    all.extend_from_slice(&tail);
    println!(
        "{{\"head_len\":{},\"match_src\":{},\"n\":-1,\"op\":\"peek\",\"tail_len\":{}}}",
        head.len(),
        all == src,
        tail.len()
    );

    let mut out: Vec<u8> = Vec::new();
    let wn = rb3.write_to(&mut out).unwrap_or(u64::MAX);
    println!(
        "{{\"err\":\"{}\",\"is_empty\":{},\"match\":{},\"n\":{},\"op\":\"writeto\"}}",
        NIL_ESC,
        rb3.is_empty(),
        out == src,
        wn
    );

    // WriteTo on wrapped buffer.
    let mut rb4 = RingBuffer::new(128);
    let _ = rb4.write(&g.bytes(128)); // full
    let _ = rb4.discard(64);
    let _ = rb4.write(&g.bytes(32));
    let mut out2: Vec<u8> = Vec::new();
    let wn = rb4.write_to(&mut out2).unwrap_or(u64::MAX);
    println!(
        "{{\"buffered\":{},\"err\":\"{}\",\"is_empty\":{},\"n\":{},\"op\":\"writeto\",\"out_len\":{}}}",
        rb4.buffered(),
        NIL_ESC,
        rb4.is_empty(),
        wn,
        out2.len()
    );

    // Scenario 4: write_byte / read_byte.
    let mut rb5 = RingBuffer::new(2);
    rb5.write_byte(b'a');
    rb5.write_byte(b'b');
    rb5.write_byte(b'c'); // triggers grow 2->4
    let bb = rb5.bytes();
    println!(
        "{{\"available\":{},\"buffered\":{},\"cap\":{},\"hex\":\"{}\",\"len\":{},\"op\":\"bytes\"}}",
        rb5.available(),
        rb5.buffered(),
        rb5.cap(),
        hex(&bb),
        bb.len()
    );
    let x1 = rb5.read_byte().unwrap_or(u8::MAX);
    let x2 = rb5.read_byte().unwrap_or(u8::MAX);
    let x3 = rb5.read_byte().unwrap_or(u8::MAX);
    println!(
        "{{\"b\":[{},{},{}],\"buffered\":{},\"is_empty\":{},\"op\":\"readbyte3\"}}",
        x1,
        x2,
        x3,
        rb5.buffered(),
        rb5.is_empty()
    );
}
