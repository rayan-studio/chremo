@echo off
setlocal
if exist "%~dp0.venv\Scripts\python.exe" (
    "%~dp0.venv\Scripts\python.exe" "%~dp0support\local_build.py" %*
) else (
    python "%~dp0support\local_build.py" %*
)
exit /b %errorlevel%
