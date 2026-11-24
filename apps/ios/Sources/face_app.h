// What the Rust side of the app offers the shell (crates/face-app).
#ifndef FACE_APP_H
#define FACE_APP_H
#include <stdint.h>

/// Start the live tracker inside this app; its port on 127.0.0.1, or -1 (face_app_error says why).
int32_t face_app_start(const char *root, const char *home, const char *pages);
/// What this device is called and its system ("iOS 26.1"), before face_app_start.
void face_app_device(const char *name, const char *os);
/// The phone's heat state (0 nominal, 1 fair, 2 serious, 3 critical; -1 not known), its battery in percent
/// (-1 not known) and whether it is on power (0, 1; -1 not known): said every few seconds, for the Signals chart.
void face_app_health(int32_t heat, float battery, int32_t on_power);
/// Why face_app_start returned -1, or NULL.
const char *face_app_error(void);

#endif
