//! Prepared inference for classic mono Neural Amp Modeler WaveNet captures.
use serde::Deserialize;
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

const WIDTH: usize = 32;
const TAPS: usize = 8;

#[derive(Deserialize)]
struct File {
    version: String,
    architecture: String,
    config: Config,
    weights: Vec<f32>,
    sample_rate: Option<f64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    layers: Vec<Shape>,
    head: Option<serde_json::Value>,
    head_scale: Option<f32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Shape {
    input_size: usize,
    condition_size: usize,
    channels: usize,
    head_size: usize,
    kernel_size: usize,
    dilations: Vec<usize>,
    activation: String,
    gated: bool,
    head_bias: bool,
}

fn take(weights: &mut &[f32], count: usize) -> Result<Vec<f32>, Error> {
    let (part, rest) = weights
        .split_at_checked(count)
        .ok_or(Error::InvalidParameter)?;
    *weights = rest;
    Ok(part.to_vec())
}
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

struct Layer {
    conv: Vec<f32>,
    bias: Vec<f32>,
    condition: Vec<f32>,
    residual: Vec<f32>,
    residual_bias: Vec<f32>,
    history: Vec<f32>,
    cursor: usize,
    dilation: usize,
    taps: usize,
}
impl Layer {
    fn new(shape: &Shape, dilation: usize, weights: &mut &[f32]) -> Result<Self, Error> {
        let n = shape.channels;
        let k = shape.kernel_size;
        let raw = take(weights, n * n * k)?;
        let mut conv = vec![0.; raw.len()];
        for out in 0..n {
            for input in 0..n {
                for tap in 0..k {
                    conv[out * n * k + tap * n + input] = raw[(out * n + input) * k + tap];
                }
            }
        }
        Ok(Self {
            conv,
            bias: take(weights, n)?,
            condition: take(weights, n)?,
            residual: take(weights, n * n)?,
            residual_bias: take(weights, n)?,
            history: vec![0.; n * (1 + (k - 1) * dilation)],
            cursor: 0,
            dilation,
            taps: k,
        })
    }

    fn tick(&mut self, signal: &mut [f32], skip: &mut [f32], input: f32) {
        let width = signal.len();
        let frames = self.history.len() / width;
        self.history[self.cursor * width..(self.cursor + 1) * width].copy_from_slice(signal);
        let mut window = [0.; WIDTH * TAPS];
        for (tap, row) in window[..width * self.taps]
            .chunks_exact_mut(width)
            .enumerate()
        {
            let frame = (self.cursor + frames - (self.taps - 1 - tap) * self.dilation) % frames;
            row.copy_from_slice(&self.history[frame * width..(frame + 1) * width]);
        }
        self.cursor = (self.cursor + 1) % frames;
        let mut activated = [0.; WIDTH];
        let window = &window[..width * self.taps];
        for (i, row) in self.conv.chunks_exact(window.len()).enumerate() {
            let value = (dot(row, window) + self.bias[i] + self.condition[i] * input).tanh();
            activated[i] = value;
            skip[i] += value;
        }
        for (i, (sample, row)) in signal
            .iter_mut()
            .zip(self.residual.chunks_exact(width))
            .enumerate()
        {
            *sample += dot(row, &activated[..width]) + self.residual_bias[i];
        }
    }

