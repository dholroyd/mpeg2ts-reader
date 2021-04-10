//! Registration descriptor indicates which kind of syntax any 'private data' within the transport
//! stream will be following

use super::DescriptorError;
use crate::descriptor::descriptor_len;
use smptera_format_identifiers_rust::FormatIdentifier;
use std::fmt;

/// Indicates which kind of syntax any 'private data' within the transport stream will be following
pub struct RegistrationDescriptor<'buf> {
    /// the registration data bytes
    buf: &'buf [u8],
}
impl<'buf> RegistrationDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as a `RegistrationDescriptor`.
    pub const TAG: u8 = 5;
    /// Construct a `RegistrationDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(tag: u8, buf: &'buf [u8]) -> Result<RegistrationDescriptor<'buf>, DescriptorError> {
        descriptor_len(buf, tag, 4)?;
        Ok(RegistrationDescriptor { buf })
    }

    /// Format identifier value assigned by a _Registration Authority_.
    ///
    /// Note that the `FormatIdentifier` type defines numerous constants for identifiers registered
    /// with the SMPTE RA, which you can use in tests like so:
    ///
    /// ```rust
    /// # use mpeg2ts_reader::descriptor::registration::RegistrationDescriptor;
    /// use smptera_format_identifiers_rust::FormatIdentifier;
    /// # let descriptor = RegistrationDescriptor::new(RegistrationDescriptor::TAG, b"CUEI")
    /// #   .unwrap();
    /// if descriptor.format_identifier() == FormatIdentifier::CUEI {
    ///     // perform some interesting action
    /// }
    /// ```
    pub fn format_identifier(&self) -> FormatIdentifier {
        FormatIdentifier::from(&self.buf[0..4])
    }

    /// Returns true if the given identifier is equal to the value returned by `format_identifier()`
    pub fn is_format(&self, id: FormatIdentifier) -> bool {
        self.format_identifier() == id
    }

    /// borrows a slice of additional_identification_info bytes, whose meaning is defined by
    /// the identifier returned by `format_identifier()`.
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
    use assert_matches::assert_matches;
    use hex_literal::*;

    #[test]
    fn descriptor() {
        let data = hex!("050443554549");
        let desc = CoreDescriptors::from_bytes(&data[..]).unwrap();
        assert_matches!(desc, CoreDescriptors::Registration(reg) => {
            let expected = smptera_format_identifiers_rust::FormatIdentifier::from(&b"CUEI"[..]);
            assert_eq!(reg.format_identifier(), expected);
            assert!(reg.is_format(expected));
            assert!(!format!("{:?}", reg).is_empty())
        });
    }
}
