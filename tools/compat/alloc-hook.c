// Linux/glibc only. Count heap operations on the calling render thread.
#include <stddef.h>
#include <stdint.h>
#include <errno.h>
extern void* __libc_malloc(size_t);extern void* __libc_calloc(size_t,size_t);extern void* __libc_realloc(void*,size_t);extern void __libc_free(void*);extern void* __libc_memalign(size_t,size_t);
static _Thread_local int enabled;static _Thread_local uint64_t allocations,deallocations;
void soniclaw_alloc_begin(void){allocations=deallocations=0;enabled=1;}
uint64_t soniclaw_alloc_end(void){enabled=0;return allocations;}
uint64_t soniclaw_free_count(void){return deallocations;}
void* malloc(size_t n){if(enabled)++allocations;return __libc_malloc(n);}
void* calloc(size_t n,size_t s){if(enabled)++allocations;return __libc_calloc(n,s);}
void* realloc(void*p,size_t n){if(enabled)++allocations;return __libc_realloc(p,n);}
void free(void*p){if(enabled&&p)++deallocations;__libc_free(p);}
void* aligned_alloc(size_t a,size_t n){if(enabled)++allocations;return __libc_memalign(a,n);}
void* memalign(size_t a,size_t n){if(enabled)++allocations;return __libc_memalign(a,n);}
int posix_memalign(void**out,size_t a,size_t n){if(a<sizeof(void*)||(a&(a-1))!=0)return EINVAL;if(enabled)++allocations;void*p=__libc_memalign(a,n);if(!p)return ENOMEM;*out=p;return 0;}
