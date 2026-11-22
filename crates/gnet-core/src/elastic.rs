//! Elastic buffers — port of gnet v2.10.0 `pkg/buffer/elastic`.
//!
//! Two types mirror the Go package:
//!
//! * [`ElasticRingBuffer`] wraps [`crate::ring::RingBuffer`] lazily (Go keeps a
//!   `nil` until first write, then checks it back into a pool on `Done`/empty).
//!   Pooling is dropped — `None`/`Some` gives the same observable behavior.
//!
//! * [`ElasticBuffer`] is the hybrid used by `connection.outboundBuffer`:
//!   a fixed-size ring for the hot path plus a [`LinkedListBuffer`] spill
//!   for overflow. `Write`/`Writev` route to the list once the ring's
//!   buffered bytes reach `max_static_bytes` or the ring's capacity does.

use std::io::{self, Read, Write};

use crate::linkedlist::{LinkedListBuffer, LinkedListError};
use crate::ring::{RingBuffer, RingError};

/// Error for [`ElasticBuffer::new`] / [`ElasticBuffer::reset`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElasticError {
    /// Go `errorx.ErrNegativeSize` — `max_static_bytes <= 0`.
    NegativeSize,
}

impl std::fmt::Display for ElasticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("gnet: negative size is not allowed")
    }
}

impl std::error::Error for ElasticError {}

/// `math.MaxInt32` sentinel used by Go `Peek` — `n <= 0` or `n == MaxInt32`
/// means "all buffered bytes".
const MAX_PEEK: isize = i32::MAX as isize;

/// Elastic wrapper of [`RingBuffer`], mirroring Go `elastic.RingBuffer`.
///
/// The inner ring is `None` until first write; `done()` returns it to the
/// pool when empty (Go) — here it just stays allocated, since `Vec` has no
/// pool to return to. Observable behavior is identical.
pub struct ElasticRingBuffer {
    rb: Option<RingBuffer>,
}

impl ElasticRingBuffer {
    /// Creates an empty elastic ring buffer (Go zero value: `rb == nil`).
    pub fn new() -> Self {
        Self { rb: None }
    }

    fn instance(&mut self) -> &mut RingBuffer {
        self.rb.get_or_insert_with(|| RingBuffer::new(0))
    }

    /// Checks and "returns" the internal ring to the pool (Go `Done`).
    ///
    /// In Rust there is no pool; this drops the allocation, matching Go's
    /// observable post-state (`b.rb == nil`).
    pub fn done(&mut self) {
        self.rb = None;
    }

    /// Internal `done()`: only drops when the ring is empty (Go `done()`).
    fn done_if_empty(&mut self) {
        if self.rb.as_ref().is_some_and(|r| r.is_empty()) {
            self.rb = None;
        }
    }

    /// Returns the next `n` bytes without advancing the read pointer.
    /// `(head, tail)`; `tail` is empty unless data wraps. `n <= 0` → all.
    pub fn peek(&self, n: isize) -> (Vec<u8>, Vec<u8>) {
        match &self.rb {
            None => (Vec::new(), Vec::new()),
            Some(rb) => rb.peek(n),
        }
    }

    /// Skips the next `n` bytes. Empty buffer → [`RingError`].
    pub fn discard(&mut self, n: isize) -> Result<usize, RingError> {
        let Some(rb) = self.rb.as_mut() else {
            return Err(RingError);
        };
        let out = rb.discard(n);
        self.done_if_empty();
        Ok(out)
    }

    /// Reads up to `p.len()` bytes into `p`. Empty → [`RingError`].
    pub fn read(&mut self, p: &mut [u8]) -> Result<usize, RingError> {
        let Some(rb) = self.rb.as_mut() else {
            return Err(RingError);
        };
        let out = rb.read(p);
        self.done_if_empty();
        out
    }

