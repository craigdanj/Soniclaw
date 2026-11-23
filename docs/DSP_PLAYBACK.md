# Advanced sampler region playback — 0.99.0

## Workflow and API

`soniclaw_dsp::sample_playback::RegionPlayer` is a reusable single voice over an immutable `Arc<Sample>`. It adds reverse playback, ping-pong loops, wrap-loop overlap fades, region seeking, speed changes and loop release into the remaining tail. It is independent of Studio and is not a replacement for soniclaw-sampler::MultiSampler: that module already supplies zones, velocity layers/crossfades, round robins, release triggers, MPE, voice allocation and other instrument features. This release adds a lower-level playback primitive for those or custom instrument designs.

Construct `Config { schema: 1, start, end, reverse, speed, gain, interpolation, loop_region }`, prepare at the output rate, then `start()`. Regions and loops use source-frame indices with exclusive end. Prepare/reset leave the voice stopped. `next_frame(&mut frame)` or AudioProcessor replaces output; mono samples can be copied to stereo, while stereo samples require stereo output. `set_speed` retains phase; `seek` changes source-frame position without starting a stopped voice. The application supplies gain envelopes and click-free seek/voice transitions.

`Loop { start, end, mode, crossfade_frames }` wraps or reflects around loop endpoints. Ping-pong avoids duplicate endpoints and supports multi-boundary advances. Wrap crossfade overlaps the last N source frames with the first N (and the converse in reverse), using complementary linear weights, then skips the already-overlapped N frames at wrapping. Thus each loop cycle is end−start−N source frames, not the full raw region length. At N=0 the full period is retained. Ping-pong requires N=0. Sinc reads use boundary wrapping/reflection; the first pass may read actual pre-loop content. These choices affect loop pitch/phase and are intentionally explicit.

`release()` disables looping and continues in the current direction until the outer region endpoint; reverse release exits through region start. It does not add a gain release envelope or a separate release sample. Disabling a crossfade mid-overlap can change amplitude abruptly; schedule releases with an application envelope. `position` and `is_active` expose audio-owner state.

## Quality, bounds and errors

Interpolation selects existing Linear, SincFast or SincBest, with rate-aware windowed-sinc lowpass for the latter two. No callback allocation is needed, but these sinc modes compute trigonometric kernels per sample and can be costly for polyphony. This is resident PCM playback, not disk streaming or a complete Kontakt-like engine.

Sample limit 16,777,216 frames, mono/stereo; finite PCM magnitude ≤16. Source region must be nonempty and contained in PCM. Loop must lie inside the outer region and contain at least 16 frames; crossfade ≤one quarter loop length. Speed 0.125–8, gain 0–1; source/output rate times speed must be ≤32 source frames per output frame. Output 8–192 kHz, max block 8192. Invalid configuration, seek/speed or unprepared start returns an error before changing state. Invalid/unprepared output shapes are silenced without advancing. Small loops at high speed may cross several endpoints in a sample; reflection/wrapping remains bounded.

`match_loop_end(sample, start, ideal_end, radius, channel)` is an offline helper, scoring a 64-frame candidate tail against the loop-start overlap. Radius ≤256, all candidates and comparison windows inside PCM. It returns the minimum squared-error endpoint, not a perceptual seamless-loop guarantee. Run on the control/background thread and audition the result.

## Ownership, persistence and migration

Create/load/validate PCM off-thread; RegionPlayer retains its Arc through processing, reset and release. Do not drop the last PCM owner in a callback. Single audio owner; all start/seek/speed/release calls belong to it or its command queue. Prepare/reset do not allocate PCM; processing and triggers use existing memory. No buffering latency is declared: sinc lookahead reads already resident source frames. No automatic envelope, pitch tracking or independent time stretch is included.

Persist schema-1 Config plus an asset ID and its source rate/channel metadata; unknown fields/schema mismatch reject. Position, loop direction, release state and PCM are not serialized as a preset. Existing Sampler/MultiSampler formats are unchanged. Validate asset-region bounds again when resolving a preset; retain the previous instrument/voice when a new asset fails validation. For topology changes, prepare a replacement off-thread and transition with a voice envelope.

## Runnable example and evidence

`cargo run -p soniclaw-dsp-suite --offline -- playback NEW_DIRECTORY` generates forward, reverse, wrap-crossfaded and ping-pong examples from owned harmonic PCM. Unit tests check endpoint order, reverse start, overlap skip, multi-boundary reflection, release completion, zero callback allocations and block-size independence. See docs/validation/v0_99_0/. Examples demonstrate playback semantics; no commercial sampler parity, automatic seamless-loop or broad asset compatibility is claimed.
