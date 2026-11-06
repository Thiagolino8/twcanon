use std::hash::Hasher;

// Only internally assigned usize IDs use this hasher. User-controlled strings
// retain randomized hashing; ordered containers preserve their insertion order.
#[derive(Default)]
pub(crate) struct IdHasher(u64);

impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = (self.0 ^ u64::from(*byte)).wrapping_mul(0x9e3779b97f4a7c15);
        }
    }

    fn write_usize(&mut self, value: usize) {
        self.0 = (value as u64).wrapping_mul(0x9e3779b97f4a7c15);
    }
}
