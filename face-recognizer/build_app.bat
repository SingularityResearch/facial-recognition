@echo off
cd /d "%~dp0"
title Build FaceRecognizer.exe
rem Builds FaceRecognizer.exe (app.py: tracker + labeling tool in one window) next to this file.
rem The .exe carries the code and pages only; it uses the models\ and data\ folders beside it.
set "PY="
if exist "%LOCALAPPDATA%\Programs\Python\Python312\python.exe" set PY="%LOCALAPPDATA%\Programs\Python\Python312\python.exe"
if not defined PY (
  echo Python 3.12 is not installed yet. Double-click setup.bat first, then run this again.
  pause
  exit /b 1
)
%PY% -m pip install -q pyinstaller pywebview || goto :failed
%PY% -m PyInstaller --noconfirm --clean --onefile --windowed --name FaceRecognizer ^
  --workpath build --distpath build\dist --specpath build ^
  --add-data "%~dp0tracker_ui.html;." --add-data "%~dp0label_ui.html;." ^
  --add-data "%~dp0expression.js;." --add-data "%~dp0signals.js;." ^
  --collect-all mediapipe --collect-all onnxruntime --collect-submodules webview ^
  --hidden-import sklearn.cluster ^
  --exclude-module torch --exclude-module torchvision --exclude-module torchaudio --exclude-module torchgen ^
  --exclude-module tensorflow --exclude-module pandas --exclude-module IPython ^
  app.py || goto :failed
copy /y build\dist\FaceRecognizer.exe FaceRecognizer.exe >nul || goto :failed
echo.
echo Built FaceRecognizer.exe. Keep it in this folder: it uses models\ and data\ beside it.
exit /b 0
:failed
echo.
echo The build failed; see the messages above.
pause
exit /b 1
