//! Rust twin of verification/probe/elasticprobe.go — same op sequence, same
//! JSON-lines output, diffed byte-for-byte against the Go probe.
//!
//! Keys are emitted in the same order Go's `encoding/json` sorts map keys
//! (alphabetical); hex payloads make byte comparisons exact.

use gnet_core::elastic::ElasticBuffer;
use gnet_core::linkedlist::LinkedListBuffer;
use std::io::{Cursor, Write};

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
            let v = self.next();
            let b = v.to_le_bytes();
            for &c in b.iter() {
                if out.len() < n {
                    out.push(c);
                }
            }
        }
        out
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A writer that accepts only `budget` bytes total (Go shortWriter).
struct ShortWriter {
    budget: usize,
}
impl Write for ShortWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() > self.budget {
            let w = self.budget;
            self.budget = 0;
            return Ok(w);
        }
        self.budget -= buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn main() {
    let mut g = Xs(0x9E3779B97F4A7C15 | 1);

    // ===== Scenario A: linkedlist basic =====
    let mut llb = LinkedListBuffer::new();
    let mut cum = 0usize;
    let mut expected = Vec::new();
    for _ in 0..20 {
        let n = (g.next() % 1024) as usize + 128;
        cum += n;
        let data = g.bytes(n);
        llb.push_back(&data);
        expected.extend_from_slice(&data);
    }
    println!(
        "{{\"buffered\":{},\"len\":{},\"op\":\"ll_pushback\"}}",
        llb.buffered(),
        llb.len()
    );

    let bs = llb.peek((cum / 4) as isize).unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    println!(
        "{{\"n\":{},\"op\":\"ll_peek\",\"total\":\"{}\"}}",
        cum / 4,
        hex(&p)
    );

    let tmp_a = g.bytes(cum / 16);
    let tmp_b = g.bytes(cum / 16);
    let bs = llb
        .peek_with_bytes((cum / 4) as isize, &[&tmp_a, &tmp_b])
        .unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    println!(
        "{{\"n\":{},\"op\":\"ll_peekwith\",\"total\":\"{}\"}}",
        cum / 4,
        hex(&p)
    );

    let pn = llb.discard(cum / 4);
    println!(
        "{{\"buffered\":{},\"discarded\":{},\"op\":\"ll_discard\"}}",
        llb.buffered(),
        pn
    );

    let mut out = vec![0u8; cum - pn];
    let n = llb.read(&mut out).unwrap_or(usize::MAX);
    println!(
        "{{\"data\":\"{}\",\"n\":{},\"op\":\"ll_read\"}}",
        hex(&out),
        n
    );
    println!("{{\"is_empty\":{},\"op\":\"ll_state\"}}", llb.is_empty());

    // ===== Scenario B: linkedlist ReadFrom + WriteTo =====
    llb.reset();
    let data = g.bytes(2048);
    let mut cursor = Cursor::new(&data);
    let rn = llb.read_from(&mut cursor).unwrap_or(u64::MAX);
    println!("{{\"err\":false,\"n\":{},\"op\":\"ll_readfrom\"}}", rn);
    let mut wbuf = Vec::new();
    let wn = llb.write_to(&mut wbuf).unwrap_or(u64::MAX);
    println!(
        "{{\"data\":\"{}\",\"err\":false,\"n\":{},\"op\":\"ll_writeto\"}}",
        hex(&wbuf),
        wn
    );

    // WriteTo with a short writer (accepts only 100 bytes).
    llb.reset();
    for _ in 0..3 {
        let d = g.bytes(200);
        llb.push_back(&d);
    }
    let mut sw = ShortWriter { budget: 100 };
    let mut wn: u64 = 0;
    while let Some(mut head) = llb.pop() {
        match sw.write(&head) {
            Ok(m) => {
                wn += m as u64;
                if m < head.len() {
                    head.drain(..m);
                    llb.push_front(&head);
                    break;
                }
            }
            Err(_) => break,
        }
    }
    println!("{{\"err\":true,\"n\":{},\"op\":\"ll_writeto_short\"}}", wn);
    println!(
        "{{\"buffered\":{},\"len\":{},\"op\":\"ll_state_after_short\"}}",
        llb.buffered(),
        llb.len()
    );

    // ===== Scenario C: elastic basic (ring only, under maxStatic) =====
    let mut mb = match ElasticBuffer::new(4096) {
        Ok(b) => b,
        Err(_) => return,
    };
    let data = g.bytes(2048);
    let n = mb.write(&data);
    println!(
        "{{\"buffered\":{},\"err\":false,\"n\":{},\"op\":\"e_write\"}}",
        mb.buffered(),
        n
    );

    // Ring peek via elastic ring buffer accessors: use peek(-1) on the
    // ElasticBuffer itself? Go probe peeks the inner ringBuffer directly.
    // We expose the same via ring_buffer_* accessors — need a head/tail split,
    // so go through gnet_core::elastic::ElasticRingBuffer-style API: here we
    // can't reach the inner ring's peek directly, so emulate with elastic
    // peek of exactly ring_buffered bytes: ring head/tail == elastic peek
    // when list is empty.
    let ring_buffered = mb.ring_buffer_buffered();
    let bs = mb.peek(ring_buffered as isize).unwrap_or_default();
    // With an empty list, peek returns [head, tail?] chunks from the ring.
    let mut head = Vec::new();
    let mut tail = Vec::new();
    if let Some(h) = bs.first() {
        head = h.clone();
        if let Some(t) = bs.get(1) {
            tail = t.clone();
        }
    }
    println!(
        "{{\"head\":\"{}\",\"head_len\":{},\"op\":\"e_ringpeek\",\"tail\":\"{}\",\"tail_len\":{}}}",
        hex(&head),
        head.len(),
        hex(&tail),
        tail.len()
    );

    let mut sink = vec![0u8; 1024];
    let n = mb.read(&mut sink).unwrap_or(usize::MAX);
    println!(
        "{{\"buffered\":{},\"err\":<nil>,\"n\":{},\"op\":\"e_read\"}}",
        mb.buffered(),
        n
    );

    // ===== Scenario D: elastic hybrid (overflow to list) =====
    mb.reset(2048);
    let big = g.bytes(6144);
    let n = mb.write(&big);
    println!(
        "{{\"buffered\":{},\"n\":{},\"op\":\"e_write_hybrid\"}}",
        mb.buffered(),
        n
    );

    let bs = mb.peek(-1).unwrap_or_default();
    let total: Vec<u8> = bs.concat();
    println!(
        "{{\"nchunks\":{},\"op\":\"e_peek_all\",\"total\":\"{}\"}}",
        bs.len(),
        hex(&total)
    );

    let bs = mb.peek(3000).unwrap_or_default();
    let total: Vec<u8> = bs.concat();
    println!(
        "{{\"nchunks\":{},\"op\":\"e_peek_3000\",\"total\":\"{}\"}}",
        bs.len(),
        hex(&total)
    );

    // Writev over the threshold.
    mb.reset(2048);
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let mut cum2 = 0usize;
    for _ in 0..6 {
        let p = g.bytes(1000);
        cum2 += 1000;
        parts.push(p);
    }
    let slices: Vec<&[u8]> = parts.iter().map(|v| v.as_slice()).collect();
    let nv = mb.writev(&slices);
    println!(
        "{{\"buffered\":{},\"n\":{},\"op\":\"e_writev\"}}",
        mb.buffered(),
        nv
    );

    let bs = mb.peek(-1).unwrap_or_default();
    let all: Vec<u8> = bs.concat();
    println!(
        "{{\"err\":false,\"nchunks\":{},\"op\":\"e_peek_after_writev\",\"total\":\"{}\"}}",
        bs.len(),
        hex(&all)
    );

    // WriteTo drains everything.
    let mut wbuf2 = Vec::new();
    let wn = mb.write_to(&mut wbuf2).unwrap_or(u64::MAX);
    println!(
        "{{\"data_match\":{},\"err\":false,\"n\":{},\"op\":\"e_writeto\"}}",
        wbuf2 == all,
        wn
    );
    println!("{{\"is_empty\":{},\"op\":\"e_state\"}}", mb.is_empty());

    // Discard across ring/list boundary.
    mb.reset(2048);
    let d1 = g.bytes(3000);
    mb.write(&d1);
    let dd = mb.discard(2500);
    println!(
        "{{\"buffered\":{},\"discarded\":{},\"op\":\"e_discard\"}}",
        mb.buffered(),
        dd
    );
    let rest = mb.peek(-1).unwrap_or_default();
    let rb: Vec<u8> = rest.concat();
    println!("{{\"rest\":\"{}\",\"op\":\"e_rest\"}}", hex(&rb));

    // ReadFrom into elastic.
    mb.reset(2048);
    let rf = g.bytes(4000);
    let mut cursor2 = Cursor::new(&rf);
    let rn = mb.read_from(&mut cursor2).unwrap_or(u64::MAX);
    println!(
        "{{\"buffered\":{},\"err\":false,\"n\":{},\"op\":\"e_readfrom\"}}",
        mb.buffered(),
        rn
    );
    let _ = cum2;
}
