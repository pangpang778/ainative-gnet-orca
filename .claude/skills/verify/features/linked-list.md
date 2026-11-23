# Linked list buffer

Chunked byte store backing `ElasticBuffer` — port of gnet `internal/bytes` /
linked-list buffer used by the elastic buffer tail.

## Sub-features

- `LinkedListBuffer::new` / `reset` / `is_empty` / `len` / `buffered`
- `append(Vec<u8>)` / `pop() -> Option<Vec<u8>>` / `push_front` / `push_back`
- `read` / `peek(max_bytes)` / `peek_with_bytes` / `discard(n)`
- `read_from` / `write_to` — `io` adapters over node chain
- `alloc_node` / `free_node` — pool hooks (currently identity; U3 replaces with pool)

## How to get to it

```rust
use gnet_core::LinkedListBuffer;
let mut l = LinkedListBuffer::new();
l.push_back(b"ab"); l.push_back(b"cd");
```

## Driving it

Indirect: `cargo test --workspace elastic_buffer` drives it through
`ElasticBuffer` spillover; `cargo run --example elastic_probe` exercises
multi-node peek/discard ordering.

## Gotchas

- `peek` returns `Vec<Vec<u8>>` — one entry per node, preserving chunk
  boundaries; do not assume a single contiguous slice.
- `discard(n)` counts bytes across node boundaries and frees fully-consumed
  nodes; return value is bytes actually discarded.
- `free_node` is a no-op today — real pooling arrives with U3
  (`pkg/pool/byteslice`); don't add a second allocator here.
- `LinkedListError` is the error type for read/peek/write_to paths.
