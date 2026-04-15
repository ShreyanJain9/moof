// example moof plugin — a random number generator capability
//
// compile:
//   cc -shared -o librandom.dylib example_plugin.c  (macOS)
//   cc -shared -fPIC -o librandom.so example_plugin.c  (Linux)
//
// load from moof REPL:
//   (load-plugin "path/to/librandom.dylib")
//   [random next]          → Act<some-integer>
//   [random nextIn: 100]   → Act<0..99>

#include <stdlib.h>
#include <stdint.h>
#include <time.h>

// moof plugin C API (provided by the moof runtime)
typedef struct MoofSetupCtx MoofSetupCtx;
typedef struct MoofCallCtx MoofCallCtx;
typedef uint64_t (*PluginHandlerFn)(MoofCallCtx*, uint64_t, const uint64_t*, uint32_t);

extern void moof_register_handler(MoofSetupCtx*, const char*, PluginHandlerFn);
extern uint64_t moof_make_integer(int64_t);
extern uint64_t moof_make_string(MoofCallCtx*, const char*);
extern int64_t moof_as_integer(uint64_t);
extern uint64_t moof_nil(void);

// handlers

static uint64_t handle_next(MoofCallCtx* ctx, uint64_t recv, const uint64_t* args, uint32_t nargs) {
    return moof_make_integer(rand());
}

static uint64_t handle_next_in(MoofCallCtx* ctx, uint64_t recv, const uint64_t* args, uint32_t nargs) {
    if (nargs < 1) return moof_make_integer(0);
    int64_t max = moof_as_integer(args[0]);
    if (max <= 0) return moof_make_integer(0);
    return moof_make_integer(rand() % max);
}

static uint64_t handle_describe(MoofCallCtx* ctx, uint64_t recv, const uint64_t* args, uint32_t nargs) {
    return moof_make_string(ctx, "<Random>");
}

// plugin entry points

const char* moof_plugin_name(void) {
    return "random";
}

void moof_plugin_setup(MoofSetupCtx* ctx) {
    srand(time(NULL));
    moof_register_handler(ctx, "next", handle_next);
    moof_register_handler(ctx, "nextIn:", handle_next_in);
    moof_register_handler(ctx, "describe", handle_describe);
}
