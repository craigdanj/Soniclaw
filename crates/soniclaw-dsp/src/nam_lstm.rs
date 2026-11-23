//! Inference for a bounded subset of Neural Amp Modeler LSTM files.
use serde::Deserialize;
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shape {
    num_layers: usize,
    input_size: usize,
    hidden_size: usize,
    #[serde(default = "mono")]
    in_channels: usize,
    #[serde(default = "mono")]
    out_channels: usize,
}
fn mono() -> usize {
    1
}
#[derive(Deserialize)]
struct File {
    version: String,
    architecture: String,
    config: Shape,
    weights: Vec<f32>,
    sample_rate: Option<f64>,
}
struct Layer {
    width: usize,
    weights: Vec<f32>,
    bias: Vec<f32>,
    saved_h: Vec<f32>,
    saved_cell: Vec<f32>,
    hidden: Vec<f32>,
    cell: Vec<f32>,
}
impl Layer {
    fn tick(&mut self, input: &[f32]) {
        let n = self.hidden.len();
        let stride = self.width + n;
        let mut gates = [0.; 256];
        for ((y, row), &bias) in gates
            .iter_mut()
            .zip(self.weights.chunks_exact(stride))
            .zip(&self.bias)
        {
            *y = row
                .iter()
                .zip(input.iter().chain(&self.hidden))
                .map(|(w, x)| w * x)
                .sum::<f32>()
                + bias;
        }
        let sigmoid = |x: f32| 1. / (1. + (-x).exp());
        for (i, (c, h)) in self.cell.iter_mut().zip(&mut self.hidden).enumerate() {
            *c = sigmoid(gates[n + i]) * *c + sigmoid(gates[i]) * gates[2 * n + i].tanh();
            *h = sigmoid(gates[3 * n + i]) * c.tanh();
        }
    }
    fn reset(&mut self) {
        self.hidden.copy_from_slice(&self.saved_h);
        self.cell.copy_from_slice(&self.saved_cell);
    }
}
pub struct NamLstm {
    layers: Vec<Layer>,
    head: Vec<f32>,
    bias: f32,
    rate: f64,
    in_gain: f32,
    out_gain: f32,
    spec: Option<ProcessSpec>,
}
impl NamLstm {
    /// Parse on the control thread. Missing sample_rate is interpreted as 48 kHz.
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        let file: File = serde_json::from_slice(bytes).map_err(|_| Error::InvalidParameter)?;
        let Shape {
            num_layers,
            input_size,
            hidden_size: n,
            in_channels,
            out_channels,
        } = file.config;
        let rate = file.sample_rate.unwrap_or(48000.);
        let version_ok = matches!(
            file.version.as_str(),
            "0.5.0" | "0.5.1" | "0.5.2" | "0.5.3" | "0.5.4" | "0.7.0"
        );
        if !version_ok
            || file.architecture != "LSTM"
            || !(1..=3).contains(&num_layers)
            || input_size != 1
            || in_channels != 1
            || out_channels != 1
            || !(1..=64).contains(&n)
            || !(8000. ..=192000.).contains(&rate)
            || file
                .weights
                .iter()
                .any(|x| !x.is_finite() || x.abs() > 100.)
        {
            return Err(Error::InvalidParameter);
        }
        let expected = 4 * n * (1 + n) + 6 * n + (num_layers - 1) * (8 * n * n + 6 * n) + n + 1;
        if file.weights.len() != expected {
            return Err(Error::InvalidParameter);
        }
        let mut cursor = 0;
        let mut take = |count: usize| {
            let v = file.weights[cursor..cursor + count].to_vec();
            cursor += count;
            v
        };
        let mut layers = Vec::with_capacity(num_layers);
        for i in 0..num_layers {
            let width = if i == 0 { 1 } else { n };
            let weights = take(4 * n * (width + n));
            let bias = take(4 * n);
            let saved_h = take(n);
            let saved_cell = take(n);
            layers.push(Layer {
                width,
                weights,
                bias,
                hidden: saved_h.clone(),
                cell: saved_cell.clone(),
                saved_h,
                saved_cell,
            });
        }
        let head = take(n);
        let bias = take(1)[0];
        Ok(Self {
            layers,
            head,
            bias,
            rate,
            in_gain: 1.,
            out_gain: 1.,
            spec: None,
        })
    }
    pub fn sample_rate(&self) -> f64 {
        self.rate
    }
    pub fn set_calibration(&mut self, input_db: f32, output_db: f32) -> Result<(), Error> {
        if !(-36. ..=24.).contains(&input_db) || !(-36. ..=24.).contains(&output_db) {
            return Err(Error::InvalidParameter);
        }
        self.in_gain = crate::db_to_gain(input_db);
        self.out_gain = crate::db_to_gain(output_db);
        Ok(())
    }
    /// Process a model-rate mono sample. Parse and prepare off the audio thread.
    pub fn tick(&mut self, input: f32) -> f32 {
        let input = [if input.is_finite() {
            (input * self.in_gain).clamp(-16., 16.)
        } else {
            0.
        }];
        let (first, rest) = self.layers.split_first_mut().expect("validated layers");
        first.tick(&input);
        let mut hidden = first.hidden.as_slice();
        for l in rest {
            l.tick(hidden);
            hidden = &l.hidden;
        }
        (self.bias
            + self
                .head
                .iter()
                .zip(hidden)
                .map(|(w, x)| w * x)
                .sum::<f32>())
            * self.out_gain
    }
    /// Explicitly warm model state with silence, outside the audio callback.
    pub fn prewarm(&mut self, frames: usize) -> Result<(), Error> {
        if frames > 192000 {
            return Err(Error::InvalidParameter);
        }
        for _ in 0..frames {
            self.tick(0.);
        }
        Ok(())
    }
}
impl AudioProcessor for NamLstm {
    fn prepare(&mut self, s: ProcessSpec) -> Result<(), Error> {
        s.validate()?;
        if s.channels != 1 || (s.sample_rate - self.rate).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        self.reset();
        self.spec = Some(s);
        Ok(())
    }
    fn process(&mut self, b: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        let Some(s) = self.spec else {
            b.clear();
            return;
        };
        if b.channels() != 1 || b.frames() > s.max_block_frames {
            b.clear();
            return;
        }
        for x in b.samples_mut() {
            *x = self.tick(*x);
        }
    }
    fn reset(&mut self) {
        for l in &mut self.layers {
            l.reset();
        }
    }
}
