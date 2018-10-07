use mpeg2ts::pes::{PesPacketReader, ReadPesPacket};
use mpeg2ts::ts::TsPacketReader;

pub struct Mpeg2ts;
impl Mpeg2ts {
    pub fn check_timestamps(buf: &[u8]) {
        let reader = TsPacketReader::new(buf);
        let mut pes_reader = PesPacketReader::new(reader);
        let mut last_ts = None;
        loop {
            match pes_reader.read_pes_packet() {
                Ok(Some(pes)) => {
                    if pes.header.stream_id.as_u8()!=224 {
                        continue;
                    }
                    let this_ts = if let Some(dts) = pes.header.dts {
                        Some(dts.as_u64())
                    } else if let Some(pts) = pes.header.pts {
                        Some(pts.as_u64())
                    } else {
                        None
                    };
                    if let Some(this) = this_ts {
                        if let Some(last) = last_ts {
                            if this <= last {
                                println!("Non-monotonically increasing timestamp, last={}, this={}", last, this);
                            }
                        }
                        last_ts = Some(this);
                    }
                },
                Ok(None) => break,
                Err(_) => (),
            }
        }
    }
}