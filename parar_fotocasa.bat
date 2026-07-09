@echo off
setlocal

tasklist /FI "IMAGENAME eq gestor-mvp.exe" | find /I "gestor-mvp.exe" >nul
if %ERRORLEVEL% NEQ 0 (
  echo FotoCasa no parece estar en marcha.
  echo.
  pause
  exit /b 0
)

echo Parando FotoCasa...
taskkill /IM gestor-mvp.exe /F >nul

echo.
echo FotoCasa parado.
echo.
pause
