# Ring buffer

Fixed-capacity circular read buffer — port of gnet `pkg/buffer/ring`.

## Sub-features

- `RingBuffer::new(size)` — capacity is rounded up to a power of two, min `MIN_READ` (512)
- `write` / `write_byte` / `write_string` — append bytes, wrap-around safe
- `read` / `read_byte` / `read_from` / `write_to` — consume bytes, `io` adapters
- `peek(n)` / `discard(n)` — inspect and drop without copying out
- `buffered` / `len` / `cap` / `available` / `is_full` / `is_empty` / `reset`

## How to get to it

```rust
use gnet_core::RingBuffer;
let mut b = RingBuffer::new(64);
b.write(b"hello");
```

Or run the probe: `cargo run --example ring_probe`.

## Driving it

```bash
cargo test --workspace ring_buffer      # integration tests in tests/ring_buffer.rs
cargo run --example ring_probe          # deterministic JSON-lines op trace
```

The probe exercises new/write/discard/peek/read/read_from/write_to/reset and
prints one JSON object per op — compare `verification/evidence/verify-skill/ring_probe.out`
against the Go twin's output when differential checks are needed.

## Gotchas

- `peek` returns `(Vec<u8>, Vec<u8>)` — head and tail segments, because a peek
  can wrap the ring boundary. Concatenate both for the logical bytes.
- `discard(n)` on `isize` discards at most buffered bytes; return value is the
  actual count discarded.
- Capacity below `MIN_READ` is silently raised; `available()` reports free space
  accounting for the one-slot ring sentinel — `is_full` and `available()==0`
  coincide.
- `read_from` treats an empty-read-then-EOF source like Go's `io.EOF`; partial
  final reads still succeed.
