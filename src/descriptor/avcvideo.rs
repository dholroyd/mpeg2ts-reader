//! Provides some metadata from the SPS/PPS within this AVC stream, and also some flags to indicate
//! usage of certain AVC stream features.

use super::DescriptorError;
use crate::descriptor::descriptor_len;
use std::fmt;

/// Descriptor holding copies of properties from the AVC metadata such as AVC 'profile' and 'level'.
pub struct AvcVideoDescriptor<'buf> {
    buf: &'buf [u8],
}
impl<'buf> AvcVideoDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as a `AvcVideoDescriptor`.
    pub const TAG: u8 = 40;
    /// Construct a `AvcVideoDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(tag: u8, buf: &'buf [u8]) -> Result<AvcVideoDescriptor<'buf>, DescriptorError> {
        assert_eq!(tag, Self::TAG);
        descriptor_len(buf, tag, 4)?;
        Ok(AvcVideoDescriptor { buf })
    }

    /// The AVC _profile_ used in this stream will be equal to, or lower than, this value
    pub fn profile_idc(&self) -> u8 {
        self.buf[0]
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set0_flag(&self) -> bool {
        self.buf[1] & 0b1000_0000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set1_flag(&self) -> bool {
        self.buf[1] & 0b0100_0000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set2_flag(&self) -> bool {
        self.buf[1] & 0b0010_0000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set3_flag(&self) -> bool {
        self.buf[1] & 0b0001_0000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set4_flag(&self) -> bool {
        self.buf[1] & 0b0000_1000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set5_flag(&self) -> bool {
        self.buf[1] & 0b0000_0100 != 0
    }
    /// Value of the same flags from this AVC stream's _Sequence Parameter Set_
    pub fn avc_compatible_flags(&self) -> u8 {
        self.buf[1] & 0b0000_0011
    }
    /// The AVC _level_ used in this stream will be equal to, or lower than, this value
    pub fn level_idc(&self) -> u8 {
        self.buf[2]
    }
    /// Stream may include AVC still pictures
    pub fn avc_still_present(&self) -> bool {
        self.buf[3] & 0b1000_0000 != 0
    }
    /// Stream may contain AVC 24-hour pictures
    pub fn avc_24_hour_picture_flag(&self) -> bool {
        self.buf[3] & 0b0100_0000 != 0
    }
    /// If false, _frame packing arrangement_ or _stereo video information_ SEI message should be
    /// present
    pub fn frame_packing_sei_not_present_flag(&self) -> bool {
        self.buf[3] & 0b0010_0000 != 0
    }
}

impl fmt::Debug for AvcVideoDescriptor<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("AvcVideoDescriptor")
            .field("profile_idc", &self.profile_idc())
            .field("constraint_set0_flag", &self.constraint_set0_flag())
            .field("constraint_set1_flag", &self.constraint_set1_flag())
            .field("constraint_set2_flag", &self.constraint_set2_flag())
            .field("constraint_set3_flag", &self.constraint_set3_flag())
            .field("constraint_set4_flag", &self.constraint_set4_flag())
            .field("constraint_set5_flag", &self.constraint_set5_flag())
            .field("avc_compatible_flags", &self.avc_compatible_flags())
            .field("level_idc", &self.level_idc())
            .field("avc_still_present", &self.avc_still_present())
            .field("avc_24_hour_picture_flag", &self.avc_24_hour_picture_flag())
            .field(
                "frame_packing_sei_not_present_flag",
                &self.frame_packing_sei_not_present_flag(),
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
        let data = hex!("280442c01e3f");
        let desc = CoreDescriptors::from_bytes(&data[..]).unwrap();
        assert_matches!(desc, CoreDescriptors::AvcVideo(avc_video) => {
            assert_eq!(avc_video.level_idc(), 30);
            assert_eq!(avc_video.constraint_set0_flag(), true);
            assert_eq!(avc_video.constraint_set1_flag(), true);
            assert_eq!(avc_video.constraint_set3_flag(), false);
            assert_eq!(avc_video.constraint_set4_flag(), false);
            assert_eq!(avc_video.constraint_set5_flag(), false);
            assert_eq!(avc_video.avc_compatible_flags(), 0);
            assert_eq!(avc_video.profile_idc(), 66);
            assert_eq!(avc_video.avc_still_present(), false);
            assert_eq!(avc_video.avc_24_hour_picture_flag(), false);
            assert_eq!(avc_video.frame_packing_sei_not_present_flag(), true);
        })
    }

    #[test]
    fn debug() {
        let data = hex!("280442c01e3f");
        let desc = CoreDescriptors::from_bytes(&data[..]).unwrap();
        assert_matches!(desc, CoreDescriptors::AvcVideo(avc_video) => {
            assert!(!format!("{:?}", avc_video).is_empty());
        });
    }
}