    /// Reads and returns the next byte. Empty → [`RingError`].
    pub fn read_byte(&mut self) -> Result<u8, RingError> {
        let Some(rb) = self.rb.as_mut() else {
            return Err(RingError);
        };
        let out = rb.read_byte();
        self.done_if_empty();
        out
    }

    /// Writes `p`, growing as needed. Empty `p` → 0.
    pub fn write(&mut self, p: &[u8]) -> usize {
        if p.is_empty() {
            return 0;
        }
        self.instance().write(p)
    }

    /// Writes one byte.
    pub fn write_byte(&mut self, c: u8) {
        self.instance().write_byte(c);
    }

    /// Length of available bytes to read (0 when no inner buffer).
    pub fn buffered(&self) -> usize {
        self.rb.as_ref().map_or(0, |r| r.buffered())
    }

    /// Length of the underlying buffer (0 when none).
    pub fn len(&self) -> usize {
        self.rb.as_ref().map_or(0, |r| r.len())
    }

    /// Capacity of the underlying buffer (0 when none).
    pub fn cap(&self) -> usize {
        self.rb.as_ref().map_or(0, |r| r.cap())
    }

    /// Length of available bytes to write (0 when none).
    pub fn available(&self) -> usize {
        self.rb.as_ref().map_or(0, |r| r.available())
    }

    /// Writes the contents of `s`. Empty → 0.
    pub fn write_string(&mut self, s: &str) -> usize {
        if s.is_empty() {
            return 0;
        }
        self.instance().write_string(s)
    }

    /// All available read bytes as one vector (empty when none).
    pub fn bytes(&self) -> Vec<u8> {
        self.rb.as_ref().map_or_else(Vec::new, |r| r.bytes())
    }

    /// `io.ReaderFrom` — reads until EOF.
    pub fn read_from<R: Read>(&mut self, r: &mut R) -> io::Result<u64> {
        self.instance().read_from(r)
    }

    /// `io.WriterTo` — writes buffered bytes. Empty → `RingError`-equivalent
    /// `io::Error` (Go returns `ErrIsEmpty`).
    pub fn write_to<W: Write>(&mut self, w: &mut W) -> io::Result<u64> {
        let Some(rb) = self.rb.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "ring-buffer is empty",
            ));
        };
        let out = rb.write_to(w);
        self.done_if_empty();
        out
    }

    /// Whether the ring is full (false when no inner buffer).
    pub fn is_full(&self) -> bool {
        self.rb.as_ref().is_some_and(|r| r.is_full())
    }

    /// Whether the ring is empty (true when no inner buffer).
    pub fn is_empty(&self) -> bool {
        self.rb.as_ref().map_or(true, |r| r.is_empty())
    }

    /// Resets read/write pointers (no-op when no inner buffer).
    pub fn reset(&mut self) {
        if let Some(rb) = self.rb.as_mut() {
            rb.reset();
        }
    }
}

impl Default for ElasticRingBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Hybrid ring+list buffer — Go `elastic.Buffer`.
///
/// Ring holds up to `max_static_bytes` of hot data; overflow spills to the
/// linked list. Once the list is non-empty, all writes go to the list
/// (Go's `Write`/`Writev`/`ReadFrom` first branch).
pub struct ElasticBuffer {
    max_static_bytes: usize,
    ring_buffer: ElasticRingBuffer,
    list_buffer: LinkedListBuffer,
}

impl ElasticBuffer {
    /// Creates a buffer. `max_static_bytes <= 0` → [`ElasticError::NegativeSize`].
    pub fn new(max_static_bytes: isize) -> Result<Self, ElasticError> {
        if max_static_bytes <= 0 {
            return Err(ElasticError::NegativeSize);
        }
        Ok(Self {
            max_static_bytes: max_static_bytes as usize,
            ring_buffer: ElasticRingBuffer::new(),
            list_buffer: LinkedListBuffer::new(),
        })
    }

