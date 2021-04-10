//! Describes the maximum bitrate of the stream to which this descriptor is attached, including
//! transport overheads.
//!
//! May be attached an an elementary stream, indicating the max bitrate of that elementary stream,
//! or to the program as a whole.  In both cases it appears in the PMT.

use super::descriptor_len;
use super::DescriptorError;
use std::fmt;

/// Describes the max bitrate of an elementary stream or a whole program.
pub struct MaximumBitrateDescriptor<'buf> {
    buf: &'buf [u8],
}
impl<'buf> MaximumBitrateDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as an `MaximumBitrateDescriptor`.
    pub const TAG: u8 = 14;
    /// Construct a `MaximumBitrateDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(
        tag: u8,
        buf: &'buf [u8],
    ) -> Result<MaximumBitrateDescriptor<'buf>, DescriptorError> {
        assert_eq!(tag, Self::TAG);
        descriptor_len(buf, tag, 3)?;
        Ok(MaximumBitrateDescriptor { buf })
    }

    /// The maximum bitrate expressed in units of 50 bytes per second
    pub fn maximum_bitrate(&self) -> u32 {
        u32::from(self.buf[0] & 0b0011_1111) << 16
            | u32::from(self.buf[1]) << 8
            | u32::from(self.buf[2])
    }

    /// Convenience method which converts the result of `maximum_bitrate()` into a bits-per-second
    /// value.
    pub fn maximum_bits_per_second(&self) -> u32 {
        self.maximum_bitrate() * 50 * 8
    }
}

impl fmt::Debug for MaximumBitrateDescriptor<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaximumBitrateDescriptor")
            .field("maximum_bitrate", &self.maximum_bitrate())
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::super::{CoreDescriptors, Descriptor};
    use assert_matches::assert_matches;
    use hex_literal::*;

    #[test]
    fn descriptor() {
        let data = hex!("0e03c00184");
        let desc = CoreDescriptors::from_bytes(&data[..]).unwrap();
        assert_matches!(desc, CoreDescriptors::MaximumBitrate(max_bitrate) => {
            assert_eq!(max_bitrate.maximum_bitrate(), 388);
            assert_eq!(max_bitrate.maximum_bits_per_second(), 155200);
            assert!(!format!("{:?}", max_bitrate).is_empty());
        });
    }
}
