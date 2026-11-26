@echo off
setlocal
cd /d "%~dp0"

where py >nul 2>nul
if errorlevel 1 (
  echo Python launcher was not found. Install Python 3.11 or newer and enable Add to PATH.
  exit /b 1
)

py -3 -m pip install --upgrade pip pyinstaller
if errorlevel 1 exit /b %errorlevel%

py -3 -m PyInstaller --noconfirm --clean MojModelBridge.spec
if errorlevel 1 exit /b %errorlevel%

echo.
echo Build complete: dist\MojModelBridge.exe
endlocal
