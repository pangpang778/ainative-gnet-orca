# U2 spec — elastic buffer (outbound) + linkedlist buffer

Baseline: gnet **v2.10.0** @ 58b59a11350707153f8c48369693ab9beba7b2fa (pinned; `reference/BASELINE.txt`).
Go source: `gnet-go/pkg/buffer/elastic/{elastic_ring_buffer,elastic_ring_list_buffer,elastic_buffer_test}.go`,
`gnet-go/pkg/buffer/linkedlist/{linked_list_buffer,llbuffer_test}.go`,
`gnet-go/pkg/pool/{byteslice,ringbuffer}/`.
Methodology: AINATIVE.md — port the behavior, not the structure; tests first; six gates.

## Scope

Two modules in `crates/gnet-core`:

| Rust module | Go source | Role |
|---|---|---|
| `src/linkedlist.rs` | `pkg/buffer/linkedlist/linked_list_buffer.go` (317 LOC) | Linked list of byte-slice nodes (head/tail, size, bytes counters). |
| `src/elastic.rs` | `pkg/buffer/elastic/elastic_ring_buffer.go` (205) + `elastic_ring_list_buffer.go` (176) | `ElasticRingBuffer` (lazy ring wrapper) + `ElasticBuffer` (ring+list hybrid). |

`byteslice`/`ringbuffer` sync.Pool dependencies become `Vec<u8>` / reuse of U1 `RingBuffer` — no custom pool code (deviation, same as U1).

## Public API surface (Rust)

### `linkedlist::LinkedListBuffer`

`new()`, `read(&mut self, p: &mut [u8]) -> Result<usize, LinkedListError>`, `alloc_node(n) -> Vec<u8>`,
`free_node(Vec<u8>)`, `append(Vec<u8>)` (no copy), `pop() -> Option<Vec<u8>>`,
`push_front(&[u8])`, `push_back(&[u8])` (copies), `peek(max_bytes) -> Result<Vec<Vec<u8>>, _>`,
`peek_with_bytes(max_bytes, bs: &[&[u8]]) -> Result<Vec<Vec<u8>>, _>`, `discard(n) -> usize`,
`read_from<R: Read>(&mut self, r: &mut R) -> io::Result<u64>`, `write_to<W: Write>(&mut self, w: &mut W) -> io::Result<u64>`,
`len()`, `buffered()`, `is_empty()`, `reset()`.

Errors: `io::ErrShortBuffer`-equivalent → `PeekError::ShortBuffer` (or reuse `io::ErrorKind::UnexpectedEof`-style mapping — spec'd deviation: dedicated `PeekError` enum); `io.EOF` on empty read → `LinkedListError::Eof`; `io.ErrShortWrite` on partial write → `LinkedListError::ShortWrite`.

### `elastic::ElasticRingBuffer`

Lazy `Option<RingBuffer>` wrapper mirroring Go `elastic.RingBuffer`:
`peek(n) -> (Vec<u8>, Vec<u8>)`, `discard(n) -> Result<usize, RingError>`, `read`, `read_byte`,
`write(&[u8]) -> usize`, `write_byte`, `buffered`, `len`, `cap`, `available`,
`write_string(&str) -> usize`, `bytes`, `read_from`, `write_to`, `is_full`, `is_empty`, `reset`, `done`.

### `elastic::ElasticBuffer`

`new(max_static_bytes) -> Result<Self, ElasticError>` (negative → error, Go `ErrNegativeSize`),
`read`, `peek(n) -> Result<Vec<Vec<u8>>, _>`, `discard(n) -> usize`, `write(&[u8]) -> usize`,
`writev(&[&[u8]]) -> usize`, `read_from`, `write_to`, `buffered`, `is_empty`, `reset(max_static_bytes)`, `release`.

## Ported test list (from Go tests → Rust `tests/`)

From `elastic_buffer_test.go`:
1. `mixed_buffer_basic` — write 5KiB (>4KiB static), reset, rewrite, Peek/Discard, Writev, Read.
2. `mixed_buffer_read_from` — ReadFrom twice, listBuffer non-empty, Read/Peek/Discard.
3. `mixed_buffer_write_to` — Writev then WriteTo, verify bytes, IsEmpty.

From `llbuffer_test.go`:
4. `linkedlist_buffer_basic` — 100 PushBack, Peek, PeekWithBytes, Discard, Read.
5. `linkedlist_buffer_read_from` — ReadFrom, Reset, head+data, Read.
6. `linkedlist_buffer_write_to` — WriteTo full and after partial Discard.

Plus unit tests: comparator self-check for peek ordering, `new(0)`/`new(-1)` error, empty-buffer edge cases.

## Six gates

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace` — all green
4. `cargo doc --workspace --no-deps` — zero warnings
5. Comparator self-check — Go test scenario replayed in Rust asserts identical field values (buffered/len/cap/peek contents/discard counts)
6. Real differential — Go probe (`cmd/elasticprobe`) vs Rust probe (`examples/elastic_probe.rs`) run same deterministic scenario, emit JSON lines, `diff` must be empty (host + containers if docker available)

## Documented deviations (carried from U1 + new)

- `Peek` returns owned `Vec<Vec<u8>>` (Go returns `[][]byte` aliased slices).
- `byteslice.Pool`/`ringbuffer.Pool` sync.Pool → plain `Vec` allocation; pooling is a perf detail, not observable behavior in tests.
- `io.ReaderFrom`/`io.WriterTo` → `std::io::Read`/`Write` traits.
- `math.MaxInt32` sentinel in Peek → `isize::MAX`/`usize::MAX` equivalent handling preserved semantically (n<=0 or n==MAX → full buffer).
- Panics in Go (`ReadFrom` negative count, `WriteTo` invalid count) → `io::Error` returns (workspace lints deny panic).
