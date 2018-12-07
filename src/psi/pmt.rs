//! Types related to the _Program Map Table_

use crate::demultiplex::DemuxError;
use crate::descriptor;
use crate::packet;
use crate::StreamType;
use std::fmt;

/// Sections of the _Program Map Table_ give details of the streams within a particular program
pub struct PmtSection<'buf> {
    data: &'buf [u8],
}
impl<'buf> fmt::Debug for PmtSection<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("PmtSection")
            .field("pcr_pid", &self.pcr_pid())
            .field("descriptors", &DescriptorsDebug(self))
            .field("streams", &StreamsDebug(self))
            .finish()
    }
}
struct StreamsDebug<'buf>(&'buf PmtSection<'buf>);
impl<'buf> fmt::Debug for StreamsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_list().entries(self.0.streams()).finish()
    }
}
struct DescriptorsDebug<'buf>(&'buf PmtSection<'buf>);
impl<'buf> fmt::Debug for DescriptorsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_list()
            .entries(self.0.descriptors::<descriptor::CoreDescriptors<'buf>>())
            .finish()
    }
}

impl<'buf> PmtSection<'buf> {
    /// Create a `PmtSection`, wrapping the given slice, whose methods can parse the section's
    /// fields
    pub fn from_bytes(data: &'buf [u8]) -> Result<PmtSection<'buf>, DemuxError> {
        if data.len() < Self::HEADER_SIZE {
            Err(DemuxError::NotEnoughData {
                field: "program_map_section",
                expected: Self::HEADER_SIZE,
                actual: data.len(),
            })
        } else {
            Ok(PmtSection { data })
        }
    }

    const HEADER_SIZE: usize = 4;

    /// Returns the Pid of packets that will contain the Program Clock Reference for this program
    pub fn pcr_pid(&self) -> packet::Pid {
        packet::Pid::new(u16::from(self.data[0] & 0b0001_1111) << 8 | u16::from(self.data[1]))
    }
    fn program_info_length(&self) -> u16 {
        u16::from(self.data[2] & 0b0000_1111) << 8 | u16::from(self.data[3])
    }
    /// Returns an iterator over the descriptors attached to this PMT section.
    pub fn descriptors<Desc: descriptor::Descriptor<'buf> + 'buf>(
        &self,
    ) -> impl Iterator<Item = Result<Desc, descriptor::DescriptorError>> + 'buf {
        let descriptor_end = Self::HEADER_SIZE + self.program_info_length() as usize;
        let descriptor_data = &self.data[Self::HEADER_SIZE..descriptor_end];
        descriptor::DescriptorIter::new(descriptor_data)
    }
    /// Returns an iterator over the streams of which this program is composed
    pub fn streams(&self) -> impl Iterator<Item = StreamInfo<'buf>> {
        let descriptor_end = Self::HEADER_SIZE + self.program_info_length() as usize;
        if descriptor_end > self.data.len() {
            warn!(
                "program_info_length={} extends beyond end of PMT section (section_length={})",
                self.program_info_length(),
                self.data.len()
            );
            // return an iterator that will produce no items,
            StreamInfoIter::new(&self.data[0..0])
        } else {
            StreamInfoIter::new(&self.data[descriptor_end..])
        }
    }
}
/// Iterator over the `StreamInfo` entries in a `PmtSection`.
struct StreamInfoIter<'buf> {
    buf: &'buf [u8],
}
impl<'buf> StreamInfoIter<'buf> {
    fn new(buf: &'buf [u8]) -> StreamInfoIter<'buf> {
        StreamInfoIter { buf }
    }
}
impl<'buf> Iterator for StreamInfoIter<'buf> {
    type Item = StreamInfo<'buf>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        if let Some((stream_info, info_len)) = StreamInfo::from_bytes(self.buf) {
            self.buf = &self.buf[info_len..];
            Some(stream_info)
        } else {
            None
        }
    }
}

/// Details of a particular elementary stream within a program.
///
///  - `stream_type` gives an indication of the kind of content carried within the stream
///  - The `elementry_pid` property allows us to find Transport Stream packets that belong to the
///    elementry stream
///  - `descriptors` _may_ provide extra metadata describing some of the
///     stream's properties (for example, the streams 'language' might be given in a descriptor; or
///     it might not)
pub struct StreamInfo<'buf> {
    data: &'buf [u8],
}

impl<'buf> StreamInfo<'buf> {
    const HEADER_SIZE: usize = 5;

    fn from_bytes(data: &'buf [u8]) -> Option<(StreamInfo<'buf>, usize)> {
        if data.len() < Self::HEADER_SIZE {
            warn!(
                "only {} bytes remaining for stream info, at least {} required {:?}",
                data.len(),
                Self::HEADER_SIZE,
                data
            );
            return None;
        }
        let result = StreamInfo { data };

        let descriptor_end = Self::HEADER_SIZE + result.es_info_length() as usize;
        if descriptor_end > data.len() {
            warn!(
                "PMT section of size {} is not large enough to contain es_info_length of {}",
                data.len(),
                result.es_info_length()
            );
            return None;
        }
        Some((result, descriptor_end))
    }

    /// The type of this stream
    pub fn stream_type(&self) -> StreamType {
        self.data[0].into()
    }
    /// The Pid that will be used for TS packets containing the data of this stream
    pub fn elementary_pid(&self) -> packet::Pid {
        packet::Pid::new(u16::from(self.data[1] & 0b0001_1111) << 8 | u16::from(self.data[2]))
    }
    fn es_info_length(&self) -> u16 {
        u16::from(self.data[3] & 0b0000_1111) << 8 | u16::from(self.data[4])
    }

    /// Returns an iterator over the descriptors attached to this stream
    pub fn descriptors<Desc: descriptor::Descriptor<'buf> + 'buf>(
        &self,
    ) -> impl Iterator<Item = Result<Desc, descriptor::DescriptorError>> + 'buf {
        let descriptor_end = Self::HEADER_SIZE + self.es_info_length() as usize;
        descriptor::DescriptorIter::new(&self.data[Self::HEADER_SIZE..descriptor_end])
    }
}
impl<'buf> fmt::Debug for StreamInfo<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("StreamInfo")
            .field("stream_type", &self.stream_type())
            .field("elementary_pid", &self.elementary_pid())
            .field("descriptors", &StreamInfoDescriptorsDebug(self))
            .finish()
    }
}
struct StreamInfoDescriptorsDebug<'buf>(&'buf StreamInfo<'buf>);
impl<'buf> fmt::Debug for StreamInfoDescriptorsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_list()
            .entries(self.0.descriptors::<descriptor::CoreDescriptors<'buf>>())
            .finish()
    }
}
