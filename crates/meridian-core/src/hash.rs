//! FNV-1a 64-bit hasher over explicit little-endian bytes. Portable and
//! dependency-free. Used for the world state hash.

pub struct StateHasher(u64);

impl Default for StateHasher {
    fn default() -> Self {
        StateHasher(0xcbf2_9ce4_8422_2325)
    }
}

impl StateHasher {
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    pub fn write_u8(&mut self, v: u8) {
        self.write_bytes(&[v]);
    }
    pub fn write_u16(&mut self, v: u16) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn write_i64(&mut self, v: i64) {
        self.write_bytes(&v.to_le_bytes());
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}
