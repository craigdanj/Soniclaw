# Sonora v0.2 vs v0.3 Linux processing

Median batch-average thread CPU per callback, microseconds; 48 kHz stereo. Lower is better. Nine shuffled rounds per cell. Baseline crate sources are frozen and hashed.

| Workload | Frames | v0.2 µs | v0.3 µs | Speedup |
|---|---:|---:|---:|---:|
| Pass-through | 64 | 0.200 | 0.156 | 1.29× |
| Pass-through | 128 | 0.398 | 0.285 | 1.40× |
| Pass-through | 256 | 0.752 | 0.556 | 1.35× |
| Pass-through | 512 | 1.459 | 1.184 | 1.23× |
| Pass-through | 1024 | 2.681 | 2.290 | 1.17× |
| Static gain | 64 | 0.330 | 0.172 | 1.92× |
| Static gain | 128 | 0.604 | 0.306 | 1.98× |
| Static gain | 256 | 1.215 | 0.607 | 2.00× |
| Static gain | 512 | 2.276 | 1.219 | 1.87× |
| Static gain | 1024 | 4.836 | 2.405 | 2.01× |
| Low-pass + gain | 64 | 0.840 | 0.748 | 1.12× |
| Low-pass + gain | 128 | 1.656 | 1.391 | 1.19× |
| Low-pass + gain | 256 | 2.977 | 2.669 | 1.12× |
| Low-pass + gain | 512 | 6.300 | 5.310 | 1.19× |
| Low-pass + gain | 1024 | 12.353 | 10.891 | 1.13× |
| Automated gain | 64 | 0.369 | 0.417 | 0.88× |
| Automated gain | 128 | 0.687 | 0.660 | 1.04× |
| Automated gain | 256 | 1.244 | 1.104 | 1.13× |
| Automated gain | 512 | 2.294 | 3.533 | 0.65× |
| Automated gain | 1024 | 4.769 | 5.821 | 0.82× |
| Automated cutoff | 64 | 0.910 | 0.779 | 1.17× |
| Automated cutoff | 128 | 1.709 | 1.515 | 1.13× |
| Automated cutoff | 256 | 3.270 | 2.765 | 1.18× |
| Automated cutoff | 512 | 6.162 | 5.131 | 1.20× |
| Automated cutoff | 1024 | 11.976 | 10.452 | 1.15× |

Both versions are measured with the same independent exported-CLAP host, compiler version and O3/thin-LTO release settings. The baseline library does not include a GUI. The v0.3 reference does, but its window is closed during timing. Both use the same processing modes and parameter values.

Shared VM: no exclusive physical core or fixed CPU frequency. Runs are sequential, not interleaved between versions. Do not interpret small gaps as significant. No JUCE rerun or language-wide conclusion is implied. Timing includes common host/event bookkeeping. Raw batches retain round ranges. These are CPU costs, not physical audio latency.
