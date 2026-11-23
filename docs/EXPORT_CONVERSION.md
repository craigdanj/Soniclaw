# Export sample-rate conversion — 0.68.0

## Workflow and API

In Reference Studio Export centre enable Convert output sample rate and choose 8,000..192,000 Hz. Timeline start/length/tail remain **project-rate frames**. Output duration is preserved; frame count is rounded once from the complete rendered duration. The project sample rate, arrangement, source assets and processor preparation rate remain unchanged. Choose an integer FLAC/WAV/AIFF format or float WAV/AIFF as before. Output is converted before peak/LUFS analysis, normalization, dither and final codec encoding, so conversion overshoot participates in peak measurement and the normalization ceiling.

`export::Settings::output_rate: Option<u32>` defaults to None (project rate) when absent from schema-1 JSON. Existing settings therefore retain their behavior. Rust struct literals without `..Default::default()` must add `output_rate: None`. `export_convert::convert(input_float_staging_wav, new_output_wav, target_rate, context)` is a lower-level worker-only converter for stereo float32 staging WAVs; it returns exact output frames. General decode/format support belongs to the export centre/format manager rather than this staging helper.

```sh
cargo run -p soniclaw-export-conversion -- NEW_DIRECTORY
```

The example renders a 48 kHz project to 44.1, 96 and 8 kHz, decodes each WAV, verifies exactly one second of destination frames and a preserved 110 Hz test tone using positive zero crossings. It checks finite samples and unchanged project rate. It is a functional pitch/duration test, not a comprehensive converter benchmark.

## Implementation and ownership

The existing `dsp::resample::ResampleQuality::SincBest` kernel performs anti-aliased windowed-sinc interpolation. Absolute output frame indices determine source phase, avoiding accumulated per-block timing drift. Each iteration seeks/reads only the source span and kernel overlap needed for up to 4,096 output frames; out-of-source samples are zero padded. Working memory depends on bounded rate ratio and kernel radius, not song length. DSP instances render at the project rate before conversion. No converter work belongs on a device callback.

Cancellation is checked between output blocks, and progress reports conversion frames. Conversion runs inside the shared export job's reservation and exclusive output directory. Generated staging WAVs are removed after encoding; failed/cancelled export uses the existing owned-directory cleanup. The lower-level converter refuses an existing output. Disk/codec/finalize failures can leave a staging file for its owning caller to clean; the export centre performs that cleanup. No other file is overwritten.

## Analysis, persistence and limits

LUFS normalization now requires **output** rate 48 kHz, allowing another project rate to be converted for analysis. Non-48-kHz destination reports null LUFS; peak normalization works at every supported destination rate. Source peak/LUFS in export reports refer to converted floating staging PCM. Report sample_rate is the encoded destination rate; Settings records source-frame range and requested output rate, while project rate remains in the original project document. Source media fingerprints still guard the entire render/conversion job.

No project schema change. Export settings retain schema 1 with an optional backward-compatible input field; older library versions that deny unknown fields cannot read settings containing output_rate. This pre-1.0 compatibility boundary is explicit. Output checks include the converted RIFF frame bound; a range rounding to zero fails rather than manufacturing duration. Existing four-hour source-end, 32-output, stereo and codec bounds apply. The fixed quality choice is not a real-time resampling policy, seamless file concatenation guarantee or true-peak limiter. No MP3/AAC support is added. Executed release checks and example evidence are in the validation report; physical audio/native DAW acceptance remains unrun.
