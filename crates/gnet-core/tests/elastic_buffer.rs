//! Tests ported from gnet v2.10.0 `pkg/buffer/elastic/elastic_buffer_test.go`
//! and `pkg/buffer/linkedlist/llbuffer_test.go`.
//!
//! Go uses `crypto/rand` + `math/rand`; Rust uses a deterministic xorshift64*
//! (same approach as U1's `ring_probe`) so tests are reproducible. Assertions
//! mirror the Go `require.*` calls field-by-field.

use gnet_core::elastic::ElasticBuffer;
use gnet_core::linkedlist::LinkedListBuffer;

/// xorshift64* PRNG — deterministic, no external deps.
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % (n as u64)) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(n);
        while v.len() < n {
            v.extend_from_slice(&self.next().to_le_bytes());
        }
        v.truncate(n);
        v
    }
}

// ---------- linkedlist tests (llbuffer_test.go) ----------

#[test]
fn linkedlist_buffer_basic() {
    let mut rng = Rng::new(0x12345678);
    let mut llb = LinkedListBuffer::new();
    let mut cum = 0usize;
    let mut expected = Vec::new();

    for _ in 0..100 {
        let n = rng.below(1024) + 128;
        cum += n;
        let data = rng.bytes(n);
        llb.push_back(&data);
        expected.extend_from_slice(&data);
    }
    assert_eq!(llb.len(), 100);
    assert_eq!(llb.buffered(), cum);

    let bs = llb.peek((cum / 4) as isize).unwrap_or_default();
    let mut p: Vec<u8> = bs.concat();
    let pn = p.len();
    assert_eq!(pn, cum / 4);
    assert_eq!(p.as_slice(), expected.get(..pn).unwrap_or(&[]));

    let tmp_a = rng.bytes(cum / 16);
    let tmp_b = rng.bytes(cum / 16);
    let bs = llb
        .peek_with_bytes((cum / 4) as isize, &[&tmp_a, &tmp_b])
        .unwrap_or_default();
    p = bs.concat();
    let pn = p.len();
    assert_eq!(pn, cum / 4);
    let mut tmp_buf = Vec::new();
    tmp_buf.extend_from_slice(&tmp_a);
    tmp_buf.extend_from_slice(&tmp_b);
    tmp_buf.extend_from_slice(
        expected
            .get(..pn - tmp_a.len() - tmp_b.len())
            .unwrap_or(&[]),
    );
    assert_eq!(p, tmp_buf);

    let discarded = llb.discard(pn);
    assert_eq!(discarded, pn);
    expected.drain(..pn);
    let mut out = vec![0u8; cum - pn];
    let n = llb.read(&mut out).unwrap_or(usize::MAX);
    assert_eq!(n, cum - pn);
    assert_eq!(out, expected);
    assert!(llb.is_empty());
}

#[test]
fn linkedlist_buffer_read_from() {
    let mut llb = LinkedListBuffer::new();
    let data_len = 4 * 1024;
    let data: Vec<u8> = (0..data_len).map(|i| (i % 256) as u8).collect();
    let mut cursor = std::io::Cursor::new(&data);
    let n = llb.read_from(&mut cursor).unwrap_or(u64::MAX);
    assert_eq!(n as usize, data_len);
    assert_eq!(llb.buffered(), data_len);

    llb.reset();
    let head_len = 256;
    let head: Vec<u8> = (0..head_len).map(|i| (i % 256) as u8).collect();
    llb.push_back(&head);
    let mut cursor2 = std::io::Cursor::new(&data);
    let n = llb.read_from(&mut cursor2).unwrap_or(u64::MAX);
    assert_eq!(n as usize, data_len);
    assert_eq!(llb.buffered(), head_len + data_len);

    let mut buf = vec![0u8; head_len + data_len];
    let m = llb.read(&mut buf).unwrap_or(usize::MAX);
    assert_eq!(m, head_len + data_len);
    let mut expected = head.clone();
    expected.extend_from_slice(&data);
    assert_eq!(buf, expected);
    assert!(llb.is_empty());
}

