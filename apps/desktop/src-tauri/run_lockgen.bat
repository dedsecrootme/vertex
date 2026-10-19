@echo off
rem Force le toolchain rustup (MSVC) pour eviter tout melange de toolchains.
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
del lockgen.log >nul 2>&1
del lockgen-exit.txt >nul 2>&1
cargo generate-lockfile --color=never > lockgen.log 2>&1
echo EXIT:%errorlevel% > lockgen-exit.txt
