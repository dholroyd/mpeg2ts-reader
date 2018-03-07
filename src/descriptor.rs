use std::fmt;
use hex_slice::AsHex;

pub struct Descriptor<'buf> {
    buf: &'buf[u8]
}
impl<'buf> Descriptor<'buf> {
    pub fn new(buf: &'buf[u8]) -> Descriptor<'buf> {
        assert!(buf.len() >= 2);
        Descriptor {
            buf,
        }
    }
    pub fn tag(&self) -> u8 {
        self.buf[0]
    }
    pub fn len(&self) -> u8 {
        self.buf[1]
    }
    pub fn payload(&self) -> &[u8] {
        &self.buf[2..2+self.len() as usize]
    }
}
impl<'buf> fmt::Debug for Descriptor<'buf> {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Descriptor {{ tag: {}, len: {} }}", self.tag(), self.len())
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
        assert_eq!(5, desc.tag());
        assert_eq!(4, desc.len());
        assert_eq!(b"CUEI", desc.payload());
    }
}