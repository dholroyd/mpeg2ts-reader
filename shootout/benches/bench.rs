#[macro_use]
extern crate criterion;
extern crate mpeg2ts_reader_shootout;

use criterion::{Criterion,Benchmark,Throughput};
use std::fs::File;
use std::io::Read;
use mpeg2ts_reader_shootout::mpeg2ts_timestamps::Mpeg2ts;
use mpeg2ts_reader_shootout::mpeg2ts_reader_timestamps::Mpeg2tsReader;
use mpeg2ts_reader_shootout::ffmpeg_timestamps::Ffmpeg;

fn load_sample_data() -> (Vec<u8>, usize) {
    let mut f = File::open("../big_buck_bunny_1080p_24fps_h264.ts").expect("file not found");
    let l = f.metadata().unwrap().len() as usize;
    let size = l.min(188*200_000);
    let mut buf = vec![0; size];
    f.read(&mut buf[..]).unwrap();
    (buf, size)
}

fn mpeg2ts(c: &mut Criterion) {
    let (buf, size_bytes) = load_sample_data();
    c.bench("crate:mpeg2ts", Benchmark::new("dts-check", move |b| {
        b.iter(|| {
            Mpeg2ts::check_timestamps(&buf[..]);
        } );
    }).throughput(Throughput::Bytes(size_bytes as u32)));
}

fn mpeg2ts_reader(c: &mut Criterion) {
    let (buf, size_bytes) = load_sample_data();
    c.bench("crate:mpeg2ts-reader", Benchmark::new("dts-check", move |b| {
        b.iter(|| {
            let mut r = Mpeg2tsReader::new();
            r.check_timestamps(&buf[..]);
        } );
    }).throughput(Throughput::Bytes(size_bytes as u32)));
}

fn ffmpeg(c: &mut Criterion) {
    let (buf, size_bytes) = load_sample_data();
    let f = Ffmpeg::new();
    c.bench("crate:ffmpeg-sys", Benchmark::new("dts-check", move |b| {
        b.iter(|| {
            f.check_timestamps(&buf[..]);
        } );
    }).throughput(Throughput::Bytes(size_bytes as u32)));
}


criterion_group!(benches, ffmpeg, mpeg2ts_reader, mpeg2ts);
criterion_main!(benches);
