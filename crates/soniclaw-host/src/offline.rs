//! Adapt an already active scoped CLAP audio lease to the generic offline renderer.
use super::*;
pub struct ScopedProcessor<'a, 'plugin> {
    audio: &'a mut HostedAudio<'plugin>,
    latency: usize,
    error: Option<Error>,
}
impl<'plugin> HostedAudio<'plugin> {
    /// `latency` is the value queried on the owner thread after activation.
    /// The returned adapter cannot outlive the run_audio lease.
    pub fn processor(&mut self, latency: usize) -> ScopedProcessor<'_, 'plugin> {
        ScopedProcessor {
            audio: self,
            latency,
            error: None,
        }
    }
}
impl ScopedProcessor<'_, '_> {
    pub fn error(&self) -> Option<Error> {
        self.error
    }
}
impl AudioProcessor for ScopedProcessor<'_, '_> {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != self.audio.spec.channels
            || s.sample_rate != self.audio.spec.sample_rate
            || s.max_block_frames > self.audio.spec.max_block_frames
            || !self.audio.processing
        {
            return Err(Error::InvalidSpec);
        }
        Ok(())
    }
    fn reset(&mut self) {
        unsafe {
            if let Some(reset) = (*self.audio.plugin).reset {
                reset(self.audio.plugin);
            }
        }
        self.error = None;
    }
    fn latency_frames(&self) -> usize {
        self.latency
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, ctx: &ProcessContext<'_>) {
        if let Err(e) = self
            .audio
            .process_block(b.samples_mut(), ctx.events, &[], ctx.transport)
        {
            b.clear();
            self.error = Some(e);
        }
    }
}
