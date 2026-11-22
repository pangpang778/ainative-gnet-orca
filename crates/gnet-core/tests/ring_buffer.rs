//! Ports of the 7 Go tests in `reference/gnet-go/pkg/buffer/ring/ring_buffer_test.go`.
//!
//! Deterministic parts are verbatim; `crypto/rand` data is replaced by a
//! seeded xorshift64* stream (contents are irrelevant — only lengths, caps and
//! round-trips are asserted, exactly as in Go).

use gnet_core::ring::{RingBuffer, DEFAULT_BUFFER_SIZE};
use std::io::Cursor;

/// xorshift64* — deterministic stand-in for `crypto/rand.Read`.
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
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
            out.extend_from_slice(&self.next_u64().to_le_bytes());
        }
        out.truncate(n);
        out
    }
}

fn repeat_abcd(n: usize) -> Vec<u8> {
    b"abcd".iter().copied().cycle().take(n).collect()
}

/// Go `TestRingBuffer_Write`.
#[test]
fn test_ring_buffer_write() {
    let mut rb = RingBuffer::new(64);

    assert!(rb.read_byte().is_err(), "expect err on empty read_byte");
    assert!(rb.is_empty(), "expect IsEmpty is true but got false");
    assert!(!rb.is_full(), "expect IsFull is false but got true");
    assert_eq!(rb.buffered(), 0, "expect len 0 bytes");
    assert_eq!(rb.available(), 64, "expect free 64 bytes");

    // write 4 * 4 = 16 bytes
    let data = repeat_abcd(16);
    let n = rb.write(&data);
    assert_eq!(n, 16, "expect write 16 bytes but got {n}");
    assert_eq!(rb.buffered(), 16, "expect len 16 bytes");
    assert_eq!(rb.available(), 48, "expect free 48 bytes");
    assert_eq!(rb.bytes(), data, "expect 4 abcd");

    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // write 48 bytes, should full
    let data = repeat_abcd(48);
    let n = rb.write(&data);
    assert_eq!(n, 48, "expect write 48 bytes but got {n}");
    assert_eq!(rb.buffered(), 64, "expect len 64 bytes");
    assert_eq!(rb.available(), 0, "expect free 0 bytes");
    assert_eq!(rb.bytes(), repeat_abcd(64), "expect 16 abcd");

    assert!(!rb.is_empty());
    assert!(rb.is_full());
    assert!(rb.is_full());

    // write more 4 bytes, should scale from 64 to 128 bytes.
    let n = rb.write(b"abcd");
    assert_eq!(n, 4, "expect write 4 bytes but got {n}");
    let size = rb.cap();
    assert_eq!(rb.buffered(), 68, "expect len 68 bytes");
    assert_eq!(rb.available(), size - 68, "expect free 60 bytes");

    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // reset and write a long slice
    rb.reset();
    let n = rb.write(&repeat_abcd(80));
    assert_eq!(n, 80, "expect write 80 bytes but got {n}");
    assert_eq!(rb.buffered(), 80, "expect len 80 bytes");
    assert_eq!(rb.available(), size - 80, "expect free 48 bytes");
    assert!(!rb.bytes().is_empty());

    assert!(!rb.is_empty());
    assert!(!rb.is_full());
    assert_eq!(rb.bytes(), repeat_abcd(80), "expect 20 abcd");

    rb.reset();
    let size = rb.cap();
    // write 4 * 2 = 8 bytes
    let n = rb.write(&repeat_abcd(8));
    assert_eq!(n, 8, "expect write 16 bytes but got {n}");
    assert_eq!(rb.buffered(), 8, "expect len 16 bytes");
    assert_eq!(rb.available(), size - 8, "expect free 48 bytes");
    let mut buf = vec![0u8; 5];
    let _ = rb.read(&mut buf);
    assert_eq!(rb.buffered(), 3, "expect len 3 bytes");
    let _ = rb.write(&repeat_abcd(60));

    let mut want = b"bcd".to_vec();
    want.extend_from_slice(&repeat_abcd(60));
    assert_eq!(rb.bytes(), want, "expect 63 ...");

    rb.reset();
    let n = rb.write(&repeat_abcd(128));
    assert_eq!(n, 128, "expect write 128 bytes but got {n}");
    assert_eq!(rb.available(), 0, "expect free 0 bytes");
    let mut buf = vec![0u8; 16];
    let _ = rb.read(&mut buf);
    let n = rb.write(b"1234123412341234");
    assert_eq!(n, 16, "expect write 16 bytes but got {n}");
    assert_eq!(rb.available(), 0, "expect free 0 bytes");
    let mut want = buf.clone();
    want.extend_from_slice(&rb.bytes());
    let mut want_full = repeat_abcd(128);
    want_full.extend_from_slice(b"1234123412341234");
    assert_eq!(want, want_full, "expect 16 abcd and 4 1234");
}

