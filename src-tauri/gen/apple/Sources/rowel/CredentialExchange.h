#pragma once

#include <stddef.h>
#include <stdint.h>

// The core's side of a credential exchange (src-tauri/src/exchange.rs), as
// CredentialExchange.swift calls it. Both copy what they are given.

// The credentials of a transfer that succeeded, as a CXF JSON document.
void rowel_credential_exchange_received(const uint8_t *bytes, size_t len);

// Why a transfer the system began produced no document; NUL-terminated.
void rowel_credential_exchange_failed(const char *reason);
