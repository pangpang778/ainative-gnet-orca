# Elastic buffer

Outbound write buffer — port of gnet `pkg/buffer/elastic`. Hybrid: a fixed ring
head plus a `LinkedListBuffer` tail that grows on demand.

## Sub-features

- `ElasticBuffer::new(max_static_bytes)` — static ring cap, `Err` if negative
- `write` / `writev` / `read_from` — append bytes (scatter-gather for writev)
- `read` / `peek` / `discard` / `write_to` — consume from ring first, then list
- `buffered` / `is_empty` / `reset(max_static_bytes)` / `release`
- Introspection: `list_buffer_is_empty`, `ring_buffer_len`, `ring_buffer_buffered`, `ring_buffer_cap`
- `ElasticRingBuffer` — the pure ring variant (same op surface as `RingBuffer` but fallible `discard`)

## How to get to it

```rust
use gnet_core::ElasticBuffer;
let mut b = ElasticBuffer::new(1024).unwrap();
b.write(b"ping");
```

Or run the probe: `cargo run --example elastic_probe`.

## Driving it

```bash
cargo test --workspace elastic_buffer    # tests/elastic_buffer.rs
cargo run --example elastic_probe        # JSON-lines trace over ring+list paths
```

The probe writes past the static cap to force list spillover, then reads back
through both regions — diff `verification/evidence/verify-skill/elastic_probe.out`
against the Go twin for regression.

## Gotchas

- Ordering invariant: ring head is always drained before list tail; `peek`
  returns `Vec<Vec<u8>>` chunks in logical order, not a flat buffer.
- `discard` returns `Result<usize, RingError>` here (unlike `RingBuffer`'s
  plain `usize`) — propagate with `?` in library code.
- `reset` takes a new `max_static_bytes`; it does not reuse the old cap.
- `release()` frees the list nodes; the buffer stays usable afterwards.
- Workspace lints deny `unwrap`/`expect`/`panic`/`indexing` — tests use
  `Result`-returning helpers or `#[allow]`-free assertions only where the lint
  permits; don't add unwraps to satisfy a quick debug print.
