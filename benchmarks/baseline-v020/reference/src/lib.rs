use sonora_core::*;
use sonora_dsp::*;
use sonora_engine::Engine;
use sonora_plugin::{PluginDescriptor,PluginInstance};
use sonora_clap::{descriptor,Definition};
use std::sync::Arc;
pub fn parameter(id:&'static str,name:&'static str,min:f32,max:f32,default:f32,unit:&'static str)->Arc<Parameter>{Arc::new(Parameter::new(ParameterInfo{id,name,min,max,default,unit}).unwrap())}
struct Reference{mode:Arc<Parameter>,gain:Gain,filter:LowPass}
impl AudioProcessor for Reference{
 fn prepare(&mut self,s:ProcessSpec)->Result<(),Error>{self.gain.prepare(s)?;self.filter.prepare(s)}
 fn process(&mut self,b:&mut AudioBuffer<'_>,c:&ProcessContext<'_>){let mode=self.mode.get().round()as u8;if mode==2{self.filter.process(b,c);}if mode!=0{self.gain.process(b,c);}}
 fn reset(&mut self){self.gain.reset();self.filter.reset();}
}
fn description(id:&'static str,name:&'static str,instrument:bool)->PluginDescriptor{PluginDescriptor{id,name,vendor:"Sonora",version:env!("CARGO_PKG_VERSION"),is_instrument:instrument,accepts_midi:instrument}}
pub fn create_reference()->PluginInstance{
 let gain=parameter("master.gain","Gain",0.0,1.0,0.5,"");let cutoff=parameter("filter.cutoff","Cutoff",20.0,20000.0,3000.0," Hz");let mode=parameter("effect.mode","Mode",0.0,2.0,2.0,"");
 let mut engine=Engine::default();engine.push(Reference{mode:mode.clone(),gain:Gain::new(gain.clone()),filter:LowPass::new(cutoff.clone())});
 PluginInstance::new(description("org.sonora.reference","Sonora Gain + Filter",false),engine,vec![mode,gain,cutoff]).unwrap()
}

static REFERENCE:sonora_clap::ClapDescriptor=descriptor(c"org.sonora.reference",c"Sonora Gain + Filter",c"Sonora",c"0.2.0",false);
sonora_clap::export_clap!([Definition{descriptor:&REFERENCE,create:create_reference,instrument:false,tail_seconds:1.0}]);