    fn reset(&mut self) {
        self.history.fill(0.);
        self.cursor = 0;
    }
}

struct Stack {
    width: usize,
    input_width: usize,
    projection: Vec<f32>,
    layers: Vec<Layer>,
    head: Vec<f32>,
    head_bias: Vec<f32>,
}
pub struct NamWaveNet {
    stacks: Vec<Stack>,
    scale: f32,
    sample_rate: f64,
    receptive_field: usize,
    spec: Option<ProcessSpec>,
}

impl NamWaveNet {
    pub fn from_json(data: &[u8]) -> Result<Self, Error> {
        if data.len() > 4 * 1024 * 1024 {
            return Err(Error::InvalidBuffer);
        }
        let file: File = serde_json::from_slice(data).map_err(|_| Error::InvalidParameter)?;
        let rate = file.sample_rate.unwrap_or(48_000.);
        if file.architecture != "WaveNet"
            || !matches!(
                file.version.as_str(),
                "0.5.0" | "0.5.1" | "0.5.2" | "0.5.3" | "0.5.4" | "0.6.0" | "0.7.0"
            )
            || !(8000. ..=192_000.).contains(&rate)
            || !(1..=3).contains(&file.config.layers.len())
            || file.config.head.is_some()
            || file.config.head_scale.is_some_and(|x| x.abs() > 100.)
            || file
                .weights
                .iter()
                .any(|x| !x.is_finite() || x.abs() > 100.)
        {
            return Err(Error::InvalidParameter);
        }
        let mut field = 1;
        let mut expected_input = 1;
        let mut expected_skip = None;
        for shape in &file.config.layers {
            if shape.input_size != expected_input
                || shape.condition_size != 1
                || expected_skip.is_some_and(|width| width != shape.channels)
                || !(1..=WIDTH).contains(&shape.channels)
                || !(1..=WIDTH).contains(&shape.head_size)
                || !(1..=TAPS).contains(&shape.kernel_size)
                || !(1..=16).contains(&shape.dilations.len())
                || shape.activation != "Tanh"
                || shape.gated
                || shape.dilations.iter().any(|d| !(1..=4096).contains(d))
            {
                return Err(Error::InvalidParameter);
            }
            field += shape.dilations.iter().sum::<usize>() * (shape.kernel_size - 1);
            expected_input = shape.channels;
            expected_skip = Some(shape.head_size);
        }
        if expected_skip != Some(1) || field > 65_536 {
            return Err(Error::InvalidParameter);
        }
        let mut weights = file.weights.as_slice();
        let mut stacks = Vec::with_capacity(file.config.layers.len());
        for shape in file.config.layers {
            let width = shape.channels;
            let projection = take(&mut weights, width * shape.input_size)?;
            let layers = shape
                .dilations
                .iter()
                .map(|&d| Layer::new(&shape, d, &mut weights))
                .collect::<Result<_, _>>()?;
            let head = take(&mut weights, width * shape.head_size)?;
            let head_bias = if shape.head_bias {
                take(&mut weights, shape.head_size)?
            } else {
                vec![0.; shape.head_size]
            };
            stacks.push(Stack {
                width,
                input_width: shape.input_size,
                projection,
                layers,
                head,
                head_bias,
            });
        }
        let [scale] = weights else {
            return Err(Error::InvalidParameter);
        };
        Ok(Self {
            stacks,
            scale: *scale,
            sample_rate: rate,
            receptive_field: field,
            spec: None,
        })
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
    pub fn receptive_field(&self) -> usize {
        self.receptive_field
    }

    fn tick(&mut self, input: f32) -> f32 {
        let input = if input.is_finite() {
            input.clamp(-16., 16.)
        } else {
            0.
        };
        let mut signal = [0.; WIDTH];
        let mut skip = [0.; WIDTH];
        signal[0] = input;
        for stack in &mut self.stacks {
            let mut projected = [0.; WIDTH];
            let signal_in = &signal[..stack.input_width];
            for (dest, row) in projected
                .iter_mut()
                .zip(stack.projection.chunks_exact(stack.input_width))
            {
                *dest = dot(row, signal_in);
            }
            signal = projected;
            for layer in &mut stack.layers {
                layer.tick(&mut signal[..stack.width], &mut skip[..stack.width], input);
            }
            let mut next = [0.; WIDTH];
            for ((dest, row), bias) in next
                .iter_mut()
                .zip(stack.head.chunks_exact(stack.width))
                .zip(&stack.head_bias)
            {
                *dest = dot(row, &skip[..stack.width]) + bias;
            }
            skip = next;
        }
        skip[0] * self.scale
    }
}

impl AudioProcessor for NamWaveNet {
    fn prepare(&mut self, spec: ProcessSpec) -> Result<(), Error> {
        spec.validate()?;
        if spec.channels != 1 || (spec.sample_rate - self.sample_rate).abs() > 0.001 {
            return Err(Error::InvalidSpec);
        }
        self.spec = Some(spec);
        self.reset();
        Ok(())
    }
    fn process(&mut self, buffer: &mut AudioBuffer<'_>, _: &ProcessContext<'_>) {
        if !self
            .spec
            .is_some_and(|s| buffer.channels() == 1 && buffer.frames() <= s.max_block_frames)
        {
            buffer.clear();
            return;
        }
        for sample in buffer.samples_mut() {
            *sample = self.tick(*sample);
        }
    }
    fn reset(&mut self) {
        for stack in &mut self.stacks {
            stack.layers.iter_mut().for_each(Layer::reset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_layer_delay_and_condition_have_expected_weights() {
        // Weight order: projection, two taps, bias, condition, residual and bias,
        // head and bias, final scale.
        let file = serde_json::json!({"version":"0.5.4","architecture":"WaveNet","config":{
            "layers":[{"input_size":1,"condition_size":1,"head_size":1,"channels":1,
            "kernel_size":2,"dilations":[2],"activation":"Tanh","gated":false,"head_bias":true}],
            "head":null},"weights":[1.,2.,3.,0.,4.,0.,0.,1.,0.,0.5]});
        let mut engine = NamWaveNet::from_json(&serde_json::to_vec(&file).unwrap()).unwrap();
        let spec = ProcessSpec {
            sample_rate: 48_000.,
            channels: 1,
            max_block_frames: 4,
        };
        engine.prepare(spec).unwrap();
        assert_eq!(engine.receptive_field(), 3);
        let mut x = [0.1, 0., 0., 0.];
        let allocations = allocation_counter::measure(|| {
            engine.process(
                &mut AudioBuffer::new(&mut x, 1).unwrap(),
                &super::super::advanced::context(),
            )
        });
        assert_eq!(allocations.count_total, 0);
        assert!((x[0] - 0.7f32.tanh() * 0.5).abs() < 1e-6);
        assert_eq!(x[1], 0.);
        assert!((x[2] - 0.2f32.tanh() * 0.5).abs() < 1e-6);
        engine.reset();
        assert_eq!(engine.tick(0.), 0.);
    }
}
