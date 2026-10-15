@echo off
setlocal
cd /d "%~dp0client"
if not exist "tauri\target\debug\ethernet.exe" (
    echo [ОШИБКА] ethernet.exe не найден. Сначала запустите "Сборка и Запуск.bat".
    pause
    exit /b 1
)
start "" "tauri\target\debug\ethernet.exe"
exit