#[test]
fn linkedlist_buffer_write_to() {
    let mut rng = Rng::new(0xabcdef);
    let mut llb = LinkedListBuffer::new();
    let mut cum = 0usize;
    let mut expected = Vec::new();

    for _ in 0..20 {
        let n = rng.below(1024) + 128;
        cum += n;
        let data = rng.bytes(n);
        llb.push_back(&data);
        expected.extend_from_slice(&data);
    }
    assert_eq!(llb.len(), 20);
    assert_eq!(llb.buffered(), cum);

    let mut out = Vec::new();
    let n = llb.write_to(&mut out).unwrap_or(u64::MAX);
    assert_eq!(n as usize, cum);
    assert_eq!(out, expected);

    // Second pass: partial discard then write_to.
    llb.reset();
    let mut cum2 = 0usize;
    let mut expected2 = Vec::new();
    for _ in 0..20 {
        let n = rng.below(1024) + 128;
        cum2 += n;
        let data = rng.bytes(n);
        llb.push_back(&data);
        expected2.extend_from_slice(&data);
    }
    assert_eq!(llb.len(), 20);
    assert_eq!(llb.buffered(), cum2);

    let discarded = llb.discard(cum2 / 2);
    expected2.drain(..discarded);
    let mut out2 = Vec::new();
    let n = llb.write_to(&mut out2).unwrap_or(u64::MAX);
    assert_eq!(n as usize, cum2 - discarded);
    assert_eq!(out2, expected2);
    llb.reset();
}

// ---------- elastic buffer tests (elastic_buffer_test.go) ----------

#[test]
fn mixed_buffer_basic() {
    let max_static = 4 * 1024;
    let mut mb = match ElasticBuffer::new(max_static as isize) {
        Ok(b) => b,
        Err(_) => return,
    };
    let data_len = 5 * 1024;
    let mut rng = Rng::new(0x9999);
    let data = rng.bytes(data_len);

    let n = mb.write(&data);
    assert_eq!(n, data_len);
    assert_eq!(mb.buffered(), data_len);
    // ring_buffer.Buffered() == dataLen (all in ring since 5K > 4K static? Go says yes —
    // ring grows to hold it; list stays empty until ring hits maxStaticBytes on NEXT write).
    // Actually Go: ring.Write grows ring to fit; list only gets data when ring.Buffered >= maxStatic.
    // After first write of 5K into empty ring: ring.Buffered=5K, list empty.

    let rbn = mb.ring_buffer_len();
    mb.reset(-1);
    let new_data_len = rbn + 2 * 1024;
    let data2 = rng.bytes(new_data_len);
    let n = mb.write(&data2);
    assert_eq!(n, new_data_len);
    assert_eq!(mb.buffered(), new_data_len);
    // ring holds rbn bytes (its Len), rest spills to list.
    assert_eq!(mb.ring_buffer_buffered(), rbn);

    let bs = mb.peek(-1).unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    assert_eq!(p, data2);

    let bs = mb.peek(rbn as isize).unwrap_or_default();
    assert_eq!(bs.first().map(Vec::as_slice), data2.get(..rbn));
    let n = mb.discard(rbn);
    assert_eq!(n, rbn);
    let bs = mb.peek((new_data_len - rbn) as isize).unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    assert_eq!(p.as_slice(), data2.get(rbn..).unwrap_or(&[]));
    let n = mb.discard(new_data_len - rbn);
    assert_eq!(n, new_data_len - rbn);
    assert!(mb.is_empty());

    // Writev phase: 3 head writes + 97 list writes.
    let mut head_cum = 0usize;
    let mut cum = 0usize;
    let mut expected = Vec::new();
    let mut bs_list: Vec<Vec<u8>> = Vec::new();
    for i in 0..100 {
        let n = rng.below(512) + 128;
        cum += n;
        let data = rng.bytes(n);
        if i < 3 {
            head_cum += n;
            mb.write(&data);
        } else {
            bs_list.push(data.clone());
        }
        expected.extend_from_slice(&data);
    }
    // Writev the list portion.
    let slices: Vec<&[u8]> = bs_list.iter().map(|v| v.as_slice()).collect();
    let n = mb.writev(&slices);
    assert!(mb.ring_buffer_cap() >= max_static);
    assert_eq!(n, cum - head_cum);
    assert_eq!(mb.buffered(), cum);

    let bs = mb.peek(-1).unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    assert_eq!(p, expected);

    let mut out = vec![0u8; cum];
    let n = mb.read(&mut out).unwrap_or(usize::MAX);
    assert_eq!(n, cum);
    assert_eq!(out, expected);
    assert!(mb.is_empty());
}

