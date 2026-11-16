// Callback-only denormal measurement. Reuse the independent host ABI fixtures;
// all sample generation happens before the timed region.
#define main soniclaw_full_compat_main
#include "host.cpp"
#undef main
#include <xmmintrin.h>
int main(int argc,char**argv){try{
 check(argc==3,"usage: denormal-probe library.clap plugin-id");Loaded lib(argv[1]);const clap_plugin*plugin=nullptr;
 for(uint32_t n=0;n<lib.factory->get_plugin_count(lib.factory);++n){auto*d=lib.factory->get_plugin_descriptor(lib.factory,n);if(!strcmp(d->id,argv[2]))plugin=lib.factory->create_plugin(lib.factory,&lib.host,d->id);}
 check(plugin&&plugin->init(plugin),"plugin init");auto*params=static_cast<const clap_plugin_params*>(plugin->get_extension(plugin,CLAP_EXT_PARAMS));check(params,"parameter extension");Events events;
 auto set=[&](const char*name,double val){for(uint32_t n=0;n<params->count(plugin);++n){clap_param_info i{};check(params->get_info(plugin,n,&i),"parameter info");if(!strcmp(i.name,name)){auto e=value(i.id,val);events.events[0]=&e.header;events.n=1;params->flush(plugin,&events.in,nullptr);events.n=0;}}};
 set("Mode",1.0);set("Gain",0.5);check(plugin->activate(plugin,48000.0,1,256)&&plugin->start_processing(plugin),"activation");
 std::array<float,256> left{},right{},outL{},outR{};float*in[]{left.data(),right.data()},*out[]{outL.data(),outR.data()};clap_audio_buffer input{in,nullptr,2,0,0},output{out,nullptr,2,0,0};clap_process c{0,256,nullptr,&input,&output,1,1,&events.in,nullptr};
 const unsigned original=_mm_getcsr();const unsigned no_flush=original&~0x8040u;_mm_setcsr(no_flush);
 uint32_t bits=0x00400000u;float tiny=0;memcpy(&tiny,&bits,sizeof(tiny));left.fill(tiny);right.fill(tiny);plugin->reset(plugin);
 unsigned before=_mm_getcsr();check(plugin->process(plugin,&c)!=CLAP_PROCESS_ERROR,"denormal callback");unsigned after=_mm_getcsr();check((before&~0x3fu)==(after&~0x3fu),"host floating-point mode restored");for(float x:outL)check(x==0.0f,"subnormal input flushed inside callback");
 std::vector<double> normal_times,denormal_times;constexpr unsigned callbacks=32768;
 for(int round=0;round<9;++round){for(int order=0;order<2;++order){bool denormal=(round+order)%2!=0;left.fill(denormal?tiny:0.25f);right.fill(denormal?tiny:0.25f);plugin->reset(plugin);for(int n=0;n<256;++n)plugin->process(plugin,&c);double start=thread_ns();for(unsigned n=0;n<callbacks;++n)check(plugin->process(plugin,&c)!=CLAP_PROCESS_ERROR,"timed callback");double cost=(thread_ns()-start)/callbacks;(denormal?denormal_times:normal_times).push_back(cost);}}
 std::sort(normal_times.begin(),normal_times.end());std::sort(denormal_times.begin(),denormal_times.end());
 _mm_setcsr(original);plugin->stop_processing(plugin);plugin->deactivate(plugin);plugin->destroy(plugin);
 std::cout<<"{\"plugin\":\""<<argv[2]<<"\",\"frames\":256,\"rounds\":9,\"callbacks_per_batch\":"<<callbacks<<",\"normal_cpu_ns\":"<<normal_times[4]<<",\"denormal_cpu_ns\":"<<denormal_times[4]<<",\"ratio\":"<<denormal_times[4]/normal_times[4]<<",\"host_control_bits_restored\":true,\"subnormal_gain_output_is_zero\":true}\n";return 0;
 }catch(const std::exception&e){std::cerr<<"FAIL: "<<e.what()<<'\n';return 1;}}
