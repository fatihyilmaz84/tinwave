@echo off
rem Configure + build the CMake variant with Ninja + MSVC (vcvars from VS 2026 Community).
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set QMAKE=C:\Qt\6.12.0\msvc2022_64\bin\qmake.exe
set PATH=C:\Qt\6.12.0\msvc2022_64\bin;C:\Qt\Tools\Ninja;C:\Qt\Tools\CMake_64\bin;%PATH%
if "%1"=="configure" (
  cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_PREFIX_PATH=C:/Qt/6.12.0/msvc2022_64 || exit /b 1
) else (
  cmake --build build || exit /b 1
)
