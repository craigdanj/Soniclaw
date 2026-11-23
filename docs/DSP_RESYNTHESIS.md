# Harmonic and residual spectral resynthesis

Prepare Resynth with FFT size 512–8192, up to 64 tracked harmonics, and Controls. Supply the fundamental from MIDI, a monophonic pitch detector or offline pitch analysis. At each hop the processor searches near each expected harmonic, estimates a sub-bin peak and smooths its location over frames. It uses a soft spectral mask and its complement to scale harmonic and residual components separately. The residual includes untracked tones and transients as well as noise. Read partials on the audio owner or publish a copy through a bounded queue.

Use gain pairs 1/0 and 0/1 on identical streams to render complementary stems. At equal gains of 1, the admitted input returns after one FFT window of buffering. The example checks stem recombination and verifies the first tracked partial before reporting WAVs. For difficult material increase window size or provide a better fundamental. The tracker needs two bins of harmonic separation, and octave errors in the fundamental also affect separation. It targets pitched monophonic sources. Independent copies of the processor render each output stem.

Prepare and build STFT plans off the audio thread. set_controls, process and reset are allocation-free. Use one mutable owner. Persist window size, partial count and Controls in the host. Retain earlier preset layouts. Limits are 8–192 kHz, 1–8 channels, 25–4000 Hz fundamental, gains 0–2 and mask width 1–4 bins. Malformed control updates are transactional, and invalid callback sizes clear output. Reset clears overlap, partial history and masks.

Run `cargo run -p soniclaw-dsp-suite --locked --offline -- spectral-resynthesis NEW_DIRECTORY`. [Executed checks](validation/v0_111_0/REPORT.md).
