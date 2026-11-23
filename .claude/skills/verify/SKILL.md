---
name: verify
description: "Launch, drive and verify the gnet-core Rust port end-to-end: cargo build, cargo test, the ring/elastic probe examples, clippy and fmt. Use after any change to crates/gnet-core or before merging a unit branch."
---

# Verify gnet-core

Project-local verification for `pangpang778/ainative-gnet-orca`. The crate is
`crates/gnet-core` (workspace root `Cargo.toml`). Nothing here touches the Go
baseline under `reference/gnet-go`; the Go/Rust differential harness lives in
`verification/probe/` and is run separately by Engineers.

## Launch

```bash
cargo build --workspace            # from repo root; ready when it exits 0
```

This is a library crate — there is no server to start. "Running" means the
unit/integration tests and the probe binaries execute.

## Doctor

```bash
cargo --version && rustc --version   # expect cargo 1.94.x / rustc 1.94.x (rust-version floor is 1.80)
cargo test --workspace --no-run      # compiles all test targets without running; exit 0 = toolchain OK
```

If `cargo test --workspace --no-run` fails, fix the build before driving anything.

## Drive

Run in this order from the repo root; stop at the first non-zero exit.

```bash
cargo test --workspace                       # unit + integration tests (ring_buffer, elastic_buffer)
cargo run --example ring_probe               # JSON-lines op trace of RingBuffer, exit 0
cargo run --example elastic_probe            # JSON-lines op trace of ElasticBuffer, exit 0
cargo clippy --workspace --all-targets -- -D warnings   # lint gate
cargo fmt --check                            # formatting gate
```

Capture output for evidence (see below). Expected current counts: 16 tests
pass, both probes emit ~19/~23 JSON lines respectively.

## Evidence

Write every command's stdout/stderr under `verification/evidence/verify-skill/`
(create it if missing). Keep the exact exit code of each command in the report.
Probe outputs are deterministic JSON-lines — diffing them against a previous
run (`git diff` on the evidence dir) is the cheapest regression check.

## Cleanup

The run starts no daemons and writes only `target/` and the evidence dir.
Cleanup is deleting nothing — evidence files stay. To reset fully:
`cargo clean` removes `target/` only; never delete `verification/evidence/`.

## Feature map

- [Ring buffer](features/ring-buffer.md) — fixed-capacity circular read buffer
- [Elastic buffer](features/elastic-buffer.md) — linked-list + ring hybrid write buffer
- [Math](features/math.md) — power-of-two helpers
- [Linked list](features/linked-list.md) — node buffer backing ElasticBuffer
- [Protocol](features/protocol.md) — probe op-trace JSON format (planned U5+)
- [Echo / conn / engine / eventloop](features/echo.md) — not yet migrated (U3–U6)

## Maintenance

Re-run Doctor plus the Drive commands above after any change to
`crates/gnet-core`. When a new unit lands (U3+), add a feature file under
`features/` and a Drive line if it ships an example or test target. Update the
test-count expectation in Drive when tests are added.
