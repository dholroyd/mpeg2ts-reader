//! Provides some metadata from the SPS/PPS within this AVC stream, and also some flags to indicate
//! usage of certain AVC stream features.

use super::DescriptorError;
use std::fmt;

/// Descriptor holding copies of properties from the AVC metadata such as AVC 'profile' and 'level'.
pub struct AvcVideoDescriptor<'buf> {
    buf: &'buf [u8],
}
impl<'buf> AvcVideoDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as an `Iso639LanguageDescriptor`.
    pub const TAG: u8 = 40;
    /// Construct a `AvcVideoDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(tag: u8, buf: &'buf [u8]) -> Result<AvcVideoDescriptor<'buf>, DescriptorError> {
        assert_eq!(tag, Self::TAG);
        if buf.len() < 4 {
            Err(DescriptorError::BufferTooShort { buflen: buf.len() })
        } else {
            Ok(AvcVideoDescriptor { buf })
        }
    }

    /// The AVC _profile_ used in this stream will be equal to, or lower than, this value
    pub fn profile_idc(&self) -> u8 {
        self.buf[0]
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set0_flag(&self) -> bool {
        self.buf[1] & 0b10000000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set1_flag(&self) -> bool {
        self.buf[1] & 0b01000000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set2_flag(&self) -> bool {
        self.buf[1] & 0b00100000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set3_flag(&self) -> bool {
        self.buf[1] & 0b00010000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set4_flag(&self) -> bool {
        self.buf[1] & 0b00001000 != 0
    }
    /// Value of the same flag from this AVC stream's _Sequence Parameter Set_
    pub fn constraint_set5_flag(&self) -> bool {
        self.buf[1] & 0b00000100 != 0
    }
    /// Value of the same flags from this AVC stream's _Sequence Parameter Set_
    pub fn avc_compatible_flags(&self) -> u8 {
        self.buf[1] & 0b00000011
    }
    /// The AVC _level_ used in this stream will be equal to, or lower than, this value
    pub fn level_idc(&self) -> u8 {
        self.buf[2]
    }
    /// Stream may include AVC still pictures
    pub fn avc_still_present(&self) -> bool {
        self.buf[3] & 0b10000000 != 0
    }
    /// Stream may contain AVC 24-hour pictures
    pub fn avc_24_hour_picture_flag(&self) -> bool {
        self.buf[3] & 0b01000000 != 0
    }
    /// If false, _frame packing arrangement_ or _stereo video information_ SEI message should be
    /// present
    pub fn frame_packing_sei_not_present_flag(&self) -> bool {
        self.buf[3] & 0b00100000 != 0
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
