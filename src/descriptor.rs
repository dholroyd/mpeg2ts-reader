use std::fmt;
use hex_slice::AsHex;

#[derive(Debug)]
pub enum Descriptor<'buf> {
    Reserved { tag: u8, payload: &'buf[u8]},
    VideoStream { payload: &'buf[u8]},
    AudioStream { payload: &'buf[u8]},
    Hierarchy { payload: &'buf[u8]},
    Registration { payload: &'buf[u8] },
    DataStreamAlignment { payload: &'buf[u8]},
    TargetBackgroundGrid { payload: &'buf[u8]},
    VideoWindow { payload: &'buf[u8]},
    CA { payload: &'buf[u8]},
    ISO639Language { payload: &'buf[u8]},
    SystemClock { payload: &'buf[u8]},
    MultiplexBufferUtilization { payload: &'buf[u8]},
    Copyright { payload: &'buf[u8]},
    MaximumBitrate { payload: &'buf[u8]},
    PrivateDataIndicator { payload: &'buf[u8]},
    SmoothingBuffer { payload: &'buf[u8]},
    STD { payload: &'buf[u8]},
    IBP { payload: &'buf[u8]},
    /// ISO IEC 13818-6
    IsoIec13818dash6 { tag: u8, payload: &'buf[u8]},
    MPEG4Video { payload: &'buf[u8]},
    MPEG4Audio { payload: &'buf[u8]},
    IOD { payload: &'buf[u8]},
    SL { payload: &'buf[u8]},
    FMC { payload: &'buf[u8]},
    ExternalESID { payload: &'buf[u8]},
    MuxCode { payload: &'buf[u8]},
    FmxBufferSize { payload: &'buf[u8]},
    MultiplexBuffer { payload: &'buf[u8]},
    UserPrivate { tag: u8, payload: &'buf[u8]},
}

impl<'buf> Descriptor<'buf> {
    pub fn new(buf: &'buf[u8]) -> Descriptor<'buf> {
        assert!(buf.len() >= 2);
        let tag = buf[0];
        let len = buf[1] as usize;
        let payload = &buf[2..2+len];
        match tag {
            0|1|36...63 => Descriptor::Reserved { tag, payload },
            2 => Descriptor::VideoStream { payload },
            3 => Descriptor::AudioStream { payload },
            4 => Descriptor::Hierarchy { payload },
            5 => Descriptor::Registration { payload },
            6 => Descriptor::DataStreamAlignment { payload },
            7 => Descriptor::TargetBackgroundGrid { payload },
            8 => Descriptor::VideoWindow { payload },
            9 => Descriptor::CA { payload },
            10 => Descriptor::ISO639Language { payload },
            11 => Descriptor::SystemClock { payload },
            12 => Descriptor::MultiplexBufferUtilization { payload },
            13 => Descriptor::Copyright { payload },
            14 => Descriptor::MaximumBitrate { payload },
            15 => Descriptor::PrivateDataIndicator { payload },
            16 => Descriptor::SmoothingBuffer { payload },
            17 => Descriptor::STD { payload },
            18 => Descriptor::IBP { payload },
            19...26 => Descriptor::IsoIec13818dash6 { tag, payload },
            27 => Descriptor::MPEG4Video { payload },
            28 => Descriptor::MPEG4Audio { payload },
            29 => Descriptor::IOD { payload },
            30 => Descriptor::SL { payload },
            31 => Descriptor::FMC { payload },
            32 => Descriptor::ExternalESID { payload },
            33 => Descriptor::MuxCode { payload },
            34 => Descriptor::FmxBufferSize { payload },
            35 => Descriptor::MultiplexBuffer { payload },
            64...255 => Descriptor::UserPrivate { tag, payload },
            _ => unimplemented!("tag {}", tag)  // TODO: why not exhaustive without this?
        }
    }
}

pub struct DescriptorIter<'buf> {
    buf: &'buf[u8]
}
impl<'buf> DescriptorIter<'buf> {
    pub fn new(buf: &'buf[u8]) -> DescriptorIter<'buf> {
        DescriptorIter { buf }
    }
}
impl<'buf> Iterator for DescriptorIter<'buf> {
    type Item = Result<Descriptor<'buf>, ()>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.buf.len() == 0 {
            return None;
        }
        let _tag = self.buf[0];
        let len = self.buf[1] as usize;
        if len > self.buf.len()-2 {
            // ensure anther call to next() will yield None,
            self.buf = &self.buf[0..0];
            Some(Err(()))
        } else {
            let (desc, rest) = self.buf.split_at(len+2);
            self.buf = rest;
            Some(Ok(Descriptor::new(desc)))
        }
    }
}

pub enum DescriptorError  {
    NotEnoughData { actual: usize, expected: usize }
}
pub struct RegistrationDescriptor<'buf> {
    buf: &'buf[u8],
}
impl<'buf> RegistrationDescriptor<'buf> {
    pub fn new(buf: &'buf[u8]) -> Result<RegistrationDescriptor<'buf>, DescriptorError> {
        if buf.len() < 4 {
            Err(DescriptorError::NotEnoughData { actual: buf.len(), expected: 4 })
        } else {
            Ok(RegistrationDescriptor { buf })
        }
    }

    pub fn format_identifier(&self) -> u32 {
        u32::from(self.buf[0]) << 24
        | u32::from(self.buf[1]) << 16
        | u32::from(self.buf[2]) << 8
        | u32::from(self.buf[3])
    }

    pub fn additional_identification_info(&self) -> &[u8] {
        &self.buf[4..]
    }
}
impl<'buf> fmt::Debug for RegistrationDescriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(),fmt::Error> {
        f.debug_struct("RegistrationDescriptor")
            .field("format_identifier", &self.format_identifier())
            .field("additional_identification_info", &format!("{:x}", self.additional_identification_info().as_hex()))
            .finish()
    }
}

#[cfg(test)]
mod test {
    use data_encoding::hex;
    use super::*;

    #[test]
    fn descriptor() {
        let data = hex::decode(b"050443554549").unwrap();
        let desc = Descriptor::new(&data);
        assert_matches!(desc, Descriptor::Registration{ payload: b"CUEI" });
    }
}