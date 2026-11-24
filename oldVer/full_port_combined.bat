@echo off
setlocal
if "%~3"=="" (
  echo MojModelBridge 3.4.0 Beta 3 - Combined Compiled + Decompiled Full Port
  echo.
  echo Usage without a template:
  echo   full_port_combined.bat "COMPILED_ROOT" "DECOMPILED_ROOT" "OUTPUT_ROOT"
  echo.
  echo Usage with a known-good MCreator template:
  echo   full_port_combined.bat "COMPILED_ROOT" "DECOMPILED_ROOT" "MCREATOR_TEMPLATE" "OUTPUT_ROOT"
  echo.
  exit /b 1
)
set "COMPILED_ROOT=%~1"
set "DECOMPILED_ROOT=%~2"
if "%~4"=="" (
  set "MCREATOR_TEMPLATE="
  set "OUTPUT_ROOT=%~3"
) else (
  set "MCREATOR_TEMPLATE=%~3"
  set "OUTPUT_ROOT=%~4"
)
set "BB_OUT=%OUTPUT_ROOT%\Blockbench"
set "MC_OUT=%OUTPUT_ROOT%\MCreatorWorkspace"

if defined MCREATOR_TEMPLATE (
  python "%~dp0model_bridge.py" --full-port --compiled-root "%COMPILED_ROOT%" --decompiled-root "%DECOMPILED_ROOT%" --output "%BB_OUT%" --mcreator-output "%MC_OUT%" --mcreator-template "%MCREATOR_TEMPLATE%" --model-type auto
) else (
  python "%~dp0model_bridge.py" --full-port --compiled-root "%COMPILED_ROOT%" --decompiled-root "%DECOMPILED_ROOT%" --output "%BB_OUT%" --mcreator-output "%MC_OUT%" --model-type auto
)
if errorlevel 1 (
  echo.
  echo Combined full port failed. Review the CMD output and reports in the output folders.
  exit /b %errorlevel%
)

echo.
echo Combined full mod port completed.
echo Blockbench output: %BB_OUT%
echo MCreator workspace: %MC_OUT%
endlocal
