#pragma once
#include <QtQml/QQmlImageProviderBase>
#include <QtCore/QString>
// variant: queued | autoconn | olda | oldb  (see providers.cpp)
QQmlImageProviderBase* make_provider(const QString& variant, int threads);
// Call on the GUI thread after QGuiApplication::exec() returns and BEFORE the QML engine is destroyed (prod variant only).
void quiesce_providers();
