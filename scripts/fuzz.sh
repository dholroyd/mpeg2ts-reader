#!/bin/sh

RUST_BACKTRACE=1 cargo +nightly fuzz run fuzz_target_1 -- #-max_total_time=60
