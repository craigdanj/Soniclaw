# Borrowed audio sub-blocks

Use `Block::sub_block`, `BlockMut::sub_block` or `BlockMut::sub_block_mut` to address a frame and channel rectangle in existing audio storage. All ranges are half-open. Both planar and interleaved parents retain their original strides. Selecting a rectangle creates a borrow without allocating or copying samples.

```rust
use soniclaw_dsp::block_dsp::{BlockMut, Layout};
let mut samples = [1.0; 256];
let mut block = BlockMut::new(&mut samples, 2, Layout::Interleaved)?;
let mut middle = block.sub_block_mut(16..80, 0..2)?;
for frame in 0..middle.as_view().frames() {
    for sample in middle.frame_mut(frame).unwrap() {
        *sample *= 0.5;
    }
}
# Ok::<(), soniclaw_core::Error>(())
```

- `BlockView` supports checked `sample`, shape inspection, and nested read views.
- `BlockViewMut` supports `as_view`, exclusive nested views, `sample_mut`, `fill` and shape-checked `copy_from`.
- `channel[_mut]` and `frame[_mut]` return a slice when the corresponding stride is one. Use the checked sample accessors for other shapes.
- `copy_from` explicitly copies matching rectangles and can convert layouts. A shape mismatch returns before writes.

```sh
cargo run -p soniclaw-dsp-suite --locked -- block-view /tmp/soniclaw-block-view
```

The example copies a selected planar interval into an existing interleaved buffer, changes the borrowed region in place and records the allocation count and a stereo WAV.

## Lifecycle, bounds and migration

Borrow a view for the lifetime of its parent samples. The same mutable parent remains exclusively borrowed during a mutable view's lifetime. Take separate views sequentially or split the surrounding storage before borrowing when simultaneous disjoint mutable regions are necessary. Creating views and using the checked methods is callback-safe. Existing contiguous `BlockMath` APIs still take whole contiguous blocks.

Empty frame ranges are supported. Channels must be nonempty and ranges must be within the parent. Invalid ranges or a copy shape mismatch return `InvalidBuffer`. Out-of-bounds sample or slice access returns `None`. Persist your region coordinates or editing intent separately. The view itself holds a live borrow and introduces no state format.

[Release validation](validation/v0_141_0/REPORT.md)
