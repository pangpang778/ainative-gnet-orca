# Protocol / op-trace format

The verification probes (`examples/ring_probe.rs`, `examples/elastic_probe.rs`)
emit JSON-lines: one object per buffer op, keys sorted alphabetically, hex
payloads, `<nil>` escaping — byte-compatible with the Go twins in
`verification/probe/`.

## Sub-features

- Deterministic op sequence (xorshift64* seeded identically in Go and Rust)
- Per-op JSON: `op` field names the operation, plus state fields
  (`buffered`, `cap`, `available`, `is_full`, `is_empty`, `len`) and results
- Diff-based regression: Rust `.out` vs Go `.out` must be identical

## How to get to it

```bash
cargo run --example ring_probe     > out.rust.jsonl
# Go twin: cd verification/probe && go run . ringprobe > out.go.jsonl
diff out.rust.jsonl out.go.jsonl    # expect no output
```

## Driving it

`cargo run --example ring_probe` / `elastic_probe` (exit 0), then diff against
`verification/evidence/*_container.out` or a fresh Go run. Evidence copies live
in `verification/evidence/verify-skill/`.

## Gotchas

- This is a *probe protocol*, not the gnet wire protocol. The real gnet
  EventHandler/Action protocol is U5 (`gnet.go` API surface) — not yet ported.
- JSON key order matters for diffing: Go sorts map keys; the Rust probes emit
  keys pre-sorted. Adding a field requires updating both twins.
- Go probe needs the pinned submodule (`reference/gnet-go`); if absent, run
  `git submodule update --init` first.
