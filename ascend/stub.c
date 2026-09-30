#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>

enum { OK=0, ERR_ARG=-1, ERR_SHAPE=-2, ERR_RUNTIME=-3, ERR_ALLOC=-4 };
typedef struct { int device; } ascend_miner_ctx;
static uint32_t rotl13(uint32_t x) { return (x << 13) | (x >> 19); }

int ascend_miner_is_hardware(void) { return 0; }
int ascend_miner_create(int device, ascend_miner_ctx **out) {
    if (!out || device < 0) return ERR_ARG;
    *out = calloc(1, sizeof(**out));
    if (!*out) return ERR_ALLOC;
    (*out)->device = device;
    return OK;
}
void ascend_miner_destroy(ascend_miner_ctx *ctx) { free(ctx); }

int ascend_miner_tile_state(ascend_miner_ctx *ctx, const int8_t *a, const int8_t *b,
    uint32_t h, uint32_t w, uint32_t k, uint32_t rank, uint32_t dot_len, int32_t out[16]) {
    if (!ctx || !a || !b || !out || !h || !w || !k || !rank || dot_len > k || dot_len % rank) return ERR_ARG;
    size_t cells = (size_t)h * w;
    uint32_t *acc = calloc(cells, sizeof(*acc));
    if (!acc) return ERR_ALLOC;
    memset(out, 0, 16 * sizeof(*out));
    for (uint32_t step=0; step<dot_len/rank; ++step) {
        uint32_t x=0, lo=step*rank;
        for (uint32_t u=0; u<h; ++u) for (uint32_t v=0; v<w; ++v) {
            uint32_t delta=0;
            for (uint32_t i=0; i<rank; ++i) delta += (uint32_t)((int32_t)a[(size_t)u*k+lo+i] * (int32_t)b[(size_t)v*k+lo+i]);
            size_t p=(size_t)u*w+v; acc[p]+=delta;
        }
        for (size_t p=0; p<cells; ++p) x ^= acc[p];
        uint32_t slot=step&15u; out[slot]=(int32_t)(rotl13((uint32_t)out[slot])^x);
    }
    free(acc); return OK;
}

const char *ascend_miner_error_string(int code) {
    switch (code) {
        case OK: return "ok"; case ERR_ARG: return "invalid argument";
        case ERR_SHAPE: return "unsupported shape"; case ERR_RUNTIME: return "CANN runtime failure";
        case ERR_ALLOC: return "allocation failure"; default: return "unknown Ascend error";
    }
}
