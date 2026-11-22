//! Port of `github.com/panjf2000/gnet/v2/pkg/math` (`pkg/math/math.go`).
//!
//! # Deviation from Go
//!
//! `ceil_to_power_of_two` in Go panics with "argument is too large" when the
//! input's head bit is set and it exceeds that bit (i.e. the result would not
//! fit in a signed `int`). Go can express the panic; the workspace lints deny
//! `panic!`, and a library must not abort its caller. The Rust port therefore
//! returns [`None`] in that one case and keeps every other result identical.
//! `closest_power_of_two` propagates that [`None`] instead of panicking.

/// Reports whether `n` is a power of two.
///
/// Mirrors Go `IsPowerOfTwo`: `n > 0 && n&(n-1) == 0`.
pub fn is_power_of_two(n: i64) -> bool {
    n > 0 && (n & (n - 1)) == 0
}

/// Returns `n` if it is a power of two, otherwise the next-highest power of two.
///
/// Mirrors Go `CeilToPowerOfTwo`:
///
/// * `n <= 2` returns `2`.
/// * Go's "argument is too large" panic becomes `None` (see the module docs).
pub fn ceil_to_power_of_two(n: i64) -> Option<i64> {
    const BIT_SIZE: i64 = 64;
    const MAX_INT_HEAD_BIT: i64 = 1i64 << (BIT_SIZE - 2); // 1 << 62

    if (n & MAX_INT_HEAD_BIT) != 0 && n > MAX_INT_HEAD_BIT {
        return None;
    }

    if n <= 2 {
        return Some(2);
    }

    let bits = 64 - (n - 1).leading_zeros() as i64;
    let out = 1i64.checked_shl(bits as u32)?;
    Some(out)
}

/// Returns `n` if it is a power of two, otherwise the next-highest power of two
/// *below* it.
///
/// # Deviation from Go
///
/// Go's `FloorToPowerOfTwo` misses the `>>32` smear (it only smears 16 bits,
/// for 32-bit ints) and returns non-powers-of-two for `n >= 2^32` (e.g.
/// `FloorToPowerOfTwo(1<<32)` returns `4294967295`); this port smears the full
/// 64 bits and returns the true floor power of two. The ring buffer does not
/// use this function.
///
/// For `n <= 2` it returns `n` unchanged (unlike `ceil_to_power_of_two`,
/// which floors to 2).
pub fn floor_to_power_of_two(n: i64) -> i64 {
    if n <= 2 {
        return n;
    }

    let mut v = n as u64;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v |= v >> 32;

    (v - (v >> 1)) as i64
}

/// Returns `n` if it is a power of two, otherwise the closest power of two.
///
/// Mirrors Go `ClosestPowerOfTwo`: it compares the distance to the next power
/// of two against the distance to the previous one. Returns `None` only when
/// [`ceil_to_power_of_two`] overflows, where Go would panic.
pub fn closest_power_of_two(n: i64) -> Option<i64> {
    let next = ceil_to_power_of_two(n)?;
    let prev = next / 2;
    if (n - prev) < (next - n) {
        Some(prev)
    } else {
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_of_two_detection_matches_go() {
        // Go: IsPowerOfTwo(n) == n > 0 && n&(n-1) == 0
        for (n, want) in [
            (0, false),
            (1, true),
            (2, true),
            (3, false),
            (4, true),
            (7, false),
            (8, true),
            (1023, false),
            (1024, true),
            (-1, false),
            (-8, false),
        ] {
            assert_eq!(is_power_of_two(n), want, "n={n}");
        }
    }

    #[test]
    fn ceil_to_power_of_two_matches_go() {
        // Go: n<=2 -> 2; else 1 << bits.Len(uint(n-1))
        for (n, want) in [
            (-5, 2),
            (0, 2),
            (1, 2),
            (2, 2),
            (3, 4),
            (4, 4),
            (5, 8),
            (7, 8),
            (8, 8),
            (9, 16),
            (1023, 1024),
            (1024, 1024),
            (1025, 2048),
            (1 << 30, 1 << 30),
            ((1 << 30) + 1, 1 << 31),
        ] {
            assert_eq!(ceil_to_power_of_two(n), Some(want), "n={n}");
        }
    }

    #[test]
    fn ceil_to_power_of_two_returns_none_where_go_panics() {
        // Go: n&maxintHeadBit != 0 && n > maxintHeadBit -> panic("argument is too large")
        let head = 1i64 << 62;
        assert_eq!(ceil_to_power_of_two(head + 1), None);
        assert_eq!(ceil_to_power_of_two(i64::MAX), None);
        // Boundary: exactly the head bit still computes (n-1 has 62 bits).
        assert_eq!(ceil_to_power_of_two(head), Some(head));
    }

    #[test]
    fn floor_to_power_of_two_matches_go() {
        // Go: n<=2 -> n; else smear bits then n - (n>>1)
        for (n, want) in [
            (-5, -5),
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 4),
            (5, 4),
            (7, 4),
            (8, 8),
            (9, 8),
            (1023, 512),
            (1024, 1024),
            (1025, 1024),
            (1 << 30, 1 << 30),
            ((1 << 30) + 1, 1 << 30),
        ] {
            assert_eq!(floor_to_power_of_two(n), want, "n={n}");
        }
    }

    #[test]
    fn closest_power_of_two_matches_go() {
        // Go: next=CeilToPowerOfTwo(n); prev=next/2; if n-prev < next-n { next=prev } else next.
        // Verified against Go v2.10.0: ClosestPowerOfTwo(0)=1, (1)=1, (2)=2, (3)=4, (6)=8.
        for (n, want) in [
            (0, 1),
            (1, 1),
            (2, 2),
            (3, 4),
            (4, 4),
            (5, 4),
            (6, 8),
            (7, 8),
            (8, 8),
            (9, 8),
            (12, 16),
            (16, 16),
            (24, 32),
            (48, 64),
            (100, 128),
        ] {
            assert_eq!(closest_power_of_two(n), Some(want), "n={n}");
        }
    }
}
