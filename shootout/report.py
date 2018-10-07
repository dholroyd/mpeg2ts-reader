# post-processes the output of 'cargo bench' to summarise throughput of each
# implementation

import json
import matplotlib.pyplot as plt; plt.rcdefaults()
import numpy as np
import matplotlib.pyplot as plt

GIGABYTE = 1024*1024*1024
NANOSECONDS_PER_SECOND = 1000000000

def into_gbytes_per_second(byte_count, nanos):
    return byte_count * NANOSECONDS_PER_SECOND / nanos / GIGABYTE

def get_benchmark_data(bench):
    with open("target/criterion/{}/dts-check/new/estimates.json".format(bench), "r") as f:
        dat = json.load(f)
        point_estimate = dat["Mean"]["point_estimate"]
        lower_bound = dat["Mean"]["confidence_interval"]["lower_bound"]
        upper_bound = dat["Mean"]["confidence_interval"]["upper_bound"]
    with open("target/criterion/{}/dts-check/new/benchmark.json".format(bench), "r") as f:
        dat = json.load(f)
        throughput_bytes = dat["throughput"]["Bytes"]

    return {
        "point_estimate": into_gbytes_per_second(throughput_bytes, point_estimate),
        "lower_bound": into_gbytes_per_second(throughput_bytes, lower_bound),
        "upper_bound": into_gbytes_per_second(throughput_bytes, upper_bound),
    }

bench_names = ["crate:ffmpeg-sys", "crate:mpeg2ts", "crate:mpeg2ts-reader"]
benches = [get_benchmark_data(name) for name in bench_names]

plt.figure(figsize=(8, 6), dpi=80)

y_pos = np.arange(len(bench_names))
performance = [bench["point_estimate"] for bench in benches]

fig, ax = plt.subplots()
rects = plt.barh(y_pos, performance, align='center', color='black', alpha=0.5, height=0.5)
(x_left, x_right) = ax.get_xlim()
x_width = x_right - x_left
ax.margins(x=0.1)  # make space for labels right-of bars
for rect in rects:
    width = rect.get_width()
    ax.text(width+(0.01*x_width),rect. get_y() + rect.get_height()/2.,
            '%.1f' % width,
            ha='left', va='center')

plt.yticks(y_pos, [b.split(":")[1] for b in bench_names])
plt.xlabel('Gigabytes per second')
plt.ylabel('Rust crate')
plt.title('Parsing throughput')
plt.tight_layout()
plt.subplots_adjust(left=0.25)  # more space for our crate-name labels

plt.savefig("report.svg")
