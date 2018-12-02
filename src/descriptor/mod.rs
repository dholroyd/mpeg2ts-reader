//! Descriptors provide metadata about an element of a Transport Stream.
//!
//! For example, a descriptor may be used to specify the language of an audio track.  Use of
//! specific descriptors is often not mandatory (many streams do not describe the language of their
//! audio).
//!
//! The syntax
//! of specific PSI tables often allows descriptors to be attached to the table itself, or to
//! entries within the table.
//!
//! # Extensions
//!
//! Descriptors are a point of extension, with a range of descriptor types defined by the core
//! standard, and further descriptor types defined by standards based upon transport streams.  In
//! order to support this extension, while avoiding allocations (a la `dyn Trait`),
//! descriptor-related types and methods within this crate have a type-parameter so that calling
//! code which wants to use externally-defined descriptors can supply a type which supports them.
//!
//! So for example code using [`PmtSection`](..//demultiplex/struct.PmtSection.html) will need to
//! specify the `Descriptor` implementation to be produced,
//!
//! ```
//! # use mpeg2ts_reader::psi::pmt::PmtSection;
//! # use mpeg2ts_reader::descriptor::CoreDescriptors;
//! # let data = [0; 4];
//! let pmt = PmtSection::from_bytes(&data).unwrap();
//! // type parameter to descriptors() is inferred from the use of CoreDescriptors below
//! for d in pmt.descriptors() {
//!     if let Ok(CoreDescriptors::Registration(reg)) = d {
//!         println!("registration_descriptor {:#x}", reg.format_identifier());
//!     }
//! }
//! ```

pub mod iso_639_language;
pub mod registration;

use self::iso_639_language::Iso639LanguageDescriptor;
use self::registration::RegistrationDescriptor;
use std::fmt;
use std::marker;

pub trait Descriptor<'buf>: Sized {
    fn from_bytes(buf: &'buf [u8]) -> Result<Self, DescriptorError>;
}

#[macro_export]
macro_rules! descriptor_enum {
    (
        $(#[$outer:meta])*
        $name:ident {
            $(
                $(#[$inner:ident $($args:tt)*])*
                $case_name:ident $($tags:pat)|* => $t:ident
            ),*,
        }
    ) => {
        $(#[$outer])*
        pub enum $name<'buf> {
            $(
                $(#[$inner $($args)*])*
                $case_name($t<'buf>),
            )*
        }
        impl<'buf> $crate::descriptor::Descriptor<'buf> for $name<'buf> {
            fn from_bytes(buf: &'buf[u8]) -> Result<Self, $crate::descriptor::DescriptorError> {
                if buf.len() <  2 {
                    return Err($crate::descriptor::DescriptorError::BufferTooShort{ buflen: buf.len() })
                }
                let tag = buf[0];
                let len = buf[1] as usize;
                let tag_end = len + 2;
                if tag_end > buf.len() {
                    return Err($crate::descriptor::DescriptorError::TagTooLongForBuffer{ taglen: len, buflen: buf.len() })
                }
                let payload = &buf[2..tag_end];
                match tag {
                    $( $( $tags )|* => Ok($name::$case_name($t::new(tag, payload)?)), )*
                    _ => Err($crate::descriptor::DescriptorError::UnhandledTagValue(tag)),
                }
            }
        }
    }
}

pub struct UnknownDescriptor<'buf> {
    pub tag: u8,
    pub payload: &'buf [u8],
}
impl<'buf> UnknownDescriptor<'buf> {
    pub fn new(tag: u8, payload: &'buf [u8]) -> Result<UnknownDescriptor<'buf>, DescriptorError> {
        Ok(UnknownDescriptor { tag, payload })
    }
}
impl<'buf> fmt::Debug for UnknownDescriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("UnknownDescriptor")
            .field("tag", &self.tag)
            .field("len", &self.payload.len())
            .finish()
    }
}

descriptor_enum!{
    #[derive(Debug)]
    CoreDescriptors {
        Reserved 0|1|36...63 => UnknownDescriptor,
        VideoStream 2 => UnknownDescriptor,
        AudioStream 3 => UnknownDescriptor,
        Hierarchy 4 => UnknownDescriptor,
        Registration 5 => RegistrationDescriptor,
        DataStreamAlignment 6 => UnknownDescriptor,
        TargetBackgroundGrid 7 => UnknownDescriptor,
        VideoWindow 8 => UnknownDescriptor,
        CA 9 => UnknownDescriptor,
        ISO639Language 10 => Iso639LanguageDescriptor,
        SystemClock 11 => UnknownDescriptor,
        MultiplexBufferUtilization 12 => UnknownDescriptor,
        Copyright 13 => UnknownDescriptor,
        MaximumBitrate 14 => UnknownDescriptor,
        PrivateDataIndicator 15 => UnknownDescriptor,
        SmoothingBuffer 16 => UnknownDescriptor,
        STD 17 => UnknownDescriptor,
        IBP 18 => UnknownDescriptor,
        /// ISO IEC 13818-6
        IsoIec13818dash6 19...26 => UnknownDescriptor,
        MPEG4Video 27 => UnknownDescriptor,
        MPEG4Audio 28 => UnknownDescriptor,
        IOD 29 => UnknownDescriptor,
        SL 30 => UnknownDescriptor,
        FMC 31 => UnknownDescriptor,
        ExternalESID 32 => UnknownDescriptor,
        MuxCode 33 => UnknownDescriptor,
        FmxBufferSize 34 => UnknownDescriptor,
        MultiplexBuffer 35 => UnknownDescriptor,
        UserPrivate 64...255 => UnknownDescriptor,
    }
}

pub struct DescriptorIter<'buf, Desc>
where
    Desc: Descriptor<'buf>,
{
    buf: &'buf [u8],
    phantom: marker::PhantomData<Desc>,
}
impl<'buf, Desc> DescriptorIter<'buf, Desc>
where
    Desc: Descriptor<'buf>,
{
    pub fn new(buf: &'buf [u8]) -> DescriptorIter<'buf, Desc> {
        DescriptorIter {
            buf,
            phantom: marker::PhantomData,
        }
    }
}
impl<'buf, Desc> Iterator for DescriptorIter<'buf, Desc>
where
    Desc: Descriptor<'buf>,
{
    type Item = Result<Desc, DescriptorError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.is_empty() {
            return None;
        }
        let tag = self.buf[0];
        let len = self.buf[1] as usize;
        let remaining_size = self.buf.len() - 2;
        if len > remaining_size {
            // ensure anther call to next() will yield None,
            self.buf = &self.buf[0..0];
            Some(Err(DescriptorError::NotEnoughData {
                tag,
                actual: remaining_size,
                expected: len,
            }))
        } else {
            let (desc, rest) = self.buf.split_at(len + 2);
            self.buf = rest;
            Some(Descriptor::from_bytes(desc))
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum DescriptorError {
    NotEnoughData {
        tag: u8,
        actual: usize,
        expected: usize,
    },
    TagTooLongForBuffer {
        taglen: usize,
        buflen: usize,
    },
    BufferTooShort {
        buflen: usize,
    },
    UnhandledTagValue(u8),
}
