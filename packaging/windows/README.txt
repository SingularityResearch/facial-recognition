Face Recognizer for Windows (64-bit), without installing

This folder is the whole app: FaceRecognizer.exe and, in runtime\, what it runs with.
The installer (FaceRecognizer-Setup-x64.exe) puts the same files in place and adds Start menu and desktop
icons; use this folder when you would rather not install.

To use it
  1. Unpack the folder somewhere you can write to (not under Program Files): the app keeps its face models
     and your data (faces, names, labels) beside FaceRecognizer.exe, in models\ and data\.
  2. Start FaceRecognizer.exe.
  3. At its first start the app says which files it still needs (the face models, 79 MB) and asks before it
     downloads them. Each file is checked by its size and SHA-256.

If you already use Face Recognizer, unpack over its folder, or copy your models\ and data\ folders beside the
new FaceRecognizer.exe: they are used as they are.

The app's window is shown by Microsoft's WebView2 runtime, which is part of Windows 11 and on nearly every
Windows 10. Without it the app opens in your web browser and works the same; the installer can bring it.

Everything runs on this computer. Nothing leaves it unless you send it on purpose.
Project, source code and licences: https://github.com/SingularityResearch/facial-recognition
