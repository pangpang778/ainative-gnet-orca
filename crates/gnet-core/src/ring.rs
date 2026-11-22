//! Port of `github.com/panjf2000/gnet/v2/pkg/buffer/ring` (`pkg/buffer/ring/ring_buffer.go`).
//!
//! A memory-efficient circular buffer implementing the `io.Reader`/`io.Writer`
//! shape. Behaviour is ported faithfully; structure is not: Go's
//! `pkg/pool/byteslice` allocation pool is replaced with a plain `Vec<u8>`
//! (the pool is an optimisation, not behaviour), and Go's slices are replaced
//! by owned `Vec<u8>` copies for `peek`/`bytes`.
//!
//! # Deviations from Go
//!
//! * Go returns a slice pair `(head, tail)` that aliases the buffer; Rust
//!   returns owned `Vec<u8>` pairs. Callers cannot observe the aliasing, and
//!   the contents and the head/tail split point are identical.
//! * Go's `Peek(n)` for `0 < n < buffered` can return a head/tail pair where
//!   `head` is short of `n` when the readable data wraps (it returns at most
//!   what is contiguous up to the wrap, then the tail holds the rest). Rust
//!   returns the same split, computed from the same formulas.
//! * `ReadFrom`/`WriteTo` return `std::io::Result<u64>` like Go's
//!   `io.ReaderFrom`/`io.WriterTo`. Go's `ErrShortWrite` from `WriteTo` ("the
//!   writer accepted fewer bytes than requested but did not error") is NOT
//!   reproduced: the Rust `WriteTo` returns `Ok(bytes_written)` and leaves the
//!   buffer state exactly as Go would, so the caller can observe
//!   `!is_empty()`. This is the one intentional behavioural difference; see
//!   [`RingBuffer::write_to`].
//! * Go panics if a reader returns a negative count; Rust's `Read` contract
//!   already forbids that (`ErrorKind::InvalidInput`), so the case is
//!   unreachable and no panic path exists.
//! * Go's `ReadFrom` calls `Read` even on a zero-length destination slice, and
//!   `bytes.Reader` answers that with `io.EOF` once exhausted — which ends the
//!   loop *before* the next `grow` check. Rust's `Read` returns `Ok(0)` for an
//!   empty buffer and cannot signal EOF there. `read_into` therefore treats
//!   `ErrorKind::UnexpectedEof` on an empty destination as EOF (the bridge a
//!   slice-backed reader uses to mimic `bytes.Reader`); a generic reader that
//!   returns `Ok(0)` on empty reads simply continues, which can perform one
//!   extra `grow` before the next non-empty read reports `Ok(0)` — same bytes
//!   read, possibly larger final capacity. See `read_into`.
//! * `grow` uses `usize` arithmetic with the same doubling / 1.25x rules as
//!   Go. Go's `n += n/4` loop cannot overflow for any `newCap` a caller can
//!   realistically request; the Rust port uses `saturating_add`/`saturating_mul`
//!   so a pathological `newCap` degrades to a huge allocation attempt instead of
//!   wrapping. See the `grow` method docs.

use std::io::{self, Read, Write};

/// `MinRead` is the minimum slice size passed to a `Read` call by
/// `RingBuffer::read_from`. As long as the buffer has at least `MIN_READ`
/// bytes beyond what is required to hold the contents of the reader,
/// `read_from` will not grow the underlying buffer.
pub const MIN_READ: usize = 512;

/// `DefaultBufferSize` is the first-time allocation on a ring buffer.
pub const DEFAULT_BUFFER_SIZE: usize = 1024;

/// `bufferGrowThreshold` is the size above which `grow` switches from doubling
/// to the 1.25x loop.
const BUFFER_GROW_THRESHOLD: usize = 4 * 1024;

/// Error returned when reading from an empty ring buffer.
///
/// Go returns `ErrIsEmpty` (`errors.New("ring-buffer is empty")`); Rust uses a
/// typed error so callers can match on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingError;

impl std::fmt::Display for RingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ring-buffer is empty")
    }
}

impl std::error::Error for RingError {}

