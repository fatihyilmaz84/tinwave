@echo off
rem usage: build.cmd <dir> [asan]   (Debug build with Qt debug DLLs; "asan" adds /fsanitize=address)
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set PATH=C:\Qt\6.12.0\msvc2022_64\bin;C:\Qt\Tools\Ninja;C:\Qt\Tools\CMake_64\bin;%PATH%
set ASAN=OFF
if "%BT%"=="" set BT=Debug
if "%2"=="asan" set ASAN=ON
cmake -S . -B %1 -G Ninja -DCMAKE_BUILD_TYPE=%BT% -DTW_ASAN=%ASAN% -DCMAKE_PREFIX_PATH=C:/Qt/6.12.0/msvc2022_64 || exit /b 1
cmake --build %1 || exit /b 1
copy /y stress.qml %1\ >nul
