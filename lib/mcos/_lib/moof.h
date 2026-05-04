// lib/mcos/_lib/moof.h — c binding for the wasm mco abi.
// see docs/reference/native-abi.md.

#ifndef MOOF_H
#define MOOF_H

#include <stddef.h>
#include <stdint.h>

// import declarations
__attribute__((import_module("moof"), import_name("moof_raise")))
__attribute__((noreturn))
extern void moof_raise(uint32_t kind_handle, const char *msg, size_t msg_len);

__attribute__((import_module("moof"), import_name("moof_make_string")))
extern uint32_t moof_make_string(const char *ptr, size_t len);

__attribute__((import_module("moof"), import_name("moof_make_bytes")))
extern uint32_t moof_make_bytes(const uint8_t *ptr, size_t len);

__attribute__((import_module("moof"), import_name("moof_string_text")))
extern size_t moof_string_text(uint32_t handle, char *buf, size_t cap);

__attribute__((import_module("moof"), import_name("moof_bytes_data")))
extern size_t moof_bytes_data(uint32_t handle, uint8_t *buf, size_t cap);

__attribute__((import_module("moof"), import_name("moof_intern")))
extern uint32_t moof_intern(const char *ptr, size_t len);

// ergonomic helpers
static inline void moof_raise_kind(const char *kind, const char *msg) {
    uint32_t k = moof_intern(kind, __builtin_strlen(kind));
    moof_raise(k, msg, __builtin_strlen(msg));
    __builtin_unreachable();
}

#endif
