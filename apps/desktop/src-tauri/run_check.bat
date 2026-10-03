@echo off
rem Force le toolchain rustup (MSVC) pour eviter tout melange de toolchains.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d C:\Users\Administrateur\Vertex\apps\desktop\src-tauri
del build.log >nul 2>&1
del build-exit.txt >nul 2>&1
cargo check --color=never > build.log 2>&1
echo EXIT:%errorlevel% > build-exit.txt
