@echo off
rem Force le toolchain rustup (MSVC) pour eviter tout melange de toolchains.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
del build.log >nul 2>&1
del build-exit.txt >nul 2>&1
cargo check --locked --color=never > build.log 2>&1
echo EXIT:%errorlevel% > build-exit.txt
