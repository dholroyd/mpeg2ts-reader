#!/bin/bash

rm -f "target/report.dat"
echo -n "'ffmpeg-sys', " >> "target/report.dat"
jq '.Mean.point_estimate' < "target/criterion/crate:ffmpeg-sys/dts-check/new/estimates.json" >> "target/report.dat"
echo -n "'mpeg2ts', " >> "target/report.dat"
jq '.Mean.point_estimate' < "target/criterion/crate:mpeg2ts/dts-check/new/estimates.json" >> "target/report.dat"
echo -n "'mpeg2ts-reader', " >> "target/report.dat"
jq '.Mean.point_estimate' < "target/criterion/crate:mpeg2ts-reader/dts-check/new/estimates.json" >> "target/report.dat"

gnuplot -e 'set terminal svg;
set output "target/report.svg";
set style histogram;
set style data histogram;
plot "target/report.dat" using 2:xtic(1)'
