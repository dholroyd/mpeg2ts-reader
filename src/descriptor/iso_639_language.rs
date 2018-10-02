use std::fmt;
use super::DescriptorError;
use encoding::all::ISO_8859_1;
use encoding::Encoding;
use encoding::types::DecoderTrap;
use std::borrow::Cow;

pub struct Iso639LanguageDescriptor<'buf> {
    pub buf: &'buf[u8],
}
impl<'buf> Iso639LanguageDescriptor<'buf> {
    pub const TAG: u8 = 10;
    pub fn new(_tag: u8, buf: &'buf[u8]) -> Result<Iso639LanguageDescriptor<'buf>, DescriptorError> {
        Ok(Iso639LanguageDescriptor { buf })
    }

    pub fn languages(&self) -> LanguageIterator {
        LanguageIterator::new(self.buf)
    }
}

pub struct LanguageIterator<'buf> {
    remaining_data: &'buf[u8],
}
impl<'buf> LanguageIterator<'buf> {
    pub fn new(data: &'buf[u8]) -> LanguageIterator<'buf> {
        LanguageIterator {
            remaining_data: data,
        }
    }
}
impl<'buf> Iterator for LanguageIterator<'buf> {
    type Item = Language<'buf>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_data.is_empty() {
            None
        } else {
            let (head, tail) = self.remaining_data.split_at(4);
            self.remaining_data = tail;
            let result = Some(Language::new(head));
            result
        }
    }
}

#[derive(Debug)]
enum AudioType {
    Undefined,
    CleanEffects,
    HearingImpaired,
    VisualImpairedCommentary,
    Reserved(u8),
}
impl From<u8> for AudioType {
    fn from(v: u8) -> Self {
        match v {
            0 => AudioType::Undefined,
            1 => AudioType::CleanEffects,
            2 => AudioType::HearingImpaired,
            3 => AudioType::VisualImpairedCommentary,
            _ => AudioType::Reserved(v)
        }
    }
}
pub struct Language<'buf> {
    buf: &'buf[u8],
}
impl<'buf> Language<'buf> {
    fn new(buf: &'buf[u8]) -> Language {
        assert_eq!(buf.len(), 4);
        Language {
            buf,
        }
    }
    fn code(&self, trap: DecoderTrap) -> Result<String, Cow<'static, str>> {
        ISO_8859_1.decode(&self.buf[0..3], trap)
    }
    fn audio_type(&self) -> AudioType {
        AudioType::from(self.buf[3])
    }
}
impl<'buf> fmt::Debug for Language<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_struct("Language")
            .field("code", &self.code(DecoderTrap::Replace).unwrap())
            .field("audio_type", &self.audio_type())
            .finish()
    }
}

struct LangsDebug<'buf>(&'buf Iso639LanguageDescriptor<'buf>);
impl<'buf> fmt::Debug for LangsDebug<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        f.debug_list().entries(self.0.languages()).finish()
    }
}
impl<'buf> fmt::Debug for Iso639LanguageDescriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(),fmt::Error> {
        f.debug_struct("Iso639LanguageDescriptor")
            .field("languages", &LangsDebug(self))
            .finish()
    }
}