/// Result of one `read_into` call inside [`RingBuffer::read_from`].
enum ReadOutcome {
    /// Read returned 0 bytes into a non-empty region: EOF.
    Eof,
    /// Read returned this many bytes (region may have been empty -> 0).
    Count(usize),
}

/// A circular buffer that implements the reader/writer shape.
///
/// The backing store is a `Vec<u8>` of length `size` (always a power of two,
/// or empty for a zero-sized buffer). `r` is the next position to read, `w`
/// the next position to write, and `is_empty` disambiguates the full/empty
/// state when `r == w`.
pub struct RingBuffer {
    buf: Vec<u8>,
    size: usize,
    r: usize,
    w: usize,
    is_empty: bool,
}

impl RingBuffer {
    /// Returns a new ring buffer whose backing store has the given size.
    ///
    /// `size == 0` produces an empty buffer (no allocation); the first `write`
    /// grows it to [`DEFAULT_BUFFER_SIZE`]. Any other size is rounded up to a
    /// power of two via [`crate::math::ceil_to_power_of_two`].
    pub fn new(size: usize) -> Self {
        if size == 0 {
            return Self {
                buf: Vec::new(),
                size: 0,
                r: 0,
                w: 0,
                is_empty: true,
            };
        }
        let size = crate::math::ceil_to_power_of_two(size as i64).unwrap_or(0) as usize;
        Self {
            buf: vec![0; size],
            size,
            r: 0,
            w: 0,
            is_empty: true,
        }
    }

    /// Returns the next `n` bytes without advancing the read pointer.
    ///
    /// Returns `(head, tail)`; `tail` is empty unless the readable data wraps
    /// around the end of the backing store. When `n <= 0`, returns ALL
    /// readable bytes (Go callers use `Peek(-1)` for the full contents).
    ///
    /// The returned vectors are copies; Go returns aliased subslices.
    pub fn peek(&self, n: isize) -> (Vec<u8>, Vec<u8>) {
        if self.is_empty {
            return (Vec::new(), Vec::new());
        }

        if n <= 0 {
            return self.peek_all();
        }
        let n = n as usize;

        if self.w > self.r {
            let mut m = self.w - self.r; // length of ring buffer
            if m > n {
                m = n;
            }
            let head = self.slice(self.r, m);
            return (head, Vec::new());
        }

        let mut m = self.size - self.r + self.w; // length of ring buffer
        if m > n {
            m = n;
        }

        if self.r + m <= self.size {
            let head = self.slice(self.r, m);
            (head, Vec::new())
        } else {
            let c1 = self.size - self.r;
            let head = self.slice(self.r, c1);
            let c2 = m - c1;
            let tail = self.slice(0, c2);
            (head, tail)
        }
    }

    /// Returns all bytes without advancing the read pointer.
    fn peek_all(&self) -> (Vec<u8>, Vec<u8>) {
        if self.is_empty {
            return (Vec::new(), Vec::new());
        }

        if self.w > self.r {
            return (self.slice(self.r, self.w - self.r), Vec::new());
        }

        let head = self.slice(self.r, self.size - self.r);
        let tail = if self.w != 0 {
            self.slice(0, self.w)
        } else {
            Vec::new()
        };
        (head, tail)
    }

    /// Copies `len` bytes starting at `off`, clamped to the backing store.
    ///
    /// Every call site computes `off`/`len` from the ring invariants
    /// (`off < size`, `off + len <= size`), so the clamp is defensive only.
    fn slice(&self, off: usize, len: usize) -> Vec<u8> {
        let start = off.min(self.buf.len());
        let end = start.saturating_add(len).min(self.buf.len());
        self.buf
            .get(start..end)
            .map(<[u8]>::to_vec)
            .unwrap_or_default()
    }

    /// Skips the next `n` bytes by advancing the read pointer.
    ///
    /// Returns the number of bytes actually discarded. `n <= 0` discards 0.
    /// When `n >= buffered()`, the buffer is reset and `buffered()` is
    /// returned (Go's `Discard` semantics).
    pub fn discard(&mut self, n: isize) -> usize {
        if n <= 0 {
            return 0;
        }
        let n = n as usize;

        let discarded = self.buffered();
        if n < discarded {
            self.r = (self.r + n) % self.size;
            return n;
        }
        self.reset();
        discarded
    }

