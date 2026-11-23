//! First-order AmbiX (ACN/SN3D) encoding, rotation and binaural decoding.
use crate::{
    binaural::{Direction, HrtfSet},
    stereo_convolution::{ConvolutionMode, StereoConvolution},
};
use soniclaw_core::{AudioBuffer, AudioProcessor, Error, ProcessContext, ProcessSpec};

/// First-order AmbiX channels, ordered W, Y, Z, X (ACN 0–3).
pub type Frame = [f32; 4];

pub fn encode(sample: f32, azimuth_deg: f32, elevation_deg: f32) -> Result<Frame, Error> {
    if !sample.is_finite()
        || !(-360. ..=360.).contains(&azimuth_deg)
        || !(-90. ..=90.).contains(&elevation_deg)
    {
        return Err(Error::InvalidParameter);
    }
    let (azimuth_sin, azimuth_cos) = azimuth_deg.to_radians().sin_cos();
    let (elevation_sin, elevation_cos) = elevation_deg.to_radians().sin_cos();
    Ok([
        sample,
        sample * azimuth_sin * elevation_cos,
        sample * elevation_sin,
        sample * azimuth_cos * elevation_cos,
    ])
}

/// Right-handed active rotation, yaw around up, pitch around left, roll around front.
/// The yaw, pitch and roll values use degrees and compose as yaw × pitch × roll.
#[derive(Clone, Copy, Debug)]
pub struct Rotation([[f32; 3]; 3]);
impl Rotation {
    pub fn new(yaw: f32, pitch: f32, roll: f32) -> Result<Self, Error> {
        if [yaw, pitch, roll]
            .iter()
            .any(|x| !(-360. ..=360.).contains(x))
        {
            return Err(Error::InvalidParameter);
        }
        let (y_s, y_c) = yaw.to_radians().sin_cos();
        let (p_s, p_c) = pitch.to_radians().sin_cos();
        let (r_s, r_c) = roll.to_radians().sin_cos();
        Ok(Self([
            [
                y_c * p_c,
                y_c * p_s * r_s - y_s * r_c,
                y_c * p_s * r_c + y_s * r_s,
            ],
            [
                y_s * p_c,
                y_s * p_s * r_s + y_c * r_c,
                y_s * p_s * r_c - y_c * r_s,
            ],
            [-p_s, p_c * r_s, p_c * r_c],
        ]))
    }
    pub fn apply(self, [w, y, z, x]: Frame) -> Frame {
        let [next_x, next_y, next_z] = self.0.map(|row| row[0] * x + row[1] * y + row[2] * z);
        [w, next_y, next_z, next_x]
    }
}

/// Decode to six virtual speakers ordered front, back, left, right, up, down.
pub fn decode_six([w, y, z, x]: Frame) -> [f32; 6] {
    [x, -x, y, -y, z, -z].map(|value| w / 6. + value * 0.5)
}

const AXES: [(f32, f32); 6] = [
    (0., 0.),
    (180., 0.),
    (90., 0.),
    (-90., 0.),
    (0., 90.),
    (0., -90.),
];

pub struct HeadphoneDecoder {
    speakers: Vec<StereoConvolution>,
    scratch: Vec<f32>,
    max_frames: usize,
}

impl HeadphoneDecoder {
    /// Prepare six stationary virtual speakers with the given listener radius.
    pub fn new(dataset: &HrtfSet, distance_metres: f32, max_frames: usize) -> Result<Self, Error> {
        if !(1..=8192).contains(&max_frames) {
            return Err(Error::InvalidSpec);
        }
        let spec = ProcessSpec {
            sample_rate: f64::from(dataset.sample_rate()),
            channels: 2,
            max_block_frames: max_frames,
        };
        let speakers = AXES
            .into_iter()
            .map(|(azimuth, elevation)| {
                let impulse = dataset.impulse(Direction {
                    azimuth,
                    elevation,
                    distance_metres,
                })?;
                let mut convolution =
                    StereoConvolution::new(impulse, ConvolutionMode::ZeroLatency, 0)?;
                convolution.prepare(spec)?;
                Ok(convolution)
            })
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            speakers,
            scratch: vec![0.; 2 * max_frames],
            max_frames,
        })
    }

    /// Fill stereo output. Rotation, if needed, can be applied to input frames before decoding.
    pub fn process(&mut self, input: &[Frame], out: &mut AudioBuffer<'_>) -> Result<(), Error> {
        if out.channels() != 2 || out.frames() != input.len() || input.len() > self.max_frames {
            return Err(Error::InvalidBuffer);
        }
        if input
            .iter()
            .flatten()
            .any(|x| !x.is_finite() || x.abs() > 16.)
        {
            return Err(Error::InvalidParameter);
        }
        let context = ProcessContext {
            events: &[],
            transport: Default::default(),
        };
        out.clear();
        let data = &mut self.scratch[..2 * input.len()];
        for (index, speaker) in self.speakers.iter_mut().enumerate() {
            for (&source, frame) in input.iter().zip(data.as_chunks_mut::<2>().0) {
                frame.fill(decode_six(source)[index]);
            }
            speaker.process(&mut AudioBuffer::new(data, 2)?, &context);
            for (y, x) in out.samples_mut().iter_mut().zip(data.iter()) {
                *y += *x;
            }
        }
        Ok(())
    }
    pub fn reset(&mut self) {
        self.speakers.iter_mut().for_each(StereoConvolution::reset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn yaw_maps_front_to_left_and_preserves_omni() {
        let input = encode(1., 0., 0.).unwrap();
        let output = Rotation::new(90., 0., 0.).unwrap().apply(input);
        for (a, b) in output.into_iter().zip(encode(1., 90., 0.).unwrap()) {
            assert!((a - b).abs() < 1e-6);
        }
        let speakers = decode_six(output);
        assert!((speakers.iter().sum::<f32>() - 1.).abs() < 1e-6);
        assert!((speakers[2] - speakers[3] - 1.).abs() < 1e-6);
        assert!((speakers[0] - speakers[1]).abs() < 1e-6);
    }
}
