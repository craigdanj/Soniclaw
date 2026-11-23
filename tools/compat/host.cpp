// Independent Linux CLAP host. No Soniclaw headers or internal APIs are used.
#include <clap/clap.h>
#include <X11/Xlib.h>
#include <dlfcn.h>
#include <sched.h>
#include <time.h>
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <random>
#include <stdexcept>
#include <thread>
#include <vector>
static void check(bool ok,const char*msg){if(!ok)throw std::runtime_error(msg);}
struct HostData {std::atomic<bool> callback{false},flush{false};Display* display=nullptr;Window parent=0;};
static void wake(const clap_host*h){static_cast<HostData*>(h->host_data)->callback=true;}
static void request_flush(const clap_host*h){static_cast<HostData*>(h->host_data)->flush=true;}
static const clap_host_params hostparams{nullptr,nullptr,request_flush};
static bool resize(const clap_host*h,uint32_t w,uint32_t height){auto&s=*static_cast<HostData*>(h->host_data);if(!s.display||!s.parent)return false;XResizeWindow(s.display,s.parent,w,height);XFlush(s.display);return true;}
static const clap_host_gui hostgui{nullptr,resize,nullptr,nullptr,nullptr};
static const void* extension(const clap_host*,const char* id){if(!strcmp(id,CLAP_EXT_PARAMS))return &hostparams;if(!strcmp(id,CLAP_EXT_GUI))return &hostgui;return nullptr;}
struct Events {std::array<const clap_event_header*,8> events{};uint32_t n=0;clap_input_events in{this,[](const clap_input_events*l)->uint32_t{return static_cast<Events*>(l->ctx)->n;},[](const clap_input_events*l,uint32_t n)->const clap_event_header*{auto&e=*static_cast<Events*>(l->ctx);return n<e.n?e.events[n]:nullptr;}};};
struct OutputEvents{unsigned begin=0,values=0,end=0,budget=0xffffffffu;clap_output_events out{this,[](const clap_output_events*l,const clap_event_header*h)->bool{auto&s=*static_cast<OutputEvents*>(l->ctx);if(s.budget==0)return false;--s.budget;if(h->type==CLAP_EVENT_PARAM_GESTURE_BEGIN)++s.begin;if(h->type==CLAP_EVENT_PARAM_VALUE)++s.values;if(h->type==CLAP_EVENT_PARAM_GESTURE_END)++s.end;return true;}};};
static clap_event_param_value value(clap_id id,double v,uint32_t time=0){return{{sizeof(clap_event_param_value),time,CLAP_CORE_EVENT_SPACE_ID,CLAP_EVENT_PARAM_VALUE,0},id,nullptr,-1,-1,-1,-1,v};}
struct State {std::vector<uint8_t> bytes;size_t position=0;clap_ostream writer{this,[](const clap_ostream*s,const void*p,uint64_t n)->int64_t{auto&v=*static_cast<State*>(s->ctx);size_t count=std::min<uint64_t>(n,7);auto*b=static_cast<const uint8_t*>(p);v.bytes.insert(v.bytes.end(),b,b+count);return count;}};clap_istream reader{this,[](const clap_istream*s,void*p,uint64_t n)->int64_t{auto&v=*static_cast<State*>(s->ctx);size_t count=std::min({static_cast<size_t>(n),size_t(5),v.bytes.size()-v.position});memcpy(p,v.bytes.data()+v.position,count);v.position+=count;return count;}};};
struct Loaded {
 void*lib=nullptr;const clap_plugin_entry*entry=nullptr;const clap_plugin_factory*factory=nullptr;HostData data;clap_host host{CLAP_VERSION,&data,"Soniclaw compatibility host","Soniclaw","","0.7.0",extension,wake,wake,wake};
 explicit Loaded(const char*path){lib=dlopen(path,RTLD_NOW|RTLD_LOCAL);if(!lib)throw std::runtime_error(dlerror());entry=static_cast<const clap_plugin_entry*>(dlsym(lib,"clap_entry"));check(entry&&entry->init(path),"entry init");factory=static_cast<const clap_plugin_factory*>(entry->get_factory(CLAP_PLUGIN_FACTORY_ID));check(factory,"factory");}
 ~Loaded(){if(entry)entry->deinit();if(lib)dlclose(lib);}
};
static double thread_ns(){timespec t{};clock_gettime(CLOCK_THREAD_CPUTIME_ID,&t);return t.tv_sec*1e9+t.tv_nsec;}
using Begin=void(*)();using End=uint64_t(*)();
int main(int argc,char**argv){try{
 check(XInitThreads()!=0,"Xlib thread initialization");check(argc>=3,"usage: host library.clap results-directory [--gui|--gui-only]");bool gui=argc>3&&(!strcmp(argv[3],"--gui")||!strcmp(argv[3],"--gui-only"));bool timing=!(argc>3&&!strcmp(argv[3],"--gui-only"));std::filesystem::create_directories(argv[2]);
 auto begin=reinterpret_cast<Begin>(dlsym(RTLD_DEFAULT,"soniclaw_alloc_begin"));auto end=reinterpret_cast<End>(dlsym(RTLD_DEFAULT,"soniclaw_alloc_end"));auto frees=reinterpret_cast<End>(dlsym(RTLD_DEFAULT,"soniclaw_free_count"));check(begin&&end&&frees,"LD_PRELOAD alloc-hook.so is required");
 begin();void* volatile allocation=malloc(177);auto test=end();free(allocation);check(test>0,"allocation interposer self-test");
 Loaded lib(argv[1]);cpu_set_t available;CPU_ZERO(&available);sched_getaffinity(0,sizeof(available),&available);int selected=-1;for(int c=0;c<CPU_SETSIZE;++c)if(CPU_ISSET(c,&available)){selected=c;break;}int affinity=-1;if(selected>=0){cpu_set_t one;CPU_ZERO(&one);CPU_SET(selected,&one);affinity=sched_setaffinity(0,sizeof(one),&one);}
 std::ofstream csv(std::filesystem::path(argv[2])/"callback-batches.csv");csv<<"plugin,mode,frames,round,cpu_ns_per_callback,allocations,deallocations,callbacks\n";
 std::ofstream report(std::filesystem::path(argv[2])/"compatibility.txt");report<<"Linux shared VM; 48 kHz stereo; release libraries; no hardware latency measurement\n"<<(timing?"Thread CPU batch timing enabled\n":"GUI-only run; no timing/stress/heap callback measurements\n")<<"CPU affinity request: "<<selected<<" return "<<affinity<<"\n";
 for(uint32_t index=0;index<lib.factory->get_plugin_count(lib.factory);++index){
  auto*d=lib.factory->get_plugin_descriptor(lib.factory,index);auto*p=lib.factory->create_plugin(lib.factory,&lib.host,d->id);check(p&&p->init(p),"plugin init");auto*ports=static_cast<const clap_plugin_audio_ports*>(p->get_extension(p,CLAP_EXT_AUDIO_PORTS));auto*params=static_cast<const clap_plugin_params*>(p->get_extension(p,CLAP_EXT_PARAMS));auto*state=static_cast<const clap_plugin_state*>(p->get_extension(p,CLAP_EXT_STATE));check(ports&&params&&state,"required extensions");
  uint32_t inputs_count=ports->count(p,true);check(inputs_count<=2&&ports->count(p,false)==1,"bus counts");clap_audio_port_info pi{};if(inputs_count==2){check(ports->get(p,1,true,&pi)&&pi.id!=CLAP_INVALID_ID&&!(pi.flags&CLAP_AUDIO_PORT_IS_MAIN)&&pi.channel_count==2,"sidechain port contract");}
  clap_id mode=CLAP_INVALID_ID,gain=CLAP_INVALID_ID,cutoff=CLAP_INVALID_ID;
  for(uint32_t n=0;n<params->count(p);++n){clap_param_info info{};check(params->get_info(p,n,&info),"param info");if(!strcmp(info.name,"Mode"))mode=info.id;if(!strcmp(info.name,"Gain"))gain=info.id;if(!strcmp(info.name,"Cutoff"))cutoff=info.id;}
  check(!p->activate(p,0.0,1,1024),"invalid activation");check(p->activate(p,48000.0,1,1024)&&p->start_processing(p),"activation");
  std::array<float,1024> inL{},inR{},keyL{},keyR{},outL{},outR{};for(size_t n=0;n<1024;++n){inL[n]=0.25f*std::sin(n*0.07f);inR[n]=0.2f*std::cos(n*0.09f);keyL[n]=keyR[n]=1.0f;}
  float*ip[]{inL.data(),inR.data()},*kp[]{keyL.data(),keyR.data()},*op[]{outL.data(),outR.data()};clap_audio_buffer inputs[]{{ip,nullptr,2,0,0},{kp,nullptr,2,0,0}},output{op,nullptr,2,0,0};Events events;OutputEvents outgoing;clap_process c{0,256,nullptr,inputs_count?inputs:nullptr,&output,inputs_count,1,&events.in,&outgoing.out};
  auto set=[&](clap_id id,double v){if(id==CLAP_INVALID_ID)return;auto e=value(id,v);events.events[0]=&e.header;events.n=1;params->flush(p,&events.in,&outgoing.out);events.n=0;};
  if(mode!=CLAP_INVALID_ID){set(mode,0);check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"passthrough callback");for(unsigned n=0;n<c.frames_count;++n){check(outL[n]==inL[n]&&outR[n]==inR[n],"pass-through audio match");}
   // In-place and constant-channel aliases exercise the exported adapter.
   output.data32=ip;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"in-place callback");output.data32=op;inputs[0].constant_mask=3;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"constant mask callback");for(unsigned n=0;n<c.frames_count;++n)check(outL[n]==inL[0]&&outR[n]==inR[0],"constant channel expansion");inputs[0].constant_mask=0;
   set(mode,1);set(gain,0.5);p->reset(p);check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"gain callback");for(unsigned n=0;n<c.frames_count;++n)check(std::abs(outL[n]-inL[n]*0.5f)<1e-6f,"gain analytic check");
   auto e=value(gain,0.25,64);events.events[0]=&e.header;events.n=1;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"automation callback");check(std::abs(outL[63]-inL[63]*0.5f)<1e-6f,"pre-automation samples");events.n=0;
  }
  if(inputs_count==2){inL.fill(0.5f);inR.fill(0.5f);for(int n=0;n<40;++n)check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"ducker callback");check(outL[255]<0.15f,"sidechain actually ducks main audio");keyL.fill(0);keyR.fill(0);for(int n=0;n<400;++n)p->process(p,&c);check(outL[255]>0.49f,"sidechain release");}
  double expected_value=0.0;if(gain!=CLAP_INVALID_ID)check(params->get_value(p,gain,&expected_value),"read saved parameter");State saved;check(state->save(p,&saved.writer),"partial-stream state save");if(gain!=CLAP_INVALID_ID){set(gain,0.8);check(state->load(p,&saved.reader),"active state restore");p->process(p,&c);double v=0;check(params->get_value(p,gain,&v)&&std::abs(v-expected_value)<1e-5,"parameter state restoration");}
  State malformed;malformed.bytes={1,2,3};check(!state->load(p,&malformed.reader),"reject invalid state");
  // Reproducible shuffled benchmark tasks. Primary value is per-round mean CPU.
  struct Task{unsigned frames;int mode;};std::vector<Task>tasks;for(auto f:{64u,128u,256u,512u,1024u})for(int m=0;m<(mode!=CLAP_INVALID_ID?5:1);++m)tasks.push_back({f,m});std::mt19937 rng(20261115);
  if(timing)for(int round=0;round<9;++round){std::shuffle(tasks.begin(),tasks.end(),rng);for(auto task:tasks){c.frames_count=task.frames;set(mode,task.mode==3?1:task.mode==4?2:task.mode);set(gain,0.5);set(cutoff,3000);p->reset(p);
   std::array<clap_event_midi,3> notes{};clap_event_param_value automation{};auto process=[&](unsigned n){if(inputs_count==0){events.n=0;if(n%8==0){const uint8_t keys[]{36,38,42};for(unsigned k=0;k<3;++k){notes[k]={{sizeof(clap_event_midi),0,CLAP_CORE_EVENT_SPACE_ID,CLAP_EVENT_MIDI,0},0,{0x90,keys[k],110}};events.events[k]=&notes[k].header;}events.n=3;}}if(task.mode>=3){automation=value(task.mode==3?gain:cutoff,task.mode==3?(n%2?0.2:0.8):(n%2?2000:5000));events.events[0]=&automation.header;events.n=1;}check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"benchmark callback");};
   if(inputs_count==0){clap_event_midi note{{sizeof(clap_event_midi),0,CLAP_CORE_EVENT_SPACE_ID,CLAP_EVENT_MIDI,0},0,{0x90,36,110}};events.events[0]=&note.header;events.n=1;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"instrument note");events.n=0;}
   for(unsigned n=0;n<512;++n)process(n);double t=thread_ns();for(unsigned n=0;n<2048;++n)process(n);double pilot=(thread_ns()-t)/2048.0;unsigned count=static_cast<unsigned>(std::clamp(30000000.0/std::max(1.0,pilot),4096.0,200000.0));
   begin();t=thread_ns();for(unsigned n=0;n<count;++n)process(n);double cost=(thread_ns()-t)/count;auto allocations=end(),deallocations=frees();check(allocations==0&&deallocations==0,"heap operation during processing");csv<<d->id<<','<<task.mode<<','<<task.frames<<','<<round<<','<<cost<<','<<allocations<<','<<deallocations<<','<<count<<'\n';events.n=0;
  }}
  // Stress variable-length callbacks, including empty buffers, and reset state.
  if(timing)for(unsigned n=0;n<20000;++n){c.frames_count=n%1025;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"variable-block stress");if(n%1000==0)p->reset(p);}
  if(gui){
   auto*g=static_cast<const clap_plugin_gui*>(p->get_extension(p,CLAP_EXT_GUI));check(g,"GUI extension");
   check(!g->is_api_supported(p,"wayland",false)&&g->is_api_supported(p,"x11",false),"GUI API negotiation");check(!g->is_api_supported(p,"x11",true),"embedded-only GUI");
   check(g->create(p,"x11",false),"GUI create");uint32_t initial_w=0,initial_h=0;check(g->get_size(p,&initial_w,&initial_h)&&initial_w>=320,"size before parenting");
   Display*display=XOpenDisplay(nullptr);check(display,"X11 display");Window parent=XCreateSimpleWindow(display,DefaultRootWindow(display),10,10,640,360,0,0,WhitePixel(display,DefaultScreen(display)));lib.data.display=display;lib.data.parent=parent;XStoreName(display,parent,"Soniclaw compatibility parent");XMapWindow(display,parent);XSync(display,false);
   clap_window w{"x11",{}};w.x11=parent;check(g->set_parent(p,&w)&&g->show(p),"GUI parenting/show");
   void*xtst=dlopen("libXtst.so.6",RTLD_NOW);check(xtst,"XTest library required for --gui");
   auto motion=reinterpret_cast<int(*)(Display*,int,int,int,unsigned long)>(dlsym(xtst,"XTestFakeMotionEvent"));auto button=reinterpret_cast<int(*)(Display*,unsigned int,int,unsigned long)>(dlsym(xtst,"XTestFakeButtonEvent"));check(motion&&button,"XTest functions");
   set(gain,0.5);unsigned initial_values=outgoing.values,second_begin=0;bool restored_during_gesture=false;int knob_x=mode!=CLAP_INVALID_ID?144:50;
   for(int n=0;n<180;++n){
    if(lib.data.callback.exchange(false))p->on_main_thread(p);
    if(n==35){motion(display,DefaultScreen(display),knob_x,142,0);button(display,1,True,0);XFlush(display);}
    if(n==45){motion(display,DefaultScreen(display),knob_x,180,0);XFlush(display);}
    if(n==55){button(display,1,False,0);XFlush(display);}
    if(n==80){double edited=0;check(outgoing.values>initial_values,"knob reaches host automation");check(params->get_value(p,gain,&edited)&&std::abs(edited-0.5)>0.001,"UI edit updates DSP parameter");}
    if(n==90)second_begin=outgoing.begin;
    if(n==95){motion(display,DefaultScreen(display),knob_x,142,0);button(display,1,True,0);XFlush(display);}
    if(n==105){motion(display,DefaultScreen(display),knob_x,165,0);XFlush(display);}
    if(n==115){button(display,1,False,0);XFlush(display);}
    // Only one event fits per callback: forces retained begin/value/end progress.
    outgoing.budget=1;lib.data.flush=false;check(p->process(p,&c)!=CLAP_PROCESS_ERROR,"processing with an open editor");
    if(n>100&&outgoing.begin>second_begin&&!restored_during_gesture){saved.position=0;check(state->load(p,&saved.reader),"restore while a GUI gesture is pending");restored_during_gesture=true;}
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
   }
   check(outgoing.values>initial_values,"knob interaction reaches host automation");check(outgoing.begin==outgoing.end,"balanced parameter gestures under output backpressure");double restored_value=0;check(restored_during_gesture&&params->get_value(p,gain,&restored_value)&&std::abs(restored_value-expected_value)<1e-5,"host state supersedes queued UI edits");outgoing.budget=0xffffffffu;
   uint32_t width=0,height=0;check(g->get_size(p,&width,&height)&&width>=320&&height>=220,"GUI size");check(g->set_size(p,640,360)&&g->set_scale(p,1.25),"GUI resize/scale");
   for(int n=0;n<10;++n){if(lib.data.callback.exchange(false))p->on_main_thread(p);std::this_thread::sleep_for(std::chrono::milliseconds(10));}
   check(g->hide(p)&&g->show(p),"GUI hide/show");g->destroy(p);XDestroyWindow(display,parent);lib.data.display=nullptr;lib.data.parent=0;XCloseDisplay(display);dlclose(xtst);
  }
  p->stop_processing(p);p->deactivate(p);check(p->activate(p,44100,1,512)&&p->start_processing(p),"sample-rate reactivation");p->stop_processing(p);p->deactivate(p);p->destroy(p);report<<"PASS "<<d->id<<": lifecycle, ports, state, audio checks"<<(timing?", variable-block stress and heap probes":"")<<(gui?", embedded GUI, balanced knob gestures, state restore during pending edits":"")<<'\n';report.flush();
 }
 report<<"All checks passed. Allocation interposer self-test passed. "<<(timing?"Timing includes common host/event setup.\n":"GUI interaction uses one output event per callback to force gesture backpressure.\n");std::cout<<"PASS compatibility suite\n";return 0;
}catch(const std::exception&e){std::cerr<<"FAIL: "<<e.what()<<'\n';return 1;}}