    /// Reads up to `p.len()` bytes into `p`.
    ///
    /// Returns the number of bytes read. If the buffer is empty and `p` is
    /// non-empty, returns [`RingError`]. Mirrors Go `Read`.
    pub fn read(&mut self, p: &mut [u8]) -> Result<usize, RingError> {
        if p.is_empty() {
            return Ok(0);
        }

        if self.is_empty {
            return Err(RingError);
        }

        if self.w > self.r {
            let mut n = self.w - self.r;
            if n > p.len() {
                n = p.len();
            }
            if let Some(src) = self.buf.get(self.r..self.r + n) {
                if let Some(dst) = p.get_mut(..n) {
                    dst.copy_from_slice(src);
                }
            }
            self.r += n;
            if self.r == self.w {
                self.reset();
            }
            return Ok(n);
        }

        let mut n = self.size - self.r + self.w;
        if n > p.len() {
            n = p.len();
        }

        if self.r + n <= self.size {
            if let Some(src) = self.buf.get(self.r..self.r + n) {
                if let Some(dst) = p.get_mut(..n) {
                    dst.copy_from_slice(src);
                }
            }
        } else {
            let c1 = self.size - self.r;
            if let Some(src) = self.buf.get(self.r..self.size) {
                if let Some(dst) = p.get_mut(..c1) {
                    dst.copy_from_slice(src);
                }
            }
            let c2 = n - c1;
            if let Some(src) = self.buf.get(..c2) {
                if let Some(dst) = p.get_mut(c1..n) {
                    dst.copy_from_slice(src);
                }
            }
        }
        self.r = (self.r + n) % self.size;
        if self.r == self.w {
            self.reset();
        }

        Ok(n)
    }

    /// Reads and returns the next byte, or [`RingError`] if empty.
    pub fn read_byte(&mut self) -> Result<u8, RingError> {
        if self.is_empty {
            return Err(RingError);
        }
        let b = self.buf.get(self.r).copied().unwrap_or(0);
        self.r += 1;
        if self.r == self.size {
            self.r = 0;
        }
        if self.r == self.w {
            self.reset();
        }
        Ok(b)
    }

    /// Writes `p` into the buffer, growing it if needed.
    ///
    /// Returns the number of bytes written (always `p.len()`). Mirrors Go
    /// `Write`: if `p` is larger than the writable capacity, the buffer grows
    /// first.
    pub fn write(&mut self, p: &[u8]) -> usize {
        let n = p.len();
        if n == 0 {
            return 0;
        }

        let free = self.available();
        if n > free {
            self.grow(self.size + n - free);
        }

        if self.w >= self.r {
            let c1 = self.size - self.w;
            if c1 >= n {
                if let (Some(dst), Some(src)) = (self.buf.get_mut(self.w..self.w + n), p.get(..n)) {
                    dst.copy_from_slice(src);
                }
                self.w += n;
            } else {
                if let (Some(dst), Some(src)) = (self.buf.get_mut(self.w..self.size), p.get(..c1)) {
                    dst.copy_from_slice(src);
                }
                let c2 = n - c1;
                if let (Some(dst), Some(src)) = (self.buf.get_mut(..c2), p.get(c1..n)) {
                    dst.copy_from_slice(src);
                }
                self.w = c2;
            }
        } else {
            // w < r: the writable region is buf[w..r], which is contiguous.
            if let (Some(dst), Some(src)) = (self.buf.get_mut(self.w..self.w + n), p.get(..n)) {
                dst.copy_from_slice(src);
            }
            self.w += n;
        }

        if self.w == self.size {
            self.w = 0;
        }

        self.is_empty = false;

        n
    }

    /// Writes one byte, growing by `grow(1)` when no space is available.
    pub fn write_byte(&mut self, c: u8) {
        if self.available() < 1 {
            self.grow(1);
        }
        if let Some(slot) = self.buf.get_mut(self.w) {
            *slot = c;
        }
        self.w += 1;

        if self.w == self.size {
            self.w = 0;
        }
        self.is_empty = false;
    }

