@echo off
cd /d "%~dp0"
title Face Labels
rem Prefer the Python 3.12 that setup.bat installs: other apps (AI toolkits, ComfyUI...) may add
rem newer Pythons that "py -3" would pick, and those don't have this tool's libraries.
set "PY="
if exist "%LOCALAPPDATA%\Programs\Python\Python312\python.exe" set PY="%LOCALAPPDATA%\Programs\Python\Python312\python.exe"
if not defined PY py -3.12 -c "import sys; sys.exit(42)" >nul 2>&1
if not defined PY if errorlevel 42 if not errorlevel 43 set "PY=py -3.12"
if not defined PY py -3 -c "import sys; sys.exit(42)" >nul 2>&1
if not defined PY if errorlevel 42 if not errorlevel 43 set "PY=py -3"
if not defined PY python -c "import sys; sys.exit(42)" >nul 2>&1
if not defined PY if errorlevel 42 if not errorlevel 43 set "PY=python"
if not defined PY (
  echo Python is not installed yet. Double-click setup.bat first, then run this again.
  pause
  exit /b 1
)
%PY% -c "import numpy" >nul 2>&1
if errorlevel 1 (
  echo The libraries are not installed yet. Double-click setup.bat first, then run this again.
  pause
  exit /b 1
)
%PY% label_tool.py
if errorlevel 1 pause