/// Go `TestZeroRingBuffer`.
#[test]
fn test_zero_ring_buffer() {
    let mut rb = RingBuffer::new(0);
    let (head, tail) = rb.peek(2);
    assert!(head.is_empty(), "head should be empty");
    assert!(tail.is_empty(), "tail should be empty");
    let (head, tail) = rb.peek(-1);
    assert!(head.is_empty(), "head should be empty");
    assert!(tail.is_empty(), "tail should be empty");
    assert_eq!(rb.buffered(), 0, "expect length is 0");
    assert_eq!(rb.available(), 0, "expect free is 0");
    let buf = b"123412341234123412341234123412341234123412341234".to_vec(); // 48 bytes
    let _ = rb.write(&buf);
    assert_eq!(rb.len(), DEFAULT_BUFFER_SIZE, "expect rb.len()=1024");
    assert_eq!(rb.cap(), DEFAULT_BUFFER_SIZE, "expect rb.cap()=1024");
    assert_eq!(rb.bytes(), buf, "expect it is equal");
    let _ = rb.discard(48);
    assert!(rb.is_empty() && rb.buffered() == 0, "expect rb is empty");
}

/// Go `TestRingBufferGrow`.
#[test]
fn test_ring_buffer_grow() {
    let mut rng = XorShift::new(0x9E3779B97F4A7C15);

    let mut rb = RingBuffer::new(0);
    let (head, tail) = rb.peek(2);
    assert!(head.is_empty());
    assert!(tail.is_empty());
    let data = rng.bytes(DEFAULT_BUFFER_SIZE + 1);
    let n = rb.write(&data);
    assert_eq!(n, DEFAULT_BUFFER_SIZE + 1);
    assert_eq!(rb.cap(), 2 * DEFAULT_BUFFER_SIZE);
    assert_eq!(rb.len(), 2 * DEFAULT_BUFFER_SIZE);
    assert_eq!(rb.buffered(), DEFAULT_BUFFER_SIZE + 1);
    assert_eq!(rb.available(), DEFAULT_BUFFER_SIZE - 1);
    assert_eq!(rb.bytes(), data);

    let mut rb = RingBuffer::new(DEFAULT_BUFFER_SIZE);
    let new_data = rng.bytes(3 * 512);
    let n = rb.write(&new_data);
    assert_eq!(n, 3 * 512);
    assert_eq!(rb.cap(), 2 * DEFAULT_BUFFER_SIZE);
    assert_eq!(rb.len(), 2 * DEFAULT_BUFFER_SIZE);
    assert_eq!(rb.buffered(), 3 * 512);
    assert_eq!(rb.available(), 512);
    assert_eq!(rb.bytes(), new_data);

    rb.reset();
    let data = rng.bytes(4096); // bufferGrowThreshold
    let n = rb.write(&data);
    assert_eq!(n, 4096);
    assert_eq!(rb.cap(), 4096);
    assert_eq!(rb.len(), 4096);
    assert_eq!(rb.buffered(), 4096);
    assert_eq!(rb.available(), 0);
    assert_eq!(rb.bytes(), data);
    let new_data = rng.bytes(4096 / 2 + 1);
    let n = rb.write(&new_data);
    assert_eq!(n, 4096 / 2 + 1);
    // Go: 1.25 * (1.25 * 4096) = 6400
    assert_eq!(rb.cap(), 6400);
    assert_eq!(rb.len(), 6400);
    assert_eq!(rb.buffered(), 4096 + 2049);
    assert_eq!(rb.available(), 6400 - rb.buffered());
    let mut want = data.clone();
    want.extend_from_slice(&new_data);
    assert_eq!(rb.bytes(), want);
}

