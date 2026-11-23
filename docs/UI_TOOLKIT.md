# Themeable audio UI toolkit

The egui toolkit now includes configurable theme tokens, continuous and stepped
knobs, modulation rings, vertical faders, toggles, enum controls and prepared
SVG/filmstrip assets. The controls bind the same shared `Parameter` handles used
by DSP/host integration; custom native applications can compose them freely.

## Run the native gallery

```sh
cargo run -p soniclaw-ui-gallery --locked
# Save a reproducible native GPU frame and JSON evidence; use a new PNG path:
cargo run -p soniclaw-ui-gallery --locked -- --theme 1 --snapshot new-gallery/light.png
```

Theme0 is Carbon,1 Studio paper,2 Amber. The gallery links multiple controls to
the same gain, shows a cutoff modulation ring, SVG play button,13-frame SVG
filmstrip, theme-aware meter/EQ/envelope/XY/waveform and keyboard. It does not
open an audio device. Changing its theme updates the prepared assets too. The
snapshot option requests an actual eframe framebuffer image, writes PNG plus a
JSON companion, and closes after receiving the image. It does not synthesize a
picture of a hypothetical UI. Normal interactive mode stays open.

## Public API

```rust,no_run
use soniclaw_ui::{self as ui,Theme,KnobOptions};
# fn panel(context:&ui::egui::Context,view:&mut ui::egui::Ui,p:&soniclaw_core::Parameter)->Result<(),&'static str>{
let theme=Theme::light();
theme.apply(context)?;
ui::audio_knob(view,p,KnobOptions{modulation:Some(0.12),..Default::default()});
ui::audio_fader(view,p,theme.fader_height);
ui::audio_toggle(view,p); // normally bind a boolean control
# Ok(()) }
```

`Theme` schema1 stores RGBA background/surface/control/accent/text/muted/danger
colors, spacing, corner radius, knob size and fader height. `dark/light/amber`
provide presets. `apply` updates egui visuals/metrics and retains an immutable
current theme in the context. `current_theme` returns that shared palette.
Existing `apply_theme` applies Carbon, retaining the old call site. The core
meter and legacy knob/XY/waveform/EQ/editor accents/backgrounds use these tokens;
fixed semantic piano key colors retain their musical meaning.

`audio_knob` uses the parameter's normalized control space. This naturally uses
the logarithmic/linear law from a typed float declaration. Its bipolar option
starts the displayed value arc at the midpoint. The optional modulation value
is a normalized display offset; the ring is clamped to0...1 and **does not change
DSP modulation**. Bind the real modulation signal explicitly if desired.
`audio_fader` uses the same parameter space and quantization. `audio_toggle`
sets normalized0/1; `parameter_control` remains the typed bool/enum/numeric
selector from v0.32. Legacy scalar logarithmic controls may keep the existing
`parameter_knob(...,true)` API; the new knob does not guess a skew from numeric
ranges.

Knob/fader/filmstrip drags accumulate an unquantized target so small individual
mouse motions can cross a discrete step. Shift applies fine sensitivity without
jumping the anchor. Double-click restores the declared default; click focuses;
arrows step continuously or by exactly one declared discrete interval; Home/End
select endpoints. These custom controls support keyboard focus but this release
does not claim complete screen-reader/platform accessibility certification.
Typed exported native editors use the new knob for codec-backed continuous
parameters and existing discrete controls for booleans/enums/integers.

## SVG icons and filmstrips

Enable `soniclaw` feature `ui-svg`, or `soniclaw-ui` feature `svg`:

```rust,no_run
# #[cfg(feature="svg")]
# fn assets(context:&egui::Context,view:&mut egui::Ui,p:&soniclaw_core::Parameter)->Result<(),String>{
let texture=soniclaw_ui::svg_texture(context,"play",include_bytes!("play.svg"),[32,32])?;
view.add(egui::Button::image(egui::Image::new((texture.id(),egui::vec2(24.,24.)))));
# Ok(()) }
```

`svg_texture` parses and rasterizes actual SVG vector geometry using pinned
`resvg0.48.1`, then uploads premultiplied RGBA to an owned egui texture. Decode/
rasterize/upload once and retain the handle; do not repeat that work every UI
frame. Pixel dimensions are explicit so the application can prepare assets for
its desired display scale. External file images are ignored by the nested SVG
parser; this build disables optional text-font/raster-image features. Convert
icon text to paths or use normal egui text for labels. No JavaScript/DOM runtime
is involved. [Renderer API](https://docs.rs/resvg/0.48.1/resvg/).

`Filmstrip::new(texture,frame_count,StripAxis::{Horizontal,Vertical})` validates
that frames divide the texture axis exactly. `frame_uv(normalized)` selects a
nearest frame; `show` draws it and binds the same drag/keyboard interaction.
Keep the returned `Filmstrip` alive while visible. It owns a texture reference;
drop releases its reference through egui's normal texture lifecycle. The gallery
creates its13 dial images from real SVG paths; applications can supply their
own decoded image texture or SVG sheet.

## Ownership, state and errors

All theme/widget/asset APIs belong to the UI/control side. Asset parsing,
formatting, shape building, texture upload and native snapshots may allocate.
Never run these APIs in an audio callback. DSP parameter reads remain the
existing atomic contract. Applications using the native editor keep its queued
detached-edit/acknowledgment bridge; custom apps choose their own accepted-edit
transport rather than assuming widgets alone provide an undo/host gesture bus.

Serialize `Theme` with serde_json; `Theme::from_json` bounds input to8KiB and
validates schema/metrics. Theme colors/layout are application preferences, not
plug-in DSP state. Textures, egui IDs, drag anchors and focus are runtime state
and are rebuilt after reopening. Existing project/preset schemas do not change.
Use stable parameter IDs and normal egui `push_id` scopes when displaying several
views of the same product.

Theme spacing4–32, corner radius≤32, knob size40–160 and fader height80–480 must
be finite. Invalid theme/JSON returns an error before applying. Widget optional
size/sensitivity arguments are clamped to documented drawable bounds; nonfinite
modulation is ignored. Filmstrips allow2–1024 frames and reject partial frames.
SVG bytes≤1MiB, dimensions1–4096 and raster≤16Mi pixels. Rasterization errors
return `String` diagnostics; retain a previous valid asset or show a normal text
button as the application fallback. Bounds limit normal asset work; this API is
intended for authored UI assets and is not a general untrusted SVG service.

## Validation

Run `cargo test -p soniclaw-ui --all-features --locked`. Tests check theme JSON
roundtrips/bounds, exact endpoint UVs, real red SVG raster pixels, and keyboard
plus accumulated discrete drags (one48px move vs24 small moves). The native
three-theme gallery screenshots and reports record actual Linux GPU rendering.
The release validation report links those images, strict Rust checks and any
additional acceptance. Physical input hardware, external DAWs and native
Windows/macOS runtime behavior remain separately pending.
