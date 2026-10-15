@echo off
setlocal
cd /d "%~dp0client"
start "" "tauri\target\debug\taurigram.exe"
exit
