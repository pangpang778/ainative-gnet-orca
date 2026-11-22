//! Linked list of byte slices — port of gnet v2.10.0 `pkg/buffer/linkedlist`.
//!
//! Go uses `bsPool` (`pkg/pool/byteslice`) for allocations; Rust uses plain
//! `Vec<u8>` (pooling is a perf detail, not observable behavior). `io.EOF` on
//! empty `Read` maps to [`LinkedListError::Eof`]; `io.ErrShortBuffer` on
//! oversized `Peek`/`PeekWithBytes` maps to [`LinkedListError::ShortBuffer`];
//! `io.ErrShortWrite` on partial `WriteTo` maps to
//! [`LinkedListError::ShortWrite`].

use std::io::{self, Read, Write};

/// Error type for linked-list buffer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedListError {
    /// `io.EOF` — read on empty buffer.
    Eof,
    /// `io.ErrShortBuffer` — peek requested more bytes than buffered.
    ShortBuffer,
    /// `io.ErrShortWrite` — writer accepted fewer bytes than offered.
    ShortWrite,
    /// Underlying I/O error kind.
    Io(io::ErrorKind),
}

impl std::fmt::Display for LinkedListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Eof => f.write_str("EOF"),
            Self::ShortBuffer => f.write_str("short buffer"),
            Self::ShortWrite => f.write_str("short write"),
            Self::Io(k) => write!(f, "I/O error: {k:?}"),
        }
    }
}

impl std::error::Error for LinkedListError {}

impl From<io::Error> for LinkedListError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.kind())
    }
}

/// One node in the list.
struct Node {
    buf: Vec<u8>,
    next: Option<Box<Node>>,
}

/// A linked list of byte-slice nodes, mirroring Go `linkedlist.Buffer`.
///
/// `size` counts nodes, `bytes` counts total payload bytes — same counters Go
/// tracks (`llb.size`, `llb.bytes`).
///
/// Implementation note: Go keeps `head`/`tail` pointers for O(1)
/// push/pop at both ends. Rust's `Option<Box<Node>>` chain gives the same
/// complexity for `push_front`/`pop`, but `push_back` on a singly-linked list
/// is O(n) — see the `push_back_node` method for the trade-off.
pub struct LinkedListBuffer {
    head: Option<Box<Node>>,
    size: usize,
    bytes: usize,
}

impl LinkedListBuffer {
    /// Creates an empty buffer.
    pub fn new() -> Self {
        Self {
            head: None,
            size: 0,
            bytes: 0,
        }
    }

    /// Reads up to `p.len()` bytes into `p`, popping nodes as needed.
    ///
    /// Empty buffer + non-empty `p` → [`LinkedListError::Eof`]. Empty `p` →
    /// `Ok(0)` (Go behavior).
    pub fn read(&mut self, p: &mut [u8]) -> Result<usize, LinkedListError> {
        if p.is_empty() {
            return Ok(0);
        }

        let mut n = 0usize;
        while n < p.len() {
            let Some(mut node) = self.pop_node() else {
                break;
            };
            let avail = node.buf.len();
            let want = p.len() - n;
            let m = avail.min(want);
            if let (Some(dst), Some(src)) = (p.get_mut(n..n + m), node.buf.get(..m)) {
                dst.copy_from_slice(src);
            }
            n += m;
            if m < avail {
                // Partial consume: requeue remainder at head.
                node.buf.drain(..m);
                self.push_front_node(node);
            }
            if n == p.len() {
                return Ok(n);
            }
        }
        if n == 0 {
            return Err(LinkedListError::Eof);
        }
        Ok(n)
    }

    /// Allocates a byte vector of length `n` (Go `AllocNode` → `bsPool.Get`).
    ///
    /// In Rust this is just `vec![0; n]`; pooling is not observable.
    pub fn alloc_node(&self, n: usize) -> Vec<u8> {
        vec![0; n]
    }

