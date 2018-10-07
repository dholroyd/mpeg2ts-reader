use ffmpeg_sys::*;
use std::ptr;
use std::mem;
use std::ffi;

pub struct Ffmpeg{
    fmt: *const AVInputFormat,
}
impl Ffmpeg {
    pub fn new() -> Ffmpeg {
        unsafe {
            av_register_all();
            let fmt_name = ffi::CString::new("mpegts").unwrap();
            let fmt = av_find_input_format(fmt_name.as_ptr());
            if fmt.is_null() {
                panic!("could not find format");
            }
            Ffmpeg { fmt }
        }
    }

    pub fn check_timestamps(&self, buf: &[u8]) {
        unsafe {
            let ioctx = avio_alloc_context(buf.as_ptr() as *mut u8,
                                           buf.len() as i32,
                                           0,
                                           ptr::null_mut(),
                                           None,
                                           None,
                                           None);
            let mut ctx = avformat_alloc_context();
            (*ctx).pb = ioctx;
            let ret = avformat_open_input(&mut ctx,
                                          ptr::null_mut(),
                                          self.fmt as *mut AVInputFormat,
                                          ptr::null_mut());
            if ret < 0 {
                panic!("avformat_open_input() failed {}", ret);
            }

            let mut pk: AVPacket  = mem::zeroed();
            av_init_packet(&mut pk);
            let mut last_ts = None;
            while av_read_frame(ctx, &mut pk) >= 0 {
                if pk.stream_index == 0 {  // the video stream, in our copy of big_buck_bunny
                    let this_ts = if pk.dts != AV_NOPTS_VALUE {
                        Some(pk.dts)
                    } else if pk.pts != AV_NOPTS_VALUE {
                        Some(pk.pts)
                    } else {
                        None
                    };
                    if let (Some(this), Some(last)) = (this_ts, last_ts) {
                        if this <= last {
                            println!("Non-monotonically increasing timestamp, last={}, this={}", last, this);
                        }
                    }
                    last_ts = this_ts;
                }
                av_packet_unref(&mut pk);
            }
            // prevent libav from trying to free our buffer,
            (*ioctx).buffer = ptr::null_mut();
            avformat_close_input(&mut ctx);
            avformat_free_context(ctx);
        }
    }
}