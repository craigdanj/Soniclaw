use soniclaw_core::{AudioBuffer,AudioProcessor,ProcessContext,ProcessSpec};
use soniclaw_dsp::nam_wavenet::NamWaveNet;
fn main() -> Result<(),Box<dyn std::error::Error>>{
    let args:Vec<_>=std::env::args().collect();
    let data=std::fs::read(&args[1])?;
    let mut model=NamWaveNet::from_json(&data)?;
    let sample_rate=model.sample_rate();
    model.prepare(ProcessSpec{channels:1,sample_rate,max_block_frames:128})?;
    let context=ProcessContext{events:&[],transport:Default::default()};
    let source:Vec<_>=(0..96_000).map(|i| (0.27*(std::f64::consts::TAU*995.*f64::from(i)/sample_rate).sin()) as f32).collect();
    let mut measured=source.clone();
    let clock=std::time::Instant::now();
    for samples in measured.chunks_mut(128){
        model.process(&mut AudioBuffer::new(samples,1)?,&context);
    }
    let elapsed=clock.elapsed().as_secs_f64();
    for (name, array) in [("input.f32le",source),("rust.f32le",measured)]{
        std::fs::write(std::path::Path::new(&args[2]).join(name),array.iter().flat_map(|x|x.to_le_bytes()).collect::<Vec<_>>())?;
    }
    println!("{elapsed} {sample_rate} {}",model.receptive_field());
    Ok(())
}
