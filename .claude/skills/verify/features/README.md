# Feature map

One file per feature of `crates/gnet-core`, in the four-section format:
Sub-features / How to get to it / Driving it / Gotchas.

| Feature | Status | Primary code |
|---------|--------|--------------|
| [ring-buffer](ring-buffer.md) | migrated (U1) | `src/ring.rs`, `tests/ring_buffer.rs`, `examples/ring_probe.rs` |
| [elastic-buffer](elastic-buffer.md) | migrated (U2) | `src/elastic.rs`, `src/linkedlist.rs`, `tests/elastic_buffer.rs`, `examples/elastic_probe.rs` |
| [math](math.md) | migrated (U1) | `src/math.rs` |
| [linked-list](linked-list.md) | migrated (U2) | `src/linkedlist.rs` |
| [protocol](protocol.md) | planned (U5) | — |
| [echo](echo.md) | planned (U5–U6) | — |

"Planned" files describe the gnet baseline surface being ported (see
`docs/audit/PLAN.md` and `reference/BASELINE.txt`) so the map stays complete;
they carry no drive recipe yet.
