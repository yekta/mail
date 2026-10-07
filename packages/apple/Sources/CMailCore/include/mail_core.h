// The Rust core's C interface (crates/core/src/ffi.rs). Everything is JSON text.

#ifndef MAIL_CORE_H
#define MAIL_CORE_H

#include <stdbool.h>

// Called on a background thread with one event. The text is only valid during the call.
typedef void (*mail_core_event_callback)(const char *event_json, void *context);

// Starts the core with an `api::Config`. Returns false if it couldn't start.
bool mail_core_start(const char *config_json, mail_core_event_callback callback, void *context);

// Sends one command, an `api::Envelope`. The answer arrives as a `reply` event.
void mail_core_command(const char *command_json);

#endif