    /// Writes the contents of `s`, returning the number of bytes written.
    pub fn write_string(&mut self, s: &str) -> usize {
        self.write(s.as_bytes())
    }

    /// Returns the length of available bytes to read.
    pub fn buffered(&self) -> usize {
        if self.r == self.w {
            if self.is_empty {
                return 0;
            }
            return self.size;
        }

        if self.w > self.r {
            return self.w - self.r;
        }

        self.size - self.r + self.w
    }

    /// Returns the length of the underlying buffer (`buf.len()` in Go).
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns the size of the underlying buffer (the ring capacity).
    pub fn cap(&self) -> usize {
        self.size
    }

    /// Returns the length of available bytes to write.
    pub fn available(&self) -> usize {
        if self.r == self.w {
            if self.is_empty {
                return self.size;
            }
            return 0;
        }

        if self.w < self.r {
            return self.r - self.w;
        }

        self.size - self.w + self.r
    }

    /// Returns all available read bytes as a single vector.
    ///
    /// Does not move the read pointer; only copies the available data. Empty
    /// when the buffer is empty.
    pub fn bytes(&self) -> Vec<u8> {
        if self.is_empty {
            return Vec::new();
        }

        let (head, tail) = self.peek_all();
        let mut out = head;
        out.extend_from_slice(&tail);
        out
    }

    /// Implements `io.ReaderFrom`: reads from `r` until EOF or error.
    ///
    /// Mirrors Go `ReadFrom` exactly, including the two-read-per-iteration
    /// pattern (read into `buf[w..]`, then into `buf[..r]` when `w >= r`) and
    /// the `grow(buffered() + MIN_READ)` rule when `available() < MIN_READ`.
    ///
    /// Go loops until the reader returns `io.EOF`; a Rust reader signals the
    /// same condition with `Ok(0)` (e.g. `&[u8]`, `Cursor`), so a zero-byte
    /// read ends the loop. A reader that returns `Err` propagates it, matching
    /// Go's `err != nil` return.
    pub fn read_from<R: Read>(&mut self, r: &mut R) -> io::Result<u64> {
        let mut n: u64 = 0;
        loop {
            if self.available() < MIN_READ {
                self.grow(self.buffered() + MIN_READ);
            }

            // Each branch picks the writable region, reads once (or twice when
            // w >= r, matching Go's two-Read pattern), advances `w` and `n`,
            // and returns on EOF. A zero-byte read into a NON-EMPTY region is
            // the Rust EOF signal (Go's `io.EOF`); a zero-byte region is
            // skipped entirely because Go's `Read` on an empty slice returns
            // `(0, nil)` and the loop continues.
            if self.w >= self.r {
                // Go reads buf[w:] then buf[..r] as two separate Read calls.
                let m = match self.read_into(r, self.w..self.buf.len())? {
                    ReadOutcome::Eof => return Ok(n),
                    ReadOutcome::Count(m) => m,
                };
                self.is_empty = false;
                self.w = (self.w + m) % self.size;
                n += m as u64;

                let m = match self.read_into(r, 0..self.r)? {
                    ReadOutcome::Eof => return Ok(n),
                    ReadOutcome::Count(m) => m,
                };
                self.w = (self.w + m) % self.size;
                n += m as u64;
            } else {
                // w < r: the free region is buf[w..r].
                let m = match self.read_into(r, self.w..self.r)? {
                    ReadOutcome::Eof => return Ok(n),
                    ReadOutcome::Count(m) => m,
                };
                self.is_empty = false;
                self.w = (self.w + m) % self.size;
                n += m as u64;
            }
        }
    }