/// Go `TestRingBuffer_Read`.
#[test]
fn test_ring_buffer_read() {
    let mut rb = RingBuffer::new(64);

    assert!(rb.is_empty());
    assert!(!rb.is_full());
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.available(), 64);

    // read empty
    let mut buf = vec![0u8; 1024];
    let res = rb.read(&mut buf);
    assert!(res.is_err(), "expect ErrIsEmpty");
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.available(), 64);

    // write 16 bytes to read
    let _ = rb.write(&repeat_abcd(16));
    let n = rb.read(&mut buf).unwrap_or(usize::MAX);
    assert_eq!(n, 16, "expect read 16 bytes but got {n}");
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.available(), 64);

    // write long slice to read
    let _ = rb.write(&repeat_abcd(80));
    let n = rb.read(&mut buf).unwrap_or(usize::MAX);
    assert_eq!(n, 80, "expect read 80 bytes but got {n}");
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.available(), rb.cap(), "expect free = cap");

    rb.reset();
    let _ = rb.write(
        &b"1234"
            .iter()
            .copied()
            .cycle()
            .take(128)
            .collect::<Vec<_>>(),
    );
    assert!(rb.is_full(), "ring buffer should be full");
    assert_eq!(rb.available(), 0);
    let (head, tail) = rb.peek(64);
    assert_eq!(head.len(), 64);
    assert!(tail.is_empty());
    let want_head: Vec<u8> = b"1234".iter().copied().cycle().take(64).collect();
    assert_eq!(head, want_head);
    let _ = rb.discard(64);
    let _ = rb.write(&b"1234".iter().copied().cycle().take(16).collect::<Vec<_>>());
    let (head, tail) = rb.peek(128);
    assert_eq!(head.len(), 64);
    assert_eq!(tail.len(), 16);
    let (head, tail) = rb.peek(-1);
    assert_eq!(head.len(), 64);
    assert_eq!(tail.len(), 16);

    let _ = rb.discard(64);
    let _ = rb.discard(16);
    assert!(rb.is_empty(), "should be empty");
}

/// Go `TestRingBuffer_ByteInterface`.
#[test]
fn test_ring_buffer_byte_interface() {
    let mut rb = RingBuffer::new(2);

    // write one
    rb.write_byte(b'a');
    assert_eq!(rb.buffered(), 1);
    assert_eq!(rb.available(), 1);
    assert_eq!(rb.bytes(), vec![b'a']);
    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // write two, isFull
    rb.write_byte(b'b');
    assert_eq!(rb.buffered(), 2);
    assert_eq!(rb.available(), 0);
    assert_eq!(rb.bytes(), vec![b'a', b'b']);
    assert!(!rb.is_empty());
    assert!(rb.is_full());

    // write, it will scale from 2 to 4 bytes.
    rb.write_byte(b'c');
    assert_eq!(rb.buffered(), 3);
    assert_eq!(rb.available(), 1);
    assert_eq!(rb.bytes(), vec![b'a', b'b', b'c']);
    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // read one
    let b = rb.read_byte().unwrap_or(u8::MAX);
    assert_eq!(b, b'a');
    assert_eq!(rb.buffered(), 2);
    assert_eq!(rb.available(), 2);
    assert_eq!(rb.bytes(), vec![b'b', b'c']);
    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // read two
    let b = rb.read_byte().unwrap_or(u8::MAX);
    assert_eq!(b, b'b');
    assert_eq!(rb.buffered(), 1);
    assert_eq!(rb.available(), 3);
    assert!(!rb.is_empty());
    assert!(!rb.is_full());

    // read three
    let _ = rb.read_byte();
    assert_eq!(rb.buffered(), 0);
    assert_eq!(rb.available(), 4);
    assert!(rb.is_empty());
    assert!(!rb.is_full());
}

