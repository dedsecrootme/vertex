@echo off
rem Force le toolchain rustup (MSVC) pour eviter tout melange de toolchains.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d C:\Users\Administrateur\Vertex\apps\desktop\src-tauri
del lockgen.log >nul 2>&1
del lockgen-exit.txt >nul 2>&1
del Cargo.lock
cargo check --color=never > lockgen.log 2>&1
echo EXIT:%errorlevel% > lockgen-exit.txt
