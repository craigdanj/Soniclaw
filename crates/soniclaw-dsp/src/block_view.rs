//! Checked borrowed rectangles in planar or interleaved blocks.
use crate::block_dsp::{Block, BlockMut, Layout};
use soniclaw_core::Error;
use std::ops::Range;

#[derive(Clone, Copy)]
struct Geometry {
    layout: Layout,
    frames: usize,
    channels: usize,
    origin: usize,
    frame_stride: usize,
    channel_stride: usize,
}

impl Geometry {
    fn new(layout: Layout, frames: usize, channels: usize) -> Self {
        let (frame_stride, channel_stride) = match layout {
            Layout::Planar => (1, frames),
            Layout::Interleaved => (channels, 1),
        };
        Self {
            layout,
            frames,
            channels,
            origin: 0,
            frame_stride,
            channel_stride,
        }
    }

    fn region(self, frames: Range<usize>, channels: Range<usize>) -> Result<Self, Error> {
        if frames.start > frames.end
            || frames.end > self.frames
            || channels.start >= channels.end
            || channels.end > self.channels
        {
            return Err(Error::InvalidBuffer);
        }
        let origin = if frames.is_empty() {
            self.origin
        } else {
            self.offset(frames.start, channels.start)
        };
        Ok(Self {
            origin,
            frames: frames.len(),
            channels: channels.len(),
            ..self
        })
    }

    fn offset(self, frame: usize, channel: usize) -> usize {
        self.origin + frame * self.frame_stride + channel * self.channel_stride
    }

    fn index(self, frame: usize, channel: usize) -> Option<usize> {
        (frame < self.frames && channel < self.channels).then(|| self.offset(frame, channel))
    }

    fn channel_span(self, channel: usize) -> Option<Range<usize>> {
        if channel >= self.channels || self.frame_stride != 1 {
            return None;
        }
        if self.frames == 0 {
            return Some(0..0);
        }
        let start = self.offset(0, channel);
        Some(start..start + self.frames)
    }

    fn frame_span(self, frame: usize) -> Option<Range<usize>> {
        if frame >= self.frames || self.channel_stride != 1 {
            return None;
        }
        let start = self.offset(frame, 0);
        Some(start..start + self.channels)
    }
}

/// Copyable read-only view. The parent layout's strides are retained.
#[derive(Clone, Copy)]
pub struct BlockView<'a> {
    data: &'a [f32],
    geometry: Geometry,
}

/// Exclusive borrowed view. Mutable descendants reborrow for a shorter lifetime.
pub struct BlockViewMut<'a> {
    data: &'a mut [f32],
    geometry: Geometry,
}

impl<'a> Block<'a> {
    pub fn sub_block(
        self,
        frames: Range<usize>,
        channels: Range<usize>,
    ) -> Result<BlockView<'a>, Error> {
        let geometry =
            Geometry::new(self.layout, self.frames, self.channels).region(frames, channels)?;
        Ok(BlockView {
            data: self.data,
            geometry,
        })
    }
}

impl BlockMut<'_> {
    pub fn sub_block(
        &self,
        frames: Range<usize>,
        channels: Range<usize>,
    ) -> Result<BlockView<'_>, Error> {
        let geometry =
            Geometry::new(self.layout, self.frames, self.channels).region(frames, channels)?;
        Ok(BlockView {
            data: self.data,
            geometry,
        })
    }
    pub fn sub_block_mut(
        &mut self,
        frames: Range<usize>,
        channels: Range<usize>,
    ) -> Result<BlockViewMut<'_>, Error> {
        let geometry =
            Geometry::new(self.layout, self.frames, self.channels).region(frames, channels)?;
        Ok(BlockViewMut {
            data: self.data,
            geometry,
        })
    }
}

impl<'a> BlockView<'a> {
    pub fn sub_block(self, frames: Range<usize>, channels: Range<usize>) -> Result<Self, Error> {
        Ok(Self {
            geometry: self.geometry.region(frames, channels)?,
            ..self
        })
    }

