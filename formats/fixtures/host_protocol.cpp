// SPDX-License-Identifier: MIT OR Apache-2.0
// Independent native VST3 host protocol fixture; no Soniclaw/CLAP processing code.
#include "public.sdk/source/vst/vstsinglecomponenteffect.h"
#include "public.sdk/source/main/pluginfactory.h"
#include "pluginterfaces/vst/ivstevents.h"
#include "pluginterfaces/vst/ivstparameterchanges.h"
#include "pluginterfaces/vst/ivstmidicontrollers.h"
#include <algorithm>
#include <cstdlib>
#include <thread>
#include <chrono>
#include <cmath>
#include "pluginterfaces/base/ibstream.h"
using namespace Steinberg;
using namespace Steinberg::Vst;
static const FUID FixtureID(0x53434C57,0x48535450,0x524F5431,0x00000001);
class Fixture;
static Fixture* editorFixture=nullptr;
class Fixture final : public SingleComponentEffect, public IMidiMapping {
 double gain=0.5;
 double fault=0.;
public:
 Fixture(){editorFixture=this;processContextRequirements.needTempo().needTimeSignature().needProjectTimeMusic().needBarPositionMusic().needCycleMusic().needTransportState();}
 ~Fixture() override {if(editorFixture==this)editorFixture=nullptr;}
 bool editorEdit(ParamID id,double value){if(!componentHandler || id!=9 || value<0. || value>1.)return false;componentHandler->beginEdit(id);setParamNormalized(id,value);componentHandler->performEdit(id,value);componentHandler->endEdit(id);return true;}
 static FUnknown* create(void*) { return static_cast<IAudioProcessor*>(new Fixture); }
 tresult PLUGIN_API initialize(FUnknown* context) override {
  auto r=SingleComponentEffect::initialize(context);if(r!=kResultOk)return r;
  addAudioInput(STR16("Main"),SpeakerArr::kStereo);addAudioOutput(STR16("Main"),SpeakerArr::kStereo);
  if(std::getenv("SONICLAW_FIXTURE_BUSES")){addAudioInput(STR16("Key"),SpeakerArr::kMono);addAudioOutput(STR16("Separate"),SpeakerArr::kStereo);}
  addEventInput(STR16("MIDI"));addEventOutput(STR16("Events"));
  parameters.addParameter(STR16("Gain"),nullptr,0,0.5,ParameterInfo::kCanAutomate,9);
  parameters.addParameter(STR16("Program"),nullptr,127,0,ParameterInfo::kIsProgramChange|ParameterInfo::kCanAutomate,10);
  parameters.addParameter(STR16("Fault mode"),nullptr,2,0,ParameterInfo::kCanAutomate,11);
  for(int i=600;i<=609;i++)parameters.addParameter(STR16("Snapshot"),nullptr,0,0,ParameterInfo::kIsReadOnly,i);
  return kResultOk;
 }
 tresult PLUGIN_API queryInterface(const TUID iid, void** obj) override {
  QUERY_INTERFACE(iid,obj,IMidiMapping::iid,IMidiMapping)
  return SingleComponentEffect::queryInterface(iid,obj);
 }
 uint32 PLUGIN_API addRef() override {return SingleComponentEffect::addRef();}
 uint32 PLUGIN_API release() override {return SingleComponentEffect::release();}
 tresult PLUGIN_API getMidiControllerAssignment(int32 bus,int16 channel,CtrlNumber number,ParamID& id) override {
  if(bus==0 && channel>=0 && channel<16 && number==kCtrlProgramChange){id=10;return kResultTrue;}return kResultFalse;
 }
 tresult PLUGIN_API getState(IBStream* stream) override {
  if(!stream)return kInvalidArgument;
  double values[2]={gain,fault};int32 written=0;
  return stream->write(values,sizeof(values),&written)==kResultOk&&written==sizeof(values)?kResultOk:kResultFalse;
 }
 tresult PLUGIN_API setState(IBStream* stream) override {
  if(!stream)return kInvalidArgument;
  double values[2]={};int32 read=0;
  if(stream->read(values,sizeof(values),&read)!=kResultOk||read!=sizeof(values))return kResultFalse;
  for(double value:values)if(!std::isfinite(value)||value<0.||value>1.)return kResultFalse;
  gain=values[0];fault=values[1];setParamNormalized(9,gain);setParamNormalized(11,fault);return kResultOk;
 }
 tresult PLUGIN_API canProcessSampleSize(int32 size) override{return size==kSample32?kResultTrue:kResultFalse;}
 tresult PLUGIN_API process(ProcessData& d) override {
  if(d.processContext){auto& c=*d.processContext;setParamNormalized(600,c.tempo/1000.);setParamNormalized(601,c.projectTimeMusic/1024.);setParamNormalized(602,c.barPositionMusic/1024.);setParamNormalized(603,c.timeSigNumerator/64.);setParamNormalized(604,c.timeSigDenominator/64.);setParamNormalized(605,c.cycleStartMusic/1024.);setParamNormalized(606,c.cycleEndMusic/1024.);int flags=((c.state&ProcessContext::kPlaying)?1:0)|((c.state&ProcessContext::kRecording)?2:0)|((c.state&ProcessContext::kCycleActive)?4:0);setParamNormalized(607,flags/16.);}
  if(d.inputEvents){for(int i=0;i<d.inputEvents->getEventCount();i++){Event e{};if(d.inputEvents->getEvent(i,e)!=kResultOk)continue;if(e.type==Event::kNoteExpressionValueEvent)setParamNormalized(608,e.noteExpressionValue.value);if(e.type==Event::kDataEvent)setParamNormalized(609,1.);if(d.outputEvents)d.outputEvents->addEvent(e);}}
  for(int i=0;i<d.numSamples;i++){
   if(d.inputParameterChanges)for(int q=0;q<d.inputParameterChanges->getParameterCount();q++){auto* queue=d.inputParameterChanges->getParameterData(q);if(!queue)continue;for(int j=0;j<queue->getPointCount();j++){int32 off;ParamValue v;if(queue->getPoint(j,off,v)==kResultOk&&off==i){if(queue->getParameterId()==9)gain=v;if(queue->getParameterId()==11)fault=v;setParamNormalized(queue->getParameterId(),v);}}}
   if(fault>=0.75)std::this_thread::sleep_for(std::chrono::seconds(2));else if(fault>=0.25)std::abort();
   if(d.numOutputs>0)for(int c=0;c<d.outputs[0].numChannels;c++)d.outputs[0].channelBuffers32[c][i]=(d.numInputs>0?d.inputs[0].channelBuffers32[c][i]:0.f)*float(gain)*(d.numInputs>1&&d.inputs[1].channelBuffers32[0][i]>0.f?0.5f:1.f);
   if(d.numOutputs>1)for(int c=0;c<d.outputs[1].numChannels;c++)d.outputs[1].channelBuffers32[c][i]=(d.numInputs>0?d.inputs[0].channelBuffers32[c][i]:0.f)*float(gain)*2.f;
  }
  return kResultOk;
 }
};
BEGIN_FACTORY_DEF("Soniclaw", "https://example.invalid", "")
 DEF_CLASS2(INLINE_UID_FROM_FUID(FixtureID),PClassInfo::kManyInstances,kVstAudioEffectClass,"Soniclaw Host Protocol Fixture",Vst::kDistributable,"Fx","1.0.0",kVstVersionString,Fixture::create)
END_FACTORY

bool InitModule(){return true;}
bool DeinitModule(){return true;}

#if defined(_WIN32)
#define SONICLAW_EXPORT __declspec(dllexport)
#else
#define SONICLAW_EXPORT __attribute__((visibility("default")))
#endif
// Owned-fixture main-thread editor gesture seam, no graphical window required.
extern "C" SONICLAW_EXPORT bool soniclaw_fixture_editor_edit(unsigned id,double value){return editorFixture&&editorFixture->editorEdit(id,value);}
