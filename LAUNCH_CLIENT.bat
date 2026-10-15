@echo off
setlocal
cd /d "%~dp0client"
start "" "tauri\target\debug\ethernet.exe"
exit