    /// Reads once into `buf[range]`.
    ///
    /// `range` is always a valid writable region by ring construction. An
    /// empty region still calls `read` (matching Go, which calls `Read` on the
    /// empty slice): most readers return `Ok(0)` and the loop continues, but a
    /// reader that signals end-of-input on a zero-length read — Go's
    /// `bytes.Reader` returns `io.EOF` there — can return
    /// `ErrorKind::UnexpectedEof`, which is treated as EOF. This is the one
    /// place the port needs a reader hint Go gets for free; see the module
    /// docs. A non-empty region that returns `Ok(0)` is EOF.
    fn read_into<R: Read>(
        &mut self,
        r: &mut R,
        range: std::ops::Range<usize>,
    ) -> io::Result<ReadOutcome> {
        match self.buf.get_mut(range) {
            Some(dst) => match r.read(dst) {
                Ok(0) if dst.is_empty() => {
                    // Zero-length read: Ok(0) is not EOF for generic readers.
                    Ok(ReadOutcome::Count(0))
                }
                Ok(0) => Ok(ReadOutcome::Eof),
                Ok(m) => Ok(ReadOutcome::Count(m)),
                Err(e) if dst.is_empty() && e.kind() == io::ErrorKind::UnexpectedEof => {
                    Ok(ReadOutcome::Eof)
                }
                Err(e) => Err(e),
            },
            None => Ok(ReadOutcome::Count(0)),
        }
    }

