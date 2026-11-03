#include "platform_window.h"

#include <QtGui/QGuiApplication>
#include <QtQuick/QQuickWindow>

rust::usize tw_native_window_handle(const QQuickWindow& window)
{
    return static_cast<rust::usize>(window.winId());
}

rust::String tw_qpa_name()
{
    return rust::String(QGuiApplication::platformName().toStdString());
}
