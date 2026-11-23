use soniclaw::{
    core::*,
    midi::{controllers::*, output::*, wire::Packet},
};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
struct Virtual(mpsc::Sender<Vec<u8>>);
impl Sink for Virtual {
    fn send(&mut self, p: &Packet) -> Result<(), OutputError> {
        self.0.send(p.bytes().to_vec())?;
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: soniclaw-controller-demo NEW_DIRECTORY")?,
    );
    std::fs::create_dir(&path)?;
    let p = Arc::new(Parameter::new(ParameterInfo {
        id: "gain",
        name: "Gain",
        min: 0.,
        max: 2.,
        default: 1.,
        unit: "",
    })?);
    let profile = Profile {
        schema: 1,
        echo_window_ms: 10.,
        banks: vec![
            Bank {
                name: "Faders".into(),
                mappings: vec![Mapping {
                    channel: 0,
                    controller: 7,
                    parameter: "gain".into(),
                    mode: Mode::Absolute {
                        pickup: true,
                        tolerance: 0.01,
                    },
                    step: 0.01,
                    feedback: Some(Feedback {
                        channel: 0,
                        controller: 7,
                    }),
                }],
            },
            Bank {
                name: "Encoders".into(),
                mappings: vec![Mapping {
                    channel: 0,
                    controller: 7,
                    parameter: "gain".into(),
                    mode: Mode::TwosComplement,
                    step: 0.01,
                    feedback: Some(Feedback {
                        channel: 0,
                        controller: 7,
                    }),
                }],
            },
        ],
    };
    std::fs::write(path.join("controller.json"), profile.to_json()?)?;
    let mut c = profile.compile(std::slice::from_ref(&p), 48000.)?;
    let (tx, rx) = mpsc::channel();
    let (mut worker, mut sender) = OutputWorker::start(
        16,
        Clock::new(48000., 0, Instant::now() + Duration::from_millis(10))?,
        move || Ok(Box::new(Virtual(tx))),
    )?;
    assert_eq!(
        c.feedback(0, |bytes| sender
            .send(0, Packet::new(&bytes).unwrap())
            .is_ok())?,
        1
    );
    let cc = |value| MidiMessage::ControlChange {
        channel: 0,
        controller: 7,
        value,
    };
    assert!(!c.apply(1, cc(64))?);
    assert!(!c.apply(1000, cc(0))?);
    assert!(c.apply(1001, cc(80))?);
    p.set(0.4)?;
    assert!(!c.apply(1002, cc(100))?);
    assert!(c.apply(1003, cc(20))?);
    c.select_bank(1)?;
    assert!(c.apply(2000, cc(1))?);
    let final_gain = p.get();
    assert_eq!(
        c.feedback(2001, |bytes| sender
            .send(2001, Packet::new(&bytes).unwrap())
            .is_ok())?,
        1
    );
    let packets = [
        rx.recv_timeout(Duration::from_secs(1))?,
        rx.recv_timeout(Duration::from_secs(1))?,
    ];
    worker.stop();
    std::fs::write(
        path.join("report.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"virtual_output_packets":packets,"final_gain":final_gain,"bank":c.bank(),"physical_hardware_tested":false}),
        )?,
    )?;
    println!("Pickup, external-change rearm, bank switch, relative encoder and scheduled virtual MIDI feedback passed");
    Ok(())
}