    /// Reads from ring first, then list.
    pub fn read(&mut self, p: &mut [u8]) -> Result<usize, LinkedListError> {
        // Go: n, err = ringBuffer.Read(p); if n == len(p) return.
        // Ring Read errors (ErrIsEmpty) are swallowed here — Go ignores err
        // on the partial path and only propagates list errors.
        let n = match self.ring_buffer.rb.as_mut() {
            Some(rb) => rb.read(p).unwrap_or(0),
            None => 0,
        };
        if n == p.len() {
            self.ring_buffer.done_if_empty();
            return Ok(n);
        }
        // Read the remainder directly into p[n..] — no temp copy needed.
        let m = if n < p.len() {
            match p.get_mut(n..) {
                Some(dst) => match self.list_buffer.read(dst) {
                    Ok(m) => m,
                    Err(LinkedListError::Eof) => 0,
                    Err(e) => return Err(e),
                },
                None => 0,
            }
        } else {
            0
        };
        self.ring_buffer.done_if_empty();
        Ok(n + m)
    }

    /// Returns up to `n` bytes as a list of slices (ring head/tail + list).
    ///
    /// `n <= 0` or `n == i32::MAX` → all buffered. `n > buffered()` →
    /// [`LinkedListError::ShortBuffer`].
    pub fn peek(&self, n: isize) -> Result<Vec<Vec<u8>>, LinkedListError> {
        let n = if n <= 0 || n == MAX_PEEK {
            usize::MAX
        } else {
            let nu = n as usize;
            if nu > self.buffered() {
                return Err(LinkedListError::ShortBuffer);
            }
            nu
        };
        let (head, tail) = self
            .ring_buffer
            .peek(if n == usize::MAX { -1 } else { n as isize });
        let ring_buffered = self.ring_buffer.buffered();
        if ring_buffered == n || (n == usize::MAX && ring_buffered == self.buffered()) {
            // All data is in ring.
            let mut out = Vec::new();
            if !head.is_empty() {
                out.push(head);
            }
            if !tail.is_empty() {
                out.push(tail);
            }
            return Ok(out);
        }
        // Mixed: ring provides head+tail, list provides the rest.
        let head_ref: &[u8] = &head;
        let tail_ref: &[u8] = &tail;
        self.list_buffer
            .peek_with_bytes(n as isize, &[head_ref, tail_ref])
    }

    /// Discards `n` bytes (ring first, then list). Returns count.
    pub fn discard(&mut self, n: usize) -> usize {
        let discarded = self.ring_buffer.discard(n as isize).unwrap_or(0);
        if n <= discarded {
            self.ring_buffer.done_if_empty();
            return discarded;
        }
        let m = self.list_buffer.discard(n - discarded);
        self.ring_buffer.done_if_empty();
        discarded + m
    }

    /// Appends `p`. Routes to list when list non-empty, ring is at static
    /// cap, or ring length >= max_static_bytes.
    pub fn write(&mut self, p: &[u8]) -> usize {
        let n = p.len();
        if n == 0 {
            return 0;
        }
        if !self.list_buffer.is_empty() || self.ring_buffer.buffered() >= self.max_static_bytes {
            self.list_buffer.push_back(p);
            return n;
        }
        if self.ring_buffer.len() >= self.max_static_bytes {
            let writable = self.ring_buffer.available();
            if n > writable {
                let (head, tail) = p.split_at(writable);
                self.ring_buffer.write(head);
                self.list_buffer.push_back(tail);
                return n;
            }
        }
        self.ring_buffer.write(p)
    }

