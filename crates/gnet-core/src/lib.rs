//! gnet-core: Rust port of panjf2000/gnet (Apache-2.0).
//!
//! Migrated units live here one at a time. See `docs/spec-u1-ring-buffer.md`
//! for the first unit's contract.

pub mod elastic;
pub mod linkedlist;
pub mod math;
pub mod ring;

pub use elastic::{ElasticBuffer, ElasticError, ElasticRingBuffer};
pub use linkedlist::{LinkedListBuffer, LinkedListError};
pub use math::{
    ceil_to_power_of_two, closest_power_of_two, floor_to_power_of_two, is_power_of_two,
};
pub use ring::{RingBuffer, RingError, MIN_READ};