/// Go `TestRingBuffer_ReadFrom`.
#[test]
fn test_ring_buffer_read_from() {
    let mut rng = XorShift::new(0xDEADBEEFCAFEF00D);

    let mut rb = RingBuffer::new(0);
    let data_len = 4 * 1024;
    let data = rng.bytes(data_len);
    let mut r = Cursor::new(&data);
    let n = rb.read_from(&mut r).unwrap_or(u64::MAX);
    assert!(!rb.is_empty());
    assert_eq!(n, data_len as u64);
    assert_eq!(rb.buffered(), data_len);
    let (buf, tail) = rb.peek(-1);
    let mut all = buf.clone();
    all.extend_from_slice(&tail);
    assert_eq!(all, data);
    let mut buf = vec![0u8; data_len];
    let m = rb.read(&mut buf).unwrap_or(usize::MAX);
    assert_eq!(m, data_len);
    assert_eq!(buf, data);
    assert!(rb.is_empty());
    assert_eq!(rb.buffered(), 0);

    let mut rb = RingBuffer::new(0);
    let prefix_len = 2 * 1024;
    let prefix = rng.bytes(prefix_len);
    let data = rng.bytes(data_len);
    let mut r = Cursor::new(&data);
    let m = rb.write(&prefix);
    assert_eq!(m, prefix_len);
    let n = rb.read_from(&mut r).unwrap_or(u64::MAX);
    assert_eq!(n, data_len as u64);
    assert_eq!(rb.buffered(), prefix_len + data_len);
    let (head, tail) = rb.peek(prefix_len as isize);
    assert!(tail.is_empty());
    assert_eq!(head, prefix);
    let _ = rb.discard(prefix_len as isize);
    assert_eq!(rb.buffered(), data_len);
    let (head, tail) = rb.peek(-1);
    assert!(tail.is_empty());
    assert_eq!(head, data);
    let _ = rb.discard(data_len as isize);
    assert!(rb.is_empty());
    assert_eq!(rb.buffered(), 0);

    let init_len = 5 * 1024;
    let mut rb = RingBuffer::new(init_len);
    let prefix = rng.bytes(prefix_len);
    let data = rng.bytes(data_len);
    let mut r = Cursor::new(&data);
    let m = rb.write(&prefix);
    assert_eq!(m, prefix_len);
    let part_len = 1024;
    let (head, tail) = rb.peek(part_len as isize);
    assert!(tail.is_empty());
    assert_eq!(
        head,
        prefix
            .get(..part_len)
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
    );
    let _ = rb.discard(part_len as isize);
    let n = rb.read_from(&mut r).unwrap_or(u64::MAX);
    assert_eq!(n, data_len as u64);
    let (buf, tail) = rb.peek(-1);
    let mut all = buf.clone();
    all.extend_from_slice(&tail);
    let mut want = prefix
        .get(part_len..)
        .map(<[u8]>::to_vec)
        .unwrap_or_default();
    want.extend_from_slice(&data);
    assert_eq!(all, want);
    let _ = rb.discard((prefix_len + data_len - part_len) as isize);
    assert!(rb.is_empty());
    assert_eq!(rb.buffered(), 0);
}

/// Go `TestRingBuffer_WriteTo`.
#[test]
fn test_ring_buffer_write_to() {
    let mut rng = XorShift::new(0x0123456789ABCDEF);

    let mut rb = RingBuffer::new(5 * 1024);
    let data_len = 4 * 1024;
    let data = rng.bytes(data_len);
    let n = rb.write(&data);
    assert_eq!(n, data_len);
    let mut buf: Vec<u8> = Vec::new();
    let m = rb.write_to(&mut buf).unwrap_or(u64::MAX);
    assert!(rb.is_empty());
    assert_eq!(m, data_len as u64);
    assert_eq!(buf, data);

    buf.clear();
    let data = rng.bytes(data_len);
    let mut rb = RingBuffer::new(data_len);
    let n = rb.write(&data);
    assert_eq!(n, data_len);
    assert!(rb.is_full(), "ringbuffer should be full");
    let m = rb.write_to(&mut buf).unwrap_or(u64::MAX);
    assert!(rb.is_empty());
    assert_eq!(m, data_len as u64);
    assert_eq!(buf, data);

    buf.clear();
    rb.reset();
    let data = rng.bytes(data_len);
    let n = rb.write(&data);
    assert_eq!(n, data_len);
    assert!(rb.is_full(), "ringbuffer should be full");
    let part_len = 1024;
    let (head, tail) = rb.peek(part_len as isize);
    assert!(tail.is_empty());
    assert_eq!(
        head,
        data.get(..part_len).map(<[u8]>::to_vec).unwrap_or_default()
    );
    let _ = rb.discard(part_len as isize);
    let part_data = rng.bytes(part_len / 2);
    let n = rb.write(&part_data);
    assert_eq!(n, part_len / 2);
    assert_eq!(rb.available(), part_len / 2);
    let m = rb.write_to(&mut buf).unwrap_or(u64::MAX);
    assert_eq!(m, (data_len - part_len / 2) as u64);
    let mut want = data.get(part_len..).map(<[u8]>::to_vec).unwrap_or_default();
    want.extend_from_slice(&part_data);
    assert_eq!(buf, want);
    assert!(rb.is_empty());
}
