# Audio sessions and recording

`SessionSettings` is a versioned, serializable request. `SessionController` is optional under the `native` feature and owns a `DuplexHost` and `Recorder`. `poll()` belongs on the UI/control thread. Nothing enumerates devices, reopens a stream or opens a file inside an audio callback.

## Device negotiation

Enumeration exposes each device's channel count, sample-rate range, sample format and optional buffer range. The native adapter chooses a supported F32/I16/U16 configuration that covers the requested channel map and rate, preferring fewer hardware channels and F32. Known buffer ranges are checked before opening; an unknown range still requires driver acceptance. Unsupported settings fail explicitly. Input and output use the same requested sample rate.

The processing layout is stereo. Input `[0,0]` duplicates one selected mono channel; `[0,1]` selects a stereo pair. Output destinations must be distinct and may be any supported hardware channels. Other output channels are cleared. Mono output devices are not supported by `DuplexHost` yet; portable `BusEngine` supports other layouts.

CPAL 0.15 device names are persisted, not globally stable hardware IDs. A renamed device requires reselection. Duplicate matching names are rejected. An output name of `None` selects the current system default when opening; input `None` disables capture. Defaults do not silently substitute for a missing explicitly named device.

## Clock bridge

Input is converted into `[f32;2]` frames and pushed into a bounded SPSC queue. Output owns an interpolating reader. It waits for prefill, then adjusts consumption around one input frame per output frame according to queue occupancy, bounded to ±0.2%. Small independent clock drift is handled with linear interpolation. Underruns return silence and restart prefill; overflow drops newly arriving frames. Both are counted. This is not a solution for arbitrary rate differences or disconnected drivers.

The prefill target is twice the requested hardware buffer, with the buffer contribution clamped to 64–8192 frames; driver default uses 512, yielding a 1024-frame prefill. Queue capacity is eight times prefill. The UI reports prefill separately; end-to-end latency has not been measured. Use the same interface for initial input/output testing.

## Recovery

The controller polls at most once per second. Stream-error counters or missing active device names stop the session. If automatic reconnection is enabled, the controller retries the saved request and constructs a new engine from its factory. Shared `Arc<Parameter>` handles can be retained in that factory, preserving controls while resetting voices, delay history and transport. Monitor mute resets; recording finalizes and does not resume automatically.

Driver behavior differs across platforms. The code path is implemented, but physical unplug/replug validation is still required. No claim is made that every driver reports disconnects identically or that recovery is seamless.

## Recording lifecycle

`Recorder::new` creates the queue, worker, control handle and `RecordTap`. Move the tap into one audio callback and keep the controller outside it. Start creates a new WAV destination on the worker and only then arms the tap. Existing files are refused. Packets contain a generation, absolute frame position, frame count and fixed sample storage.

The native dry tap is selected input after the clock bridge; it is not a raw multi-channel device dump. The tap copies dry or processed stereo audio in packets of at most 256 frames. A full queue increments `dropped_frames`; the worker fills any missing positions with silence. Stop disarms capture, waits on the control thread for in-flight enqueue calls, drains the queue, fills any final lost frames and finalizes the header. A new take uses a new generation. `RecordSummary` remains available after finalization; the native controller retains the last take's summary and finalization error across recovery.

WAV is currently stereo PCM16. Samples saturate at [-1,1] and non-finite values become silence. Recording does not dither, stream multiple files, capture MIDI, recover from disk-full by itself, or preserve a full project. If the disk write fails, stop returns an error and counters identify the failed take. File generation tests include overflow and concurrent capture/stop/restart.