    /// Frees a node buffer (Go `FreeNode` → `bsPool.Put`). No-op in Rust.
    pub fn free_node(&self, _p: Vec<u8>) {}

    /// Appends `p` without copying (Go `Append`).
    pub fn append(&mut self, p: Vec<u8>) {
        if p.is_empty() {
            return;
        }
        self.push_back_node(Box::new(Node { buf: p, next: None }));
    }

    /// Removes and returns the head node's buffer, or `None` if empty.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        self.pop_node().map(|n| n.buf)
    }

    /// Pushes `p` to the front, copying (Go `PushFront`).
    pub fn push_front(&mut self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        self.push_front_node(Box::new(Node {
            buf: p.to_vec(),
            next: None,
        }));
    }

    /// Pushes `p` to the back, copying (Go `PushBack`).
    pub fn push_back(&mut self, p: &[u8]) {
        if p.is_empty() {
            return;
        }
        self.push_back_node(Box::new(Node {
            buf: p.to_vec(),
            next: None,
        }));
    }

    /// Returns up to `max_bytes` of buffered data as owned slices.
    ///
    /// Does not remove nodes; [`discard`](Self::discard) consumes.
    /// `max_bytes <= 0` or `== isize::MAX` → all buffered bytes.
    /// `max_bytes > buffered()` → [`LinkedListError::ShortBuffer`].
    pub fn peek(&self, max_bytes: isize) -> Result<Vec<Vec<u8>>, LinkedListError> {
        let max_bytes = self.norm_peek(max_bytes)?;
        let mut out = Vec::new();
        let mut cum = 0usize;
        let mut cur = self.head.as_ref();
        while let Some(node) = cur {
            let mut offset = node.buf.len();
            if cum + offset > max_bytes {
                offset = max_bytes - cum;
            }
            out.push(
                node.buf
                    .get(..offset)
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default(),
            );
            cum += offset;
            if cum == max_bytes {
                break;
            }
            cur = node.next.as_ref();
        }
        Ok(out)
    }

    /// Like [`peek`](Self::peek) but prepends `bs` slices before list nodes.
    pub fn peek_with_bytes(
        &self,
        max_bytes: isize,
        bs: &[&[u8]],
    ) -> Result<Vec<Vec<u8>>, LinkedListError> {
        let max_bytes = self.norm_peek(max_bytes)?;
        let mut out = Vec::new();
        let mut cum = 0usize;
        for b in bs {
            if b.is_empty() {
                continue;
            }
            let mut offset = b.len();
            if cum + offset > max_bytes {
                offset = max_bytes - cum;
            }
            out.push(b.get(..offset).map(<[u8]>::to_vec).unwrap_or_default());
            cum += offset;
            if cum == max_bytes {
                return Ok(out);
            }
        }
        let mut cur = self.head.as_ref();
        while let Some(node) = cur {
            let mut offset = node.buf.len();
            if cum + offset > max_bytes {
                offset = max_bytes - cum;
            }
            out.push(
                node.buf
                    .get(..offset)
                    .map(<[u8]>::to_vec)
                    .unwrap_or_default(),
            );
            cum += offset;
            if cum == max_bytes {
                break;
            }
            cur = node.next.as_ref();
        }
        Ok(out)
    }

    fn norm_peek(&self, max_bytes: isize) -> Result<usize, LinkedListError> {
        if max_bytes <= 0 || max_bytes == isize::MAX {
            return Ok(usize::MAX);
        }
        let n = max_bytes as usize;
        if n > self.bytes {
            return Err(LinkedListError::ShortBuffer);
        }
        Ok(n)
    }

    /// Discards `n` bytes from the head, returning bytes actually discarded.
    pub fn discard(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        let mut remaining = n;
        let mut discarded = 0usize;
        while remaining != 0 {
            let Some(mut node) = self.pop_node() else {
                break;
            };
            let blen = node.buf.len();
            if remaining < blen {
                node.buf.drain(..remaining);
                discarded += remaining;
                self.push_front_node(node);
                break;
            }
            remaining -= blen;
            discarded += blen;
        }
        discarded
    }

    /// Reads from `r` until EOF, appending each chunk as a node.
    ///
    /// Mirrors Go `ReadFrom`: chunks of `MIN_READ` (512) bytes per iteration.
    /// A zero-byte read signals EOF (Rust convention; Go uses `io.EOF`).
    pub fn read_from<R: Read>(&mut self, r: &mut R) -> io::Result<u64> {
        const MIN_READ: usize = 512;
        let mut n: u64 = 0;
        loop {
            let mut buf = vec![0u8; MIN_READ];
            let m = r.read(&mut buf)?;
            if m == 0 {
                return Ok(n);
            }
            buf.truncate(m);
            n += m as u64;
            self.push_back_node(Box::new(Node { buf, next: None }));
        }
    }

    /// Writes all buffered bytes to `w`, consuming nodes.
    ///
    /// Partial write → [`LinkedListError::ShortWrite`] after re-queueing the
    /// unwritten remainder (Go `io.ErrShortWrite` semantics).
    pub fn write_to<W: Write>(&mut self, w: &mut W) -> Result<u64, LinkedListError> {
        let mut n: u64 = 0;
        while let Some(mut node) = self.pop_node() {
            let blen = node.buf.len();
            match w.write(&node.buf) {
                Ok(m) => {
                    n += m as u64;
                    if m < blen {
                        node.buf.drain(..m);
                        self.push_front_node(node);
                        return Err(LinkedListError::ShortWrite);
                    }
                }
                Err(e) => {
                    self.push_front_node(node);
                    return Err(e.into());
                }
            }
        }
        Ok(n)
    }

    /// Returns the number of nodes in the list.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns whether the list has no nodes (`len() == 0`).
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
    }

    /// Returns the total buffered byte count.
    pub fn buffered(&self) -> usize {
        self.bytes
    }

    /// Removes all nodes.
    pub fn reset(&mut self) {
        self.head = None;
        self.size = 0;
        self.bytes = 0;
    }

    // ---- internal helpers ----

    fn pop_node(&mut self) -> Option<Box<Node>> {
        let mut head = self.head.take()?;
        self.head = head.next.take();
        self.size -= 1;
        self.bytes -= head.buf.len();
        Some(head)
    }

    fn push_front_node(&mut self, mut boxed: Box<Node>) {
        boxed.next = self.head.take();
        let len = boxed.buf.len();
        self.head = Some(boxed);
        self.size += 1;
        self.bytes += len;
    }

    fn push_back_node(&mut self, mut boxed: Box<Node>) {
        boxed.next = None;
        let len = boxed.buf.len();
        // O(1) push_back requires reaching the current tail. Go does this with
        // a raw `*node` tail pointer. Rust's safe equivalent that keeps the
        // same big-O is a `Vec`-backed deque of nodes — but we already have a
        // linked structure for `peek` ordering. The simplest correct safe
        // approach: walk to the tail. For buffer workloads (push_back per
        // network write) this is O(n) per push — a real regression vs Go.
        //
        // Compromise: keep the singly-linked chain for head-pop ordering and
        // accept O(n) push_back. Buffers are drained FIFO; typical list depth
        // is small (spill chunks). Documented deviation; upgrade path is a
        // VecDeque<Node> rewrite if profiling shows it matters.
        if self.head.is_none() {
            self.head = Some(boxed);
        } else {
            // Find tail by walking; tail pointer not needed.
            let mut cur = self.head.as_mut();
            while let Some(node) = cur {
                if node.next.is_none() {
                    node.next = Some(boxed);
                    break;
                }
                cur = node.next.as_mut();
            }
        }
        self.size += 1;
        self.bytes += len;
    }
}

impl Default for LinkedListBuffer {
    fn default() -> Self {
        Self::new()
    }
}