    pub fn layout(self) -> Layout {
        self.geometry.layout
    }
    pub fn frames(self) -> usize {
        self.geometry.frames
    }
    pub fn channels(self) -> usize {
        self.geometry.channels
    }

    pub fn sample(self, frame: usize, channel: usize) -> Option<f32> {
        self.geometry.index(frame, channel).map(|i| self.data[i])
    }

    /// Borrows the channel when its selected samples are contiguous.
    pub fn channel(self, channel: usize) -> Option<&'a [f32]> {
        self.geometry.channel_span(channel).map(|r| &self.data[r])
    }

    /// Borrows the frame when its selected samples are contiguous.
    pub fn frame(self, frame: usize) -> Option<&'a [f32]> {
        self.geometry.frame_span(frame).map(|r| &self.data[r])
    }
}

impl BlockViewMut<'_> {
    pub fn as_view(&self) -> BlockView<'_> {
        BlockView {
            data: self.data,
            geometry: self.geometry,
        }
    }

    pub fn sub_block_mut(
        &mut self,
        frames: Range<usize>,
        channels: Range<usize>,
    ) -> Result<BlockViewMut<'_>, Error> {
        Ok(BlockViewMut {
            data: self.data,
            geometry: self.geometry.region(frames, channels)?,
        })
    }

    pub fn sample_mut(&mut self, frame: usize, channel: usize) -> Option<&mut f32> {
        self.geometry
            .index(frame, channel)
            .map(|i| &mut self.data[i])
    }

    pub fn channel_mut(&mut self, channel: usize) -> Option<&mut [f32]> {
        self.geometry
            .channel_span(channel)
            .map(|r| &mut self.data[r])
    }

    pub fn frame_mut(&mut self, frame: usize) -> Option<&mut [f32]> {
        self.geometry.frame_span(frame).map(|r| &mut self.data[r])
    }

    pub fn fill(&mut self, value: f32) {
        for c in 0..self.geometry.channels {
            for i in 0..self.geometry.frames {
                self.data[self.geometry.offset(i, c)] = value;
            }
        }
    }

    /// Copies between matching shapes. Source and destination may differ in layout.
    pub fn copy_from(&mut self, source: BlockView<'_>) -> Result<(), Error> {
        if self.geometry.frames != source.geometry.frames
            || self.geometry.channels != source.geometry.channels
        {
            return Err(Error::InvalidBuffer);
        }
        for c in 0..self.geometry.channels {
            for i in 0..self.geometry.frames {
                self.data[self.geometry.offset(i, c)] = source.data[source.geometry.offset(i, c)];
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_retains_parent_strides_and_writes_only_borrowed_samples() {
        let planar = [1., 2., 3., 4., 5., 6., 7., 8., 9., 10., 11., 12.];
        let source = Block::new(&planar, 3, Layout::Planar).unwrap();
        let sub = source.sub_block(1..4, 1..3).unwrap();
        assert_eq!(sub.channel(0), Some(&[6., 7., 8.][..]));
        assert_eq!(sub.frame(0), None);
        assert_eq!(
            sub.sub_block(1..3, 1..2).unwrap().channel(0),
            Some(&[11., 12.][..])
        );
        assert!(sub.sub_block(0..usize::MAX, 0..1).is_err());
        assert!(sub.sub_block(0..0, 0..0).is_err());
        assert_eq!(sub.sub_block(3..3, 0..2).unwrap().channel(0), Some(&[][..]));
        let mut interleaved = [0.; 16];
        let mut block = BlockMut::new(&mut interleaved, 4, Layout::Interleaved).unwrap();
        let mut destination = block.sub_block_mut(0..3, 1..3).unwrap();
        destination.copy_from(sub).unwrap();
        assert_eq!(destination.as_view().frame(1), Some(&[7., 11.][..]));
        assert_eq!(destination.as_view().channel(1), None);
        let mut nested = destination.sub_block_mut(2..3, 0..1).unwrap();
        *nested.sample_mut(0, 0).unwrap() = 88.;
        assert_eq!(
            interleaved,
            [0., 6., 10., 0., 0., 7., 11., 0., 0., 88., 12., 0., 0., 0., 0., 0.]
        );
    }
}
