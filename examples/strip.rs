use mpeg2ts_reader::demultiplex;
use mpeg2ts_reader::packet::Packet;
use std::fs::File;
use std::io::Read;
use std::{env, io, marker};

pub enum StripFilterSwitch<W: io::Write> {
    Write(PacketWriter<W>),
    Strip(PacketStripper<W>),
}
impl<W: io::Write> demultiplex::PacketFilter for StripFilterSwitch<W> {
    type Ctx = StripDemuxContext<W>;
    #[inline(always)]
    fn consume(&mut self, ctx: &mut StripDemuxContext<W>, pk: &Packet<'_>) {
        match self {
            &mut StripFilterSwitch::Write(ref mut f) => f.consume(ctx, pk),
            &mut StripFilterSwitch::Strip(ref mut f) => f.consume(ctx, pk),
        }
    }
}

pub struct StripDemuxContext<W: io::Write> {
    changeset: demultiplex::FilterChangeset<StripFilterSwitch<W>>,
    out: W,
    written: u64,
    stripped: u64,
}
impl<W: io::Write> demultiplex::DemuxContext for StripDemuxContext<W> {
    type F = StripFilterSwitch<W>;

    fn filter_changeset(&mut self) -> &mut demultiplex::FilterChangeset<Self::F> {
        &mut self.changeset
    }
    fn construct(&mut self, req: demultiplex::FilterRequest<'_, '_>) -> Self::F {
        match req {
            // 'Stuffing' data on PID 0x1fff may be used to pad-out parts of the transport stream
            // so that it has constant overall bitrate.  Since we are stripping these packets,
            // we use an implementation of PacketFilter that just ignores them
            demultiplex::FilterRequest::ByPid(mpeg2ts_reader::STUFFING_PID) => {
                StripFilterSwitch::Strip(PacketStripper::default())
            }
            // for any other packet, we will use a PacketFilter implementation that writes the
            // packet to the output
            _ => StripFilterSwitch::Write(PacketWriter::default()),
        }
    }
}
impl<W: io::Write> StripDemuxContext<W> {
    pub fn new(out: W) -> Self {
        StripDemuxContext {
            changeset: demultiplex::FilterChangeset::default(),
            out,
            written: 0,
            stripped: 0,
        }
    }
}

pub struct PacketWriter<W: io::Write>(marker::PhantomData<W>);
impl<W: io::Write> demultiplex::PacketFilter for PacketWriter<W> {
    type Ctx = StripDemuxContext<W>;

    fn consume(&mut self, ctx: &mut Self::Ctx, pk: &Packet<'_>) {
        ctx.out
            .write_all(pk.buffer())
            .expect("writing to stdout failed");
        ctx.written += 1;
    }
}
impl<W: io::Write> Default for PacketWriter<W> {
    fn default() -> Self {
        PacketWriter(marker::PhantomData::default())
    }
}

pub struct PacketStripper<W: io::Write>(marker::PhantomData<W>);
impl<W: io::Write> demultiplex::PacketFilter for PacketStripper<W> {
    type Ctx = StripDemuxContext<W>;

    fn consume(&mut self, ctx: &mut Self::Ctx, _pk: &Packet<'_>) {
        ctx.stripped += 1;
    }
}
impl<W: io::Write> Default for PacketStripper<W> {
    fn default() -> Self {
        PacketStripper(marker::PhantomData::default())
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // open input file named on command line,
    let name = env::args().nth(1).unwrap();
    let mut f = File::open(&name).unwrap_or_else(|_| panic!("file not found: {}", &name));

    let out = io::stdout();
    let buf = io::BufWriter::new(out);

    // create the context object that stores the state of the transport stream demultiplexing
    // process
    let mut ctx = StripDemuxContext::new(buf);

    // create the demultiplexer, which will use the ctx to create a filter for pid 0 (PAT)
    let mut demux = demultiplex::Demultiplex::new(&mut ctx);

    // consume the input file,
    let mut buf = [0u8; 188 * 1024];
    loop {
        match f.read(&mut buf[..]).expect("read failed") {
            0 => break,
            n => demux.push(&mut ctx, &buf[0..n]),
        }
    }
    eprintln!(
        "Written {} TS packets, stripped {} TS packets",
        ctx.written, ctx.stripped
    );
}
