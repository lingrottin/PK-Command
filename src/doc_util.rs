#![doc(hidden)]

#[doc(hidden)]
/// Simulates the transportation layer
pub struct Transport();

impl Transport {
    #[doc(hidden)]
    pub fn recv(&self) -> Option<Vec<u8>> {
        Some(Vec::new())
    }
    #[doc(hidden)]
    pub fn send(&self, _vec: Vec<u8>) {
        // do nothing
    }
    #[doc(hidden)]
    pub fn new() -> Self {
        Self()
    }
}
