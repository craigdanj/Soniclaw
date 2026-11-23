# Hybrid instrument live smoke test

On 2026-11-21, the v0.109.0 example opened a 44.1 kHz virtual ALSA default
device and exited successfully after terminal note-on, note-off and quit
commands. The test sink used the ALSA null PCM behind a plug device. No physical
audio output or listening quality was evaluated. The offline v0.109.0 fixture
separately checked that the synth produces nonzero audio at the expected onset.