    /// Appends multiple slices (Go `Writev`).
    pub fn writev(&mut self, bs: &[&[u8]]) -> usize {
        if !self.list_buffer.is_empty() || self.ring_buffer.buffered() >= self.max_static_bytes {
            let mut n = 0;
            for b in bs {
                self.list_buffer.push_back(b);
                n += b.len();
            }
            return n;
        }

        let mut writable = self.ring_buffer.available();
        if self.ring_buffer.len() < self.max_static_bytes {
            writable = self.max_static_bytes - self.ring_buffer.buffered();
        }
        let mut cum = 0usize;
        let mut pos = 0usize;
        for (i, b) in bs.iter().enumerate() {
            pos = i;
            cum += b.len();
            if b.len() > writable {
                let (head, tail) = b.split_at(writable);
                self.ring_buffer.write(head);
                self.list_buffer.push_back(tail);
                break;
            }
            let written = self.ring_buffer.write(b);
            writable = writable.saturating_sub(written);
        }
        for b in bs.iter().skip(pos + 1) {
            cum += b.len();
            self.list_buffer.push_back(b);
        }
        cum
    }

    /// `io.ReaderFrom` — ring if hot path, else list.
    pub fn read_from<R: Read>(&mut self, r: &mut R) -> io::Result<u64> {
        if !self.list_buffer.is_empty() || self.ring_buffer.buffered() >= self.max_static_bytes {
            return self.list_buffer.read_from(r);
        }
        self.ring_buffer.read_from(r)
    }

    /// `io.WriterTo` — ring first, then list.
    ///
    /// Go propagates `io.ErrShortWrite`/`ErrIsEmpty` as errors; Rust's
    /// `Write` trait has no `ErrShortWrite`, so a short write surfaces as
    /// `Ok(n)` with `n < buffered()` — caller checks `is_empty()`.
    /// [`LinkedListError`] converts to `io::Error` for the `?` operator.
    pub fn write_to<W: Write>(&mut self, w: &mut W) -> io::Result<u64> {
        let mut n = self.ring_buffer.write_to(w)?;
        match self.list_buffer.write_to(w) {
            Ok(m) => n += m,
            Err(LinkedListError::ShortWrite) => {
                // Go returns ErrShortWrite; Rust Write trait signals via Ok(n).
                // Convert to io::Error to preserve the error channel.
                return Err(io::Error::new(io::ErrorKind::WriteZero, "short write"));
            }
            Err(LinkedListError::Io(k)) => return Err(io::Error::new(k, "I/O error")),
            Err(_) => {}
        }
        Ok(n)
    }

    /// Total buffered bytes (ring + list).
    pub fn buffered(&self) -> usize {
        self.ring_buffer.buffered() + self.list_buffer.buffered()
    }

    /// Whether both ring and list are empty.
    pub fn is_empty(&self) -> bool {
        self.ring_buffer.is_empty() && self.list_buffer.is_empty()
    }

    /// Resets both buffers; `max_static_bytes > 0` updates the cap.
    pub fn reset(&mut self, max_static_bytes: isize) {
        self.ring_buffer.reset();
        self.list_buffer.reset();
        if max_static_bytes > 0 {
            self.max_static_bytes = max_static_bytes as usize;
        }
    }

    /// Whether the list buffer is empty (Go test asserts `!mb.listBuffer.IsEmpty()`).
    /// Exposed for tests; not part of the public Go API.
    #[doc(hidden)]
    pub fn list_buffer_is_empty(&self) -> bool {
        self.list_buffer.is_empty()
    }

    /// Ring buffer's current capacity (Go `mb.ringBuffer.Len()`).
    #[doc(hidden)]
    pub fn ring_buffer_len(&self) -> usize {
        self.ring_buffer.len()
    }

    /// Ring buffer's buffered byte count (Go `mb.ringBuffer.Buffered()`).
    #[doc(hidden)]
    pub fn ring_buffer_buffered(&self) -> usize {
        self.ring_buffer.buffered()
    }

    /// Ring buffer's capacity (Go `mb.ringBuffer.Cap()`).
    #[doc(hidden)]
    pub fn ring_buffer_cap(&self) -> usize {
        self.ring_buffer.cap()
    }

    /// Releases resources (Go `Release`): drops ring, clears list.
    pub fn release(&mut self) {
        self.ring_buffer.done();
        self.list_buffer.reset();
    }
}
