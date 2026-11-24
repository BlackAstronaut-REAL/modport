@echo off
setlocal
if "%~2"=="" (
  echo MojModelBridge 3.4.0 Beta 3 - Full Mod Port
  echo.
  echo Usage without a template:
  echo   full_port.bat "MOD_ROOT" "OUTPUT_ROOT"
  echo.
  echo Usage with a known-good MCreator template:
  echo   full_port.bat "MOD_ROOT" "MCREATOR_TEMPLATE" "OUTPUT_ROOT"
  echo.
  echo The converter will create:
  echo   OUTPUT_ROOT\Blockbench\        - converted .bbmodel files and animation analysis
  echo   OUTPUT_ROOT\MCreatorWorkspace\  - complete MCreator workspace
  echo.
  exit /b 1
)
set "MOD_ROOT=%~1"
if "%~3"=="" (
  set "MCREATOR_TEMPLATE="
  set "OUTPUT_ROOT=%~2"
) else (
  set "MCREATOR_TEMPLATE=%~2"
  set "OUTPUT_ROOT=%~3"
)
set "BB_OUT=%OUTPUT_ROOT%\Blockbench"
set "MC_OUT=%OUTPUT_ROOT%\MCreatorWorkspace"

if defined MCREATOR_TEMPLATE (
  python "%~dp0model_bridge.py" --full-port --mod-root "%MOD_ROOT%" --output "%BB_OUT%" --mcreator-output "%MC_OUT%" --mcreator-template "%MCREATOR_TEMPLATE%" --model-type auto
) else (
  python "%~dp0model_bridge.py" --full-port --mod-root "%MOD_ROOT%" --output "%BB_OUT%" --mcreator-output "%MC_OUT%" --model-type auto
)
if errorlevel 1 (
  echo.
  echo Full port failed. Review the CMD output and reports in the output folders.
  exit /b %errorlevel%
)

echo.
echo Full mod port completed.
echo Blockbench output: %BB_OUT%
echo MCreator workspace: %MC_OUT%
if defined MCREATOR_TEMPLATE echo Template mode: %MCREATOR_TEMPLATE%
if not defined MCREATOR_TEMPLATE echo Standalone MCreator workspace mode.
endlocal
