@echo off
cd /d "%~dp0"
python -m pytest -q
if errorlevel 1 (
  echo.
  echo Tests failed. Ensure pytest is installed with: python -m pip install pytest
  exit /b %errorlevel%
)
echo.
echo MojModelBridge tests passed.
