# T-0011: build the spike host (Qt 6.12 MSVC kit + VS 2026 + Ninja). Output: <BuildDir>\t0011-host.exe
# Usage:  .\build.ps1                       (builds into .\build, which is git-ignored)
#         .\build.ps1 -BuildDir C:\tmp\t11
param(
    [string] $BuildDir = "$PSScriptRoot\build",
    [string] $Qt = "C:\Qt\6.12.0\msvc2022_64",
    [string] $VcVars = "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat"
)
$script = @"
@echo off
call "$VcVars" >nul
set PATH=C:\Qt\Tools\Ninja;C:\Qt\Tools\CMake_64\bin;%PATH%
cmake -S "$PSScriptRoot" -B "$BuildDir" -G Ninja -DCMAKE_PREFIX_PATH=$($Qt -replace '\\','/') -DCMAKE_BUILD_TYPE=Release || exit /b 1
cmake --build "$BuildDir" || exit /b 1
"@
$bat = [IO.Path]::Combine([IO.Path]::GetTempPath(), "t0011-build.bat")
Set-Content -Path $bat -Value $script -Encoding ascii
cmd /c $bat
Remove-Item $bat -ErrorAction SilentlyContinue
"Built: $BuildDir\t0011-host.exe"
