//! Types related to the _Transport Stream Description Table_

use crate::descriptor;
use crate::packet;
use std::fmt;

/// The identifier of TS Packets containing Transport Stream Description Table sections, with
/// value `0x0002`.
pub const TSDT_PID: packet::Pid = packet::Pid::new(0x0002);

/// Sections of the _Transport Stream Description Table_ carry descriptors applying to the entire
/// transport stream.
pub struct TsdtSection<'buf> {
    data: &'buf [u8],
}

impl<'buf> TsdtSection<'buf> {
    /// Create a `TsdtSection`, wrapping the given slice, whose methods can parse the section's
    /// fields
    pub fn new(data: &'buf [u8]) -> TsdtSection<'buf> {
        TsdtSection { data }
    }

    /// Returns an iterator over the descriptors in this TSDT section.
    pub fn descriptors<Desc: descriptor::Descriptor<'buf> + 'buf>(
        &self,
    ) -> impl Iterator<Item = Result<Desc, descriptor::DescriptorError>> + 'buf {
        descriptor::DescriptorIter::new(self.data)
    }
}

struct DescriptorsDebug<'buf>(&'buf TsdtSection<'buf>);
impl<'buf> fmt::Debug for DescriptorsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_list()
            .entries(self.0.descriptors::<descriptor::CoreDescriptors<'buf>>())
            .finish()
    }
}

impl<'buf> fmt::Debug for TsdtSection<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("TsdtSection")
            .field("descriptors", &DescriptorsDebug(self))
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use hex_literal::hex;

    #[test]
    fn debug_does_not_panic() {
        // A section with a single unknown descriptor (tag=0x80, length=2, payload=0x0102)
        let data = hex!("800201 02");
        let section = TsdtSection::new(&data);
        assert!(!format!("{:?}", section).is_empty());
    }

    #[test]
    fn descriptor_iteration() {
        // Two descriptors: tag=0x80 len=1 payload=0xAA, tag=0x81 len=2 payload=0xBBCC
        let data = hex!("8001AA 8102BBCC");
        let section = TsdtSection::new(&data);
        let descs: Vec<_> = section
            .descriptors::<descriptor::CoreDescriptors<'_>>()
            .collect();
        assert_eq!(descs.len(), 2);
        assert!(descs[0].is_ok());
        assert!(descs[1].is_ok());
    }

    #[test]
    fn empty_section() {
        let data = [];
        let section = TsdtSection::new(&data);
        let descs: Vec<_> = section
            .descriptors::<descriptor::CoreDescriptors<'_>>()
            .collect();
        assert_eq!(descs.len(), 0);
    }
}
