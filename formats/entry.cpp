// The Rust static library owns the actual immutable CLAP entry and callbacks.
// Referencing it keeps the implementation linked even with archive dead stripping.
#include <clap/entry.h>
extern const clap_plugin_entry clap_entry;
extern "C" const clap_plugin_entry* soniclaw_static_entry() { return &clap_entry; }
