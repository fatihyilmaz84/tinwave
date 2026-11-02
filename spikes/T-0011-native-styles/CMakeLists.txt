cmake_minimum_required(VERSION 3.21)
project(t0011_native_styles LANGUAGES CXX)

# Throwaway spike host for T-0011. Not part of the Cargo workspace; never merged into crates/.
# Build (from this folder, Qt 6.12 MSVC kit):
#   cmake -S . -B <builddir> -G Ninja -DCMAKE_PREFIX_PATH=C:/Qt/6.12.0/msvc2022_64 -DCMAKE_BUILD_TYPE=Release
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Qt6 6.8 REQUIRED COMPONENTS Quick QuickControls2 Gui Qml)
qt_standard_project_setup(REQUIRES 6.8)

# ---- Tinwave.Theme: the singleton module (mirrors apps/tinwave-desktop/qml/Theme/) ----
qt_add_library(t0011-theme STATIC)
set_source_files_properties(qml/Theme/Theme.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)
qt_add_qml_module(t0011-theme
    URI Tinwave.Theme
    VERSION 1.0
    QML_FILES qml/Theme/Theme.qml
)
target_link_libraries(t0011-theme PRIVATE Qt6::Quick)

# ---- host executable + Tinwave.Spike module (the prototype pages) ----
qt_add_executable(t0011-host
    host/main.cpp
    host/platforminfo.h
    host/platforminfo.cpp
)
qt_add_qml_module(t0011-host
    URI Tinwave.Spike
    VERSION 1.0
    QML_FILES
        qml/Main.qml
        qml/ControlsPage.qml
        qml/SelfTest.qml
        qml/MinimalWindow.qml
)
target_include_directories(t0011-host PRIVATE host)
target_link_libraries(t0011-host PRIVATE
    Qt6::Quick Qt6::QuickControls2 Qt6::Gui Qt6::Qml
    t0011-themeplugin
)
if(WIN32)
    target_link_libraries(t0011-host PRIVATE dwmapi user32 psapi gdi32 advapi32)
endif()
