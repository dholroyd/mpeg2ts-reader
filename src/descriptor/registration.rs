//! Registration descriptor indicates which kind of syntax any 'private data' within the transport
//! stream will be following

use super::DescriptorError;
use std::fmt;

/// Indicates which kind of syntax any 'private data' within the transport stream will be following
pub struct RegistrationDescriptor<'buf> {
    /// the registration data bytes
    pub buf: &'buf [u8],
}
impl<'buf> RegistrationDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as a `RegistrationDescriptor`.
    pub const TAG: u8 = 5;
    /// Construct a `RegistrationDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(_tag: u8, buf: &'buf [u8]) -> Result<RegistrationDescriptor<'buf>, DescriptorError> {
        if buf.len() < 4 {
            Err(DescriptorError::NotEnoughData {
                tag: Self::TAG,
                actual: buf.len(),
                expected: 4,
            })
        } else {
            Ok(RegistrationDescriptor { buf })
        }
    }

    /// Format identifier value assigned by a _Registration Authority_.
    pub fn format_identifier(&self) -> u32 {
        u32::from(self.buf[0]) << 24
            | u32::from(self.buf[1]) << 16
            | u32::from(self.buf[2]) << 8
            | u32::from(self.buf[3])
    }

    /// borrows a slice of additional_identification_info bytes, whose meaning is defined by
    /// the identifier returned by `format_idenfifier()`.
    pub fn additional_identification_info(&self) -> &[u8] {
        &self.buf[4..]
    }
}
impl<'buf> fmt::Debug for RegistrationDescriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("RegistrationDescriptor")
            .field("format_identifier", &self.format_identifier())
            .field(
                "additional_identification_info",
                &format!("{:x?}", self.additional_identification_info()),
            )
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::super::{CoreDescriptors, Descriptor};
    use super::*;
    use data_encoding::hex;
    use matches::assert_matches;

    #[test]
    fn descriptor() {
        let data = hex::decode(b"050443554549").unwrap();
        let desc = CoreDescriptors::from_bytes(&data).unwrap();
        assert_matches!(
            desc,
            CoreDescriptors::Registration(RegistrationDescriptor { buf: b"CUEI" })
        );
    }
}
