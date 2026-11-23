//! Same-pass routed render taps. Stems are captured before master inserts.
use crate::*;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub enum Source {
    TrackPost(String),
    Bus(String),
    Aux(usize),
}
#[derive(Clone)]
pub struct Tap {
    pub source: Source,
    pub latency_frames: usize,
    pub bank: Arc<ChannelBus>,
    pub enabled: Arc<std::sync::atomic::AtomicBool>,
}
impl Tap {
    pub fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }
    pub(crate) fn begin(&self) {
        if self.enabled.load(Ordering::Relaxed) {
            self.bank.begin();
        }
    }
    pub(crate) fn write(&self, data: &[f32], offset: usize) -> Result<(), Error> {
        if self.enabled.load(Ordering::Relaxed) {
            self.bank.write_at(offset, data)?;
        }
        Ok(())
    }
}
pub(crate) fn prepare(
    d: &Document,
    maximum: usize,
    track_latency: usize,
    aux_latency: usize,
    r: Option<&mix_graph::Runtime>,
) -> Result<Vec<Tap>, Error> {
    let sources = d
        .tracks
        .iter()
        .map(|t| (Source::TrackPost(t.id.clone()), track_latency))
        .chain(
            d.mix_routing
                .buses
                .iter()
                .enumerate()
                .map(|(i, b)| (Source::Bus(b.id.clone()), r.map_or(0, |r| r.bus_latency(i)))),
        )
        .chain(
            d.auxiliaries
                .iter()
                .enumerate()
                .map(|(i, _)| (Source::Aux(i), track_latency + aux_latency)),
        );
    sources
        .map(|(source, latency_frames)| {
            Ok(Tap {
                source,
                latency_frames,
                bank: Arc::new(ChannelBus::new(2, maximum)?),
                enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            })
        })
        .collect()
}
