@echo off
setlocal
cd /d "%~dp0client"
if not exist "tauri\target\debug\taurigram.exe" (
    echo [ОШИБКА] taurigram.exe не найден. Сначала запустите "Сборка и Запуск.bat".
    pause
    exit /b 1
)
start "" "tauri\target\debug\taurigram.exe"
exit
