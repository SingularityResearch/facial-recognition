@echo off
setlocal
cd /d "%~dp0"
title Face Recognizer setup
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
if defined PY goto deps

echo Installing Python 3.12 with winget (Windows package manager)...
winget install -e --id Python.Python.3.12 --scope user --accept-package-agreements --accept-source-agreements
set PY="%LOCALAPPDATA%\Programs\Python\Python312\python.exe"
if not exist %PY% (
  echo.
  echo Python did not install. Install it from https://www.python.org/downloads/ ^(tick "Add to PATH"^), then run this file again.
  pause
  exit /b 1
)

:deps
echo.
echo Using Python:
%PY% --version
echo.
echo Installing libraries (OpenCV, NumPy, scikit-learn, Pillow, HEIC support)...
%PY% -m pip install --upgrade pip
%PY% -m pip install -r requirements.txt
if errorlevel 1 (
  echo.
  echo Library install failed. See the messages above.
  pause
  exit /b 1
)
echo.
echo Checking the face models load...
%PY% -c "import face_id as F; F.Faces(); print('Models OK')"
echo.
echo Setup finished. You can close this window and double-click label.bat.
pause
