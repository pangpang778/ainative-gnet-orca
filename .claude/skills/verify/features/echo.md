# Echo server / connection / engine / eventloop

Placeholder for the socket-facing units still ahead in the migration plan.

## Status: not yet migrated

| Planned unit | gnet source | Rust target |
|--------------|-------------|-------------|
| U3 | `pkg/pool/*` (byteslice, goroutine pools) | `crates/gnet-core/src/pool*` |
| U4 | `load_balancer.go` | `crates/gnet-core/src/lb.rs` |
| U5 | `gnet.go`, `options.go` — EventHandler trait, Action enum | `crates/gnet-core/src/handler.rs` |
| U6 | `pkg/netpoll/`, `eventloop_unix.go`, `engine_unix.go`, `connection_unix.go` | `crates/gnet-core/src/net/` |

The canonical "echo" example (`examples/echo/` in gnet) depends on U5+U6 and
cannot run until the engine exists.

## How to get to it (today)

Nothing to drive — `cargo test --workspace` covers only migrated units.
Check `docs/audit/PLAN.md` for unit status and `docs/spec-u*.md` for contracts.

## Driving it (when it lands)

Expected recipe once U6 ships: `cargo run --example echo` starts a TCP echo
server; a `cargo run --example echo_client` (or `nc`) round-trips bytes.
Verification will add a port-binding smoke test under `tests/` and a
`verification/evidence/` capture of the session transcript.

## Gotchas

- Windows-only gnet files (`*_windows.go`) are intentionally skipped — target
  is the Linux epoll path; do not port them.
- `unsafe_code = "forbid"` at workspace level: epoll/netpoll bindings will need
  an `unsafe`-allowed sub-crate or FFI boundary decision — resolve in the U6
  spec before coding.
