@echo off
rem Force le toolchain rustup (MSVC) : sans cela, un rustc/cargo GNU installe
rem ailleurs dans le PATH peut etre utilise et melanger les toolchains
rem (erreurs E0107 sur indexmap / E0658 sur embed-resource).
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
del tbuild.log >nul 2>&1
del tbuild-exit.txt >nul 2>&1
cargo tauri build > tbuild.log 2>&1
echo EXIT:%errorlevel% > tbuild-exit.txt