#[test]
fn mixed_buffer_read_from() {
    let max_static = 2 * 1024;
    let mut mb = match ElasticBuffer::new(max_static as isize) {
        Ok(b) => b,
        Err(_) => return,
    };
    let data_len = 2 * 1024;
    let data: Vec<u8> = (0..data_len)
        .map(|i: usize| ((i.wrapping_mul(7)) % 256) as u8)
        .collect();

    let mut cursor = std::io::Cursor::new(&data);
    let n = mb.read_from(&mut cursor).unwrap_or(u64::MAX);
    assert_eq!(n as usize, data_len);
    assert_eq!(mb.buffered(), data_len);

    let new_data: Vec<u8> = (0..data_len)
        .map(|i: usize| ((i.wrapping_mul(13)) % 256) as u8)
        .collect();
    let mut cursor2 = std::io::Cursor::new(&new_data);
    let n = mb.read_from(&mut cursor2).unwrap_or(u64::MAX);
    assert_eq!(n as usize, data_len);
    assert_eq!(mb.buffered(), 2 * data_len);
    // list_buffer should be non-empty (Go asserts this).
    assert!(!mb.list_buffer_is_empty());

    let mut buf = vec![0u8; data_len];
    let m = mb.read(&mut buf).unwrap_or(usize::MAX);
    assert_eq!(m, data_len);
    assert_eq!(buf, data);

    let bs = mb.peek(data_len as isize).unwrap_or_default();
    let p: Vec<u8> = bs.concat();
    assert_eq!(p, new_data);
    let m = mb.discard(data_len);
    assert_eq!(m, data_len);
    assert!(mb.is_empty());
}

#[test]
fn mixed_buffer_write_to() {
    let max_static = 4 * 1024;
    let mut mb = match ElasticBuffer::new(max_static as isize) {
        Ok(b) => b,
        Err(_) => return,
    };
    let mut rng = Rng::new(0x777);
    let mut head_cum = 0usize;
    let mut cum = 0usize;
    let mut bs_list: Vec<Vec<u8>> = Vec::new();
    let mut expected = Vec::new();

    for i in 0..50 {
        let n = rng.below(512) + 128;
        cum += n;
        let data = rng.bytes(n);
        if i < 3 {
            head_cum += n;
            mb.write(&data);
        } else {
            bs_list.push(data.clone());
        }
        expected.extend_from_slice(&data);
    }
    let slices: Vec<&[u8]> = bs_list.iter().map(|v| v.as_slice()).collect();
    let n = mb.writev(&slices);
    assert_eq!(n, cum - head_cum);
    assert_eq!(mb.buffered(), cum);

    let mut out = Vec::new();
    let m = mb.write_to(&mut out).unwrap_or(u64::MAX);
    assert_eq!(m as usize, cum);
    assert_eq!(out, expected);
    assert!(mb.is_empty());
}

// ---------- edge cases ----------

#[test]
fn elastic_buffer_new_rejects_nonpositive() {
    assert!(ElasticBuffer::new(0).is_err());
    assert!(ElasticBuffer::new(-1).is_err());
}

#[test]
fn elastic_ring_buffer_lazy_alloc() {
    let mut rb = gnet_core::elastic::ElasticRingBuffer::new();
    assert!(rb.is_empty());
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.len(), 0);
    // Read on empty → RingError.
    let mut p = [0u8; 4];
    assert!(rb.read(&mut p).is_err());
    // Write allocates.
    assert_eq!(rb.write(b"hi"), 2);
    assert_eq!(rb.buffered(), 2);
    let n = rb.read(&mut p).unwrap_or(usize::MAX);
    assert_eq!(n, 2);
    assert_eq!(&p[..2], b"hi");
}

#[test]
fn linkedlist_peek_short_buffer() {
    let mut llb = LinkedListBuffer::new();
    llb.push_back(b"abc");
    assert!(llb.peek(100).is_err());
    assert!(llb.peek_with_bytes(100, &[b"x"]).is_err());
    let ok = llb.peek(3).unwrap_or_default();
    assert_eq!(ok.concat(), b"abc");
}
