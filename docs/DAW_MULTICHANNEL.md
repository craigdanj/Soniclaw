# Independent multichannel recording — 0.74.0

## Recording workflow

Create audio tracks, enable input, choose the interface in Device settings, then arm each track in Recording controls. Select **Dry mapped input**, enable **Independent hardware channels**, choose a mono channel or stereo pair (indices start at zero), and optionally enable monitoring through that track. Other hardware channels do not enter that track. Existing legacy stereo input mapping remains available when independent routing is disabled. Save stores independent channel choices in project layout; Open restores them as dry-input arms. Monitoring/arming remain explicit session controls.

Record opens synchronized float WAV stems. Initial bridge prefill is awaited before the recording clock starts. Finish/Stop closes the device, finalizes files and places gap-free linear dry-input recordings as editable disk regions on their armed tracks. Existing regions/media are preserved; Undo removes the placement without deleting recordings. Rendered/post-insert stems remain in the take catalog to avoid applying their inserts twice on the source track. Master and loop take workflows remain available. Loop recordings use comp/take lanes rather than a flattened linear placement. Input underruns/overflows are reported as take quality; recordings with such discontinuities are retained for inspection but not automatically placed or accepted for gap-free comping.

```sh
cargo run -p soniclaw-daw-multichannel -- NEW_DIRECTORY
```

The example drives four synthetic hardware channels, independently records a mono mic and stereo pair, checks exact WAV values and channel counts, places both, and reopens a portable save. Its test checks callback allocation behavior. Native hardware is not opened by this example.

## Public API and lifecycle

`core::ChannelBus::new(channels,max_frames)` prepares an atomic PCM block exchange. A single device callback writes once before its engine runs; readers use the same prepared bus. It is not a queue between asynchronous independent engines. `write`, `sample`, `set_offset`, `ready` and discontinuity counters allocate nothing in the callback. Application code must preserve the single-writer sequencing contract. `studio::inputs::Setup` binds track IDs to one/two hardware channels; `Document::compile_inputs_at` validates routes and attaches them to prepared track processing. `RecordingSource::InputChannels` records selected raw channels with native mono/stereo width. `recording::prepare_with_inputs` prepares the same bus for track monitoring and dry capture; `place_input_take` transactionally places linear dry stems.

`device::OutputEngine::input_bus` passes the bus into the native duplex adapter. `DuplexRoute::input_count` requests enough physical channels during negotiation. `DuplexOptions::input_bus` supports direct adapter use. Match rate/block size and ensure an active input device; unsupported maps/counts fail before streaming. Hardware bridge frames retain up to 64 channels and share one interpolation phase, preserving interchannel alignment. Track mixing remains stereo. The adapter downmaps the legacy stereo input separately, while independent routes use raw channel indices. All allocation, graph construction and file operations occur during preparation or on control/recording workers. Stop the device before finalizing or mutating track structure.

## Persistence, bounds and recovery

Project schema 3 gains optional layout input routes. Missing routes preserve previous behavior and legacy identity hashing. Mono sources are duplicated to stereo only for monitoring/mixing; recorded files stay mono. Take schema 1 gains `input_discontinuities` (default zero), distinguishing physical input quality from writer queue drops and missing acquisition frames. Strict older readers may reject the new field. The count is saved in take.json and survives portable project copies; schedule/quality sidecars are copied when present, each at most 32 KiB. DSP/output and disk errors retain their existing reports. Input routes are project-relative track IDs, never device handles. Device selection stays machine-specific.

Existing limits apply: at most 64 physical channels, 32 armed stems and 64 total recorded stem channels; individual recording arms select one or two channels, 1024 max recording block frames, 16 streamed playback regions and two GiB portable resources. A ChannelBus supports up to 8192 prepared frames for other clients. Structural edits discard input routes for deleted tracks. Hardware clock drift correction uses shared linear interpolation and is not a replacement for a single-clock studio interface or measured latency calibration. A discontinuity count signals one or more bridge errors, not a precise number of lost physical samples. At zero project time, early compensated audio can require trimming; calibration is added by the next timing release.

## Evidence and limitations

Executed unit tests and the example verify isolated source values, native mono/stereo file layout, zero-allocation track routing, portable reopen and existing count-in/punch behavior. Full workspace Clippy/docs and selected device/record/studio/desktop tests are recorded in validation. These checks do not establish actual interface latency, driver stability or Windows/macOS physical-device acceptance. Device loss/reconnection during a take must be treated as a damaged performance and reviewed, never silently accepted as a clean recording.
