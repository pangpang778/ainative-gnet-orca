# Math helpers

Power-of-two arithmetic ported from gnet `pkg/math` — used by ring/elastic
buffer sizing.

## Sub-features

- `is_power_of_two(n: i64) -> bool`
- `ceil_to_power_of_two(n: i64) -> Option<i64>` — `None` on overflow/invalid
- `floor_to_power_of_two(n: i64) -> i64`
- `closest_power_of_two(n: i64) -> Option<i64>`

## How to get to it

```rust
use gnet_core::math::ceil_to_power_of_two;
assert_eq!(ceil_to_power_of_two(100), Some(128));
```

## Driving it

Covered indirectly by `cargo test --workspace` (ring tests exercise
`RingBuffer::new` size rounding). There is no dedicated math test file yet —
edge cases (0, 1, negative, i64::MAX) are worth adding when U3 lands.

## Gotchas

- Inputs are `i64`, not `usize` — negative values return `None`/`0`, never panic.
- `ceil_to_power_of_two` can overflow near `i64::MAX`; always handle `Option`.
- `closest_power_of_two` rounds to nearest, ties behavior matches the Go
  `math/bits` implementation — check `reference/gnet-go/pkg/math` before
  assuming.