    /// Implements `io.WriterTo`: writes buffered bytes to `w`.
    ///
    /// Mirrors Go `WriteTo`'s head/tail split and state updates. **Deviation:**
    /// Go returns `io.ErrShortWrite` when the writer accepts fewer bytes than
    /// requested without erroring. Rust's `Write` trait has no such signal —
    /// `write` returning `Ok(n)` with `n < buf.len()` IS the short-write case,
    /// and callers are expected to handle it. This method returns
    /// `Ok(bytes_written)` and leaves the buffer in the same state Go would
    /// (read pointer advanced by what was written), so a short write is
    /// observable via `!is_empty()` rather than via the error value.
    pub fn write_to<W: Write>(&mut self, w: &mut W) -> io::Result<u64> {
        if self.is_empty {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "ring-buffer is empty",
            ));
        }

        if self.w > self.r {
            let n = self.w - self.r;
            let src = self.slice(self.r, n);
            let m = w.write(&src)?;
            self.r += m;
            if self.r == self.w {
                self.reset();
            }
            return Ok(m as u64);
        }

        let n = self.size - self.r + self.w;
        if self.r + n <= self.size {
            let src = self.slice(self.r, n);
            let m = w.write(&src)?;
            self.r = (self.r + m) % self.size;
            if self.r == self.w {
                self.reset();
            }
            return Ok(m as u64);
        }

        let mut cum: u64 = 0;
        let c1 = self.size - self.r;
        let head = self.slice(self.r, c1);
        let m = w.write(&head)?;
        self.r = (self.r + m) % self.size;
        if m < c1 {
            return Ok(m as u64);
        }
        cum += m as u64;
        let c2 = n - c1;
        let tail = self.slice(0, c2);
        let m = w.write(&tail)?;
        // Go: rb.r = m (the tail starts at buf[0], so m bytes written land at r=m).
        self.r = m;
        cum += m as u64;
        if self.r == self.w {
            self.reset();
        }
        Ok(cum)
    }

    /// Tells if this ring buffer is full.
    pub fn is_full(&self) -> bool {
        self.r == self.w && !self.is_empty
    }

    /// Tells if this ring buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.is_empty
    }

    /// Resets the read pointer and write pointer to zero.
    pub fn reset(&mut self) {
        self.is_empty = true;
        self.r = 0;
        self.w = 0;
    }

    /// Grows the buffer to at least `new_cap`, porting Go's `grow` exactly.
    ///
    /// Algorithm (Go `ring_buffer.go` lines 487-522):
    ///
    /// * `size == 0`: `new_cap <= DEFAULT_BUFFER_SIZE` -> `DEFAULT_BUFFER_SIZE`,
    ///   else `ceil_to_power_of_two(new_cap)`.
    /// * `size > 0` and `new_cap <= 2*size`: if `size < BUFFER_GROW_THRESHOLD`
    ///   double; otherwise grow by 1.25x steps (`n += n/4`) until `n >= new_cap`.
    /// * `size > 0` and `new_cap > 2*size`: keep `new_cap` as requested.
    ///
    /// The Go `bsPool.Get`/`Put` calls become `Vec::with_capacity` + copy; the
    /// readable bytes are drained through `read` into the new buffer exactly as
    /// Go does (`rb.Read(newBuf)`), preserving order and the `r=0, w=old_len`
    /// post-condition.
    fn grow(&mut self, new_cap: usize) {
        let n = self.size;
        let new_cap = if n == 0 {
            if new_cap <= DEFAULT_BUFFER_SIZE {
                DEFAULT_BUFFER_SIZE
            } else {
                crate::math::ceil_to_power_of_two(new_cap as i64)
                    .map(|v| v as usize)
                    .unwrap_or(new_cap)
            }
        } else {
            let double_cap = n.saturating_mul(2);
            if new_cap <= double_cap {
                if n < BUFFER_GROW_THRESHOLD {
                    double_cap
                } else {
                    // Check 0 < n to detect overflow and prevent an infinite loop.
                    let mut m = n;
                    while m > 0 && m < new_cap {
                        m = m.saturating_add(m / 4);
                    }
                    // The m calculation doesn't overflow, set m to new_cap.
                    if m > 0 {
                        m
                    } else {
                        new_cap
                    }
                }
            } else {
                new_cap
            }
        };

        let mut new_buf = vec![0u8; new_cap];
        let old_len = self.buffered();
        // Drain readable bytes into the new buffer. `read` cannot fail here:
        // new_buf is large enough (new_cap >= buffered by construction of the
        // grow rules) and self is non-empty only when old_len > 0. We copy
        // through `read` into a scratch slice of new_buf to keep the exact Go
        // ordering (head then tail) without indexing the vec directly.
        if old_len > 0 {
            // Take the readable bytes out via `bytes()` — a single ordered
            // copy — then write them into new_buf. This is equivalent to
            // Go's `rb.Read(newBuf)` because Read drains in order and
            // new_buf is >= old_len.
            let drained = self.bytes();
            if let Some(dst) = new_buf.get_mut(..old_len) {
                dst.copy_from_slice(&drained);
            }
            self.reset();
        }
        self.buf = new_buf;
        self.r = 0;
        self.w = old_len;
        self.size = new_cap;
        if self.w > 0 {
            self.is_empty = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Comparator self-check: peek/discard/bytes invariants on a wrapped buffer.
    #[test]
    fn wrapped_buffer_invariants_hold() {
        let mut rb = RingBuffer::new(64);
        let data: Vec<u8> = (0..64u8).collect();
        assert_eq!(rb.write(&data), 64);
        assert!(rb.is_full());

        // Discard 32 to force r=32, w=0 (wrapped read region).
        assert_eq!(rb.discard(32), 32);
        assert_eq!(rb.buffered(), 32);

        // Write 16 more — wraps around the end.
        let more: Vec<u8> = (100..116u8).collect();
        assert_eq!(rb.write(&more), 16);
        assert_eq!(rb.buffered(), 48);

        // peek(-1) must return all 48 readable bytes: head = buf[32..64] (32
        // bytes), tail = buf[0..16] (16 bytes).
        let (head, tail) = rb.peek(-1);
        assert_eq!(head.len(), 32);
        assert_eq!(tail.len(), 16);
        let want_head: Vec<u8> = data.iter().skip(32).copied().collect();
        assert_eq!(head, want_head);
        assert_eq!(tail, more);

        // bytes() == head ++ tail.
        let all = rb.bytes();
        assert_eq!(all.len(), 48);
        let mut want_all = want_head.clone();
        want_all.extend_from_slice(&more);
        assert_eq!(all, want_all);

        // peek(10) splits at the wrap: head has 10 bytes from r=32.
        let (h10, t10) = rb.peek(10);
        assert_eq!(h10.len(), 10);
        assert!(t10.is_empty());
        let want_h10: Vec<u8> = data.iter().skip(32).take(10).copied().collect();
        assert_eq!(h10, want_h10);

        // peek(40) crosses the wrap: head=32 bytes, tail=8 bytes.
        let (h40, t40) = rb.peek(40);
        assert_eq!(h40.len(), 32);
        assert_eq!(t40.len(), 8);

        // discard(48) empties the buffer.
        assert_eq!(rb.discard(48), 48);
        assert!(rb.is_empty());
        assert_eq!(rb.buffered(), 0);
        assert_eq!(rb.bytes().len(), 0);
    }
}
