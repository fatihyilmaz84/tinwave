#pragma once
// Two one-line shims for the platform bridge (src/app/platform_bridge.rs, through cxx). They exist for the types:
// QWindow::winId() returns WId = quintptr = quint64 (unsigned long long on every 64-bit target), while cxx maps Rust's
// `usize` to `std::size_t`, which is `unsigned long` on 64-bit Linux and macOS. The two are the same width but not the
// same C++ type there, so cxx cannot bind winId() directly; a function returning rust::usize can.
#include "rust/cxx.h"

class QQuickWindow;

// The native handle of `window` (the HWND on Windows; creates the native window if it does not exist yet). Pointer-sized
// on every platform Qt supports, so the conversion to rust::usize loses nothing.
rust::usize tw_native_window_handle(const QQuickWindow& window);

// Name of the Qt platform plugin in use ("windows", "offscreen", "cocoa", "xcb"...). Only some plugins hand out real
// OS window handles; winId() of the others is a counter.
rust::String tw_qpa_name();
