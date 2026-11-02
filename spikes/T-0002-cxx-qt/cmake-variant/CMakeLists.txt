cmake_minimum_required(VERSION 3.24)
project(t0002_cmake LANGUAGES CXX)

# Rust always links the non-debug MSVC runtime; keep everything consistent (see Corrosion docs).
if(CMAKE_CXX_COMPILER_ID STREQUAL "MSVC")
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")
endif()
set(CMAKE_AUTOMOC ON)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

set(CXXQT_QTCOMPONENTS Core Gui Qml Quick QuickControls2)
find_package(Qt6 REQUIRED COMPONENTS ${CXXQT_QTCOMPONENTS})

find_package(CxxQt QUIET)
if(NOT CxxQt_FOUND)
  include(FetchContent)
  FetchContent_Declare(CxxQt GIT_REPOSITORY https://github.com/kdab/cxx-qt-cmake.git GIT_TAG main)
  FetchContent_MakeAvailable(CxxQt)
endif()

cxx_qt_import_crate(
  MANIFEST_PATH rust/Cargo.toml
  CRATES t0002_lib
  QT_MODULES Qt::Core Qt::Gui Qt::Qml Qt::Quick Qt::QuickControls2)
cxx_qt_import_qml_module(t0002_qml_module URI "Tw.Spike" SOURCE_CRATE t0002_lib)

add_executable(t0002_cmake main.cpp)
target_link_libraries(t0002_cmake PRIVATE t0002_qml_module Qt6::QuickControls2)
