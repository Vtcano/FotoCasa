@echo off
setlocal

cd /d "%~dp0"
set "PHOTO_ROOT=Z:\Fotos_iphone"

tasklist /FI "IMAGENAME eq gestor-mvp.exe" | find /I "gestor-mvp.exe" >nul
if %ERRORLEVEL% EQU 0 (
  echo FotoCasa ya esta en marcha.
  echo.
  echo Portatil: http://localhost:3000
  echo Movil en WiFi: http://IP-DE-ESTE-PC:3000
  echo Tailscale: http://100.116.221.8:3000
  echo.
  pause
  exit /b 0
)

echo Preparando FotoCasa...
cargo build
if %ERRORLEVEL% NEQ 0 (
  echo.
  echo No se pudo compilar FotoCasa. Mira server.err.log o la salida de esta ventana.
  pause
  exit /b 1
)

echo Arrancando motor FotoCasa...
start "FotoCasa Motor" /min cmd /c "set PHOTO_ROOT=%PHOTO_ROOT%&& target\debug\gestor-mvp.exe > server.out.log 2> server.err.log"

echo.
echo FotoCasa se esta arrancando en segundo plano.
echo.
echo Portatil: http://localhost:3000
echo Movil en la misma WiFi: http://IP-DE-ESTE-PC:3000
echo Tailscale: http://100.116.221.8:3000
echo.
echo Para ver la IP WiFi, ejecuta ipconfig y busca "Direccion IPv4".
echo Para pararlo, usa parar_fotocasa.bat.
echo.
pause
