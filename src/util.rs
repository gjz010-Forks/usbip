/// Check validity of a USB descriptor
pub fn verify_descriptor(desc: &[u8]) {
    let mut offset = 0;
    while offset < desc.len() {
        offset += desc[offset] as usize; // length
    }
    assert_eq!(offset, desc.len());
}

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
pub(crate) mod tests;
