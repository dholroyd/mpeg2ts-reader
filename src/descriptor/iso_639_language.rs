//! Support for [ISO-639](https://en.wikipedia.org/wiki/ISO_639) language code metadata, and
//! audio-type metadata.

use super::DescriptorError;
use encoding::all::ISO_8859_1;
use encoding::types::DecoderTrap;
use encoding::Encoding;
use std::borrow::Cow;
use std::fmt;

/// Descriptor which may be attached to Transport Stream metadata to indicate the language of the
/// content.
pub struct Iso639LanguageDescriptor<'buf> {
    buf: &'buf [u8],
}
impl<'buf> Iso639LanguageDescriptor<'buf> {
    /// The descriptor tag value which identifies the descriptor as an `Iso639LanguageDescriptor`.
    pub const TAG: u8 = 10;
    /// Construct a `Iso639LanguageDescriptor` instance that will parse the data from the given
    /// slice.
    pub fn new(
        _tag: u8,
        buf: &'buf [u8],
    ) -> Result<Iso639LanguageDescriptor<'buf>, DescriptorError> {
        Ok(Iso639LanguageDescriptor { buf })
    }

    /// Produce an iterator over the `Language` items in the provided buffer.
    pub fn languages(&self) -> impl Iterator<Item = Result<Language<'buf>, LangError>> {
        LanguageIterator::new(self.buf)
    }
}

struct LanguageIterator<'buf> {
    remaining_data: &'buf [u8],
}
impl<'buf> LanguageIterator<'buf> {
    pub fn new(data: &'buf [u8]) -> LanguageIterator<'buf> {
        LanguageIterator {
            remaining_data: data,
        }
    }
}
impl<'buf> Iterator for LanguageIterator<'buf> {
    type Item = Result<Language<'buf>, LangError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_data.is_empty() {
            None
        } else if self.remaining_data.len() < 4 {
            let actual = self.remaining_data.len();
            self.remaining_data = &self.remaining_data[0..0];
            Some(Err(LangError::TooShort { actual }))
        } else {
            let (head, tail) = self.remaining_data.split_at(4);
            self.remaining_data = tail;
            Some(Ok(Language::new(head)))
        }
    }
}

/// An error reading a language code out of the descriptor
#[derive(Debug)]
pub enum LangError {
    /// there is not sufficient data in the descriptor to read out another complete language code
    TooShort {
        /// the actual buffer size in bytes (expected size is 4 bytes)
        actual: usize,
    },
}

/// Metadata about the role of the audio elementary stream to which this descriptor is attached.
#[derive(Debug, PartialEq, Eq)]
pub enum AudioType {
    /// The audio has no particular role define
    Undefined,
    /// There is no language-specific content within the audio
    CleanEffects,
    /// The audio is prepared for the heading impaired
    HearingImpaired,
    /// The audio is prepared for the visually impaired
    VisualImpairedCommentary,
    /// Values `0x80` to `0xFF` are reserved for use in future versions of _ISO/IEC 13818-1_
    Reserved(u8),
}
impl From<u8> for AudioType {
    fn from(v: u8) -> Self {
        match v {
            0 => AudioType::Undefined,
            1 => AudioType::CleanEffects,
            2 => AudioType::HearingImpaired,
            3 => AudioType::VisualImpairedCommentary,
            _ => AudioType::Reserved(v),
        }
    }
}
/// One of potentially many pieces of language metadata within an `Iso639LanguageDescriptor`.
pub struct Language<'buf> {
    buf: &'buf [u8],
}
impl<'buf> Language<'buf> {
    fn new(buf: &'buf [u8]) -> Language<'buf> {
        assert_eq!(buf.len(), 4);
        Language { buf }
    }
    /// Returns a string containing the ISO-639 language code of the elementary stream to which
    /// this descriptor is attached.
    pub fn code(&self, trap: DecoderTrap) -> Result<String, Cow<'static, str>> {
        ISO_8859_1.decode(&self.buf[0..3], trap)
    }
    /// Returns an `AudioType` variant indicating what role this audio track plays within the
    /// program.
    pub fn audio_type(&self) -> AudioType {
        AudioType::from(self.buf[3])
    }
}
impl<'buf> fmt::Debug for Language<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("Language")
            .field("code", &self.code(DecoderTrap::Replace).unwrap())
            .field("audio_type", &self.audio_type())
            .finish()
    }
}

struct LangsDebug<'buf>(&'buf Iso639LanguageDescriptor<'buf>);
impl<'buf> fmt::Debug for LangsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_list().entries(self.0.languages()).finish()
    }
}
impl<'buf> fmt::Debug for Iso639LanguageDescriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        f.debug_struct("Iso639LanguageDescriptor")
            .field("languages", &LangsDebug(self))
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::super::{CoreDescriptors, Descriptor};
    use super::*;
    use assert_matches::assert_matches;
    use hex_literal::*;

    #[test]
    fn descriptor() {
        let data = hex!("0a04656e6700");
        let desc = CoreDescriptors::from_bytes(&data).unwrap();
        assert_matches!(desc, CoreDescriptors::ISO639Language(iso_639_language) => {
            let mut langs = iso_639_language.languages();
            let first = langs.next().unwrap().unwrap();
            assert_eq!("eng", first.code(encoding::DecoderTrap::Strict).unwrap());
            assert_eq!(AudioType::Undefined, first.audio_type());
            assert_matches!(langs.next(), None);
        });
    }

    #[test]
    fn debug() {
        let data = hex!("0a04656e6700");
        let desc = CoreDescriptors::from_bytes(&data).unwrap();
        assert_matches!(desc, CoreDescriptors::ISO639Language(iso_639_language) => {
            assert!(!format!("{:?}", iso_639_language).is_empty());
        });
    }

    #[test]
    fn too_short() {
        let data = hex!("0a03656e67");
        let desc = CoreDescriptors::from_bytes(&data).unwrap();
        if let CoreDescriptors::ISO639Language(iso_639_language) = desc {
            let mut langs = iso_639_language.languages();
            langs.next();
        }
    }
}
