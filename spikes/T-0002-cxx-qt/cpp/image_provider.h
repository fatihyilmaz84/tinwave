#pragma once
#include <QtQml/QQmlImageProviderBase>

// Factory so Rust never sees the Qt Quick class hierarchy.
QQmlImageProviderBase* tw_make_image_provider();
