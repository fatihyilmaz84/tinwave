#pragma once
#include <QtQml/QQmlImageProviderBase>

// Factory so Rust never sees the Qt Quick class hierarchy.
QQmlImageProviderBase* tw_make_image_provider();

// GUI thread only: call after QGuiApplication::exec() returned and BEFORE the QML engine is destroyed.
// Stops the worker pool (see image_provider.cpp, rule 4). Idempotent; no-op if no provider exists.
void tw_image_provider_shutdown();
