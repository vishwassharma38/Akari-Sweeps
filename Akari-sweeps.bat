@echo off
setlocal EnableExtensions EnableDelayedExpansion
title Windows 10 - Deep Junk Cleaner v2

:: ============================================================
:: WINDOWS 10 DEEP JUNK CLEANER v2
::
:: Designed for Windows 10.
::
:: CLEANS:
::   - User/System temporary files
::   - Windows Update download cache
::   - Delivery Optimization cache
::   - Windows error reports / crash dumps
::   - Thumbnail + DirectX/NVIDIA shader caches
::   - Windows Store cache
::   - Browser caches for common Chromium browsers + Firefox
::   - VS Code caches/logs
::   - Discord cache
::   - npm + pip caches
::   - Recycle Bin
::   - DNS resolver cache
::   - Windows Disk Cleanup / SilentCleanup
::   - Windows component-store cleanup
::
:: DOES NOT intentionally delete:
::   - Browser history, cookies, passwords, bookmarks, profiles
::   - VS Code settings/extensions/projects
::   - Documents, Desktop, Downloads
::   - node_modules
::   - Cargo/Rust registry or build caches
::   - NuGet global packages
::   - Prefetch
::   - pagefile.sys / hiberfil.sys
::   - Windows.old directly
::   - arbitrary AppData folders
::
:: IMPORTANT:
::   Close browsers, VS Code, Discord, and other apps first.
::   Locked files are skipped automatically.
:: ============================================================

:: --- Request Administrator privileges ---
fltmc >nul 2>&1
if not "%errorlevel%"=="0" (
    echo Requesting Administrator privileges...
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
    exit /b
)

:: --- Save starting free space ---
for /f "tokens=3" %%A in ('fsutil volume diskfree C: ^| findstr /i "Total # of free bytes"') do set "FREE_BEFORE=%%A"

echo.
echo ============================================================
echo              WINDOWS 10 DEEP JUNK CLEANER v2
echo ============================================================
echo.
echo This performs a deeper cache/temp cleanup.
echo.
echo IMPORTANT: Close Chrome, Edge, Firefox, Brave, Opera,
echo Vivaldi, VS Code, Discord, and other apps before continuing.
echo.
echo Browser cache is deleted, but history/cookies/passwords/
echo bookmarks/profile data are NOT intentionally deleted.
echo.
choice /C YN /N /M "Start cleanup? [Y/N]: "
if errorlevel 2 exit /b

echo.
echo ============================================================
echo [1/18] USER TEMPORARY FILES
echo ============================================================
call :CleanDir "%TEMP%"
call :CleanDir "%LOCALAPPDATA%\Temp"

echo.
echo ============================================================
echo [2/18] WINDOWS TEMPORARY FILES
echo ============================================================
call :CleanDir "%SystemRoot%\Temp"

echo.
echo ============================================================
echo [3/18] WINDOWS ERROR REPORTING / CRASH DUMPS
echo ============================================================
call :CleanDir "%ProgramData%\Microsoft\Windows\WER\ReportArchive"
call :CleanDir "%ProgramData%\Microsoft\Windows\WER\ReportQueue"
call :CleanDir "%ProgramData%\Microsoft\Windows\WER\Temp"
call :CleanDir "%LOCALAPPDATA%\CrashDumps"

echo.
echo ============================================================
echo [4/18] THUMBNAIL / GRAPHICS SHADER CACHES
echo ============================================================
call :CleanDir "%LOCALAPPDATA%\Microsoft\Windows\Explorer\ThumbCache"
call :CleanDir "%LOCALAPPDATA%\D3DSCache"
call :CleanDir "%LOCALAPPDATA%\NVIDIA\DXCache"
call :CleanDir "%LOCALAPPDATA%\NVIDIA\GLCache"
call :CleanDir "%ProgramData%\NVIDIA Corporation\NV_Cache"
call :CleanDir "%LOCALAPPDATA%\AMD\DxCache"
call :CleanDir "%LOCALAPPDATA%\AMD\GLCache"

echo.
echo ============================================================
echo [5/18] WINDOWS UPDATE DOWNLOAD CACHE
echo ============================================================
net stop wuauserv /y >nul 2>&1
net stop bits /y >nul 2>&1
call :CleanDir "%SystemRoot%\SoftwareDistribution\Download"
net start bits >nul 2>&1
net start wuauserv >nul 2>&1

echo.
echo ============================================================
echo [6/18] DELIVERY OPTIMIZATION CACHE
echo ============================================================
call :CleanDir "%ProgramData%\Microsoft\Windows\DeliveryOptimization\Cache"

echo.
echo ============================================================
echo [7/18] MICROSOFT STORE CACHE
echo ============================================================
if exist "%SystemRoot%\System32\wsreset.exe" (
    echo Resetting Microsoft Store cache...
    start /wait "" "%SystemRoot%\System32\wsreset.exe" >nul 2>&1
)

echo.
echo ============================================================
echo [8/18] VS CODE CACHE / LOGS
echo ============================================================
call :CleanDir "%APPDATA%\Code\Cache"
call :CleanDir "%APPDATA%\Code\CachedData"
call :CleanDir "%APPDATA%\Code\GPUCache"
call :CleanDir "%APPDATA%\Code\logs"
call :CleanDir "%APPDATA%\Code\Service Worker\CacheStorage"
call :CleanDir "%APPDATA%\Code\Service Worker\ScriptCache"
call :CleanDir "%APPDATA%\Code\CachedExtensionVSIXs"
call :CleanDir "%LOCALAPPDATA%\Code\Cache"
call :CleanDir "%LOCALAPPDATA%\Code\CachedData"
call :CleanDir "%LOCALAPPDATA%\Code\GPUCache"

echo.
echo ============================================================
echo [9/18] GOOGLE CHROME CACHE - ALL PROFILES
echo ============================================================
call :CleanChromium "%LOCALAPPDATA%\Google\Chrome\User Data"

echo.
echo ============================================================
echo [10/18] MICROSOFT EDGE CACHE - ALL PROFILES
echo ============================================================
call :CleanChromium "%LOCALAPPDATA%\Microsoft\Edge\User Data"

echo.
echo ============================================================
echo [11/18] BRAVE CACHE - ALL PROFILES
echo ============================================================
call :CleanChromium "%LOCALAPPDATA%\BraveSoftware\Brave-Browser\User Data"

echo.
echo ============================================================
echo [12/18] VIVALDI CACHE - ALL PROFILES
echo ============================================================
call :CleanChromium "%LOCALAPPDATA%\Vivaldi\User Data"

echo.
echo ============================================================
echo [13/18] OPERA / OPERA GX CACHE
echo ============================================================
call :CleanChromium "%APPDATA%\Opera Software\Opera Stable"
call :CleanChromium "%APPDATA%\Opera Software\Opera GX Stable"

echo.
echo ============================================================
echo [14/18] FIREFOX CACHE - ALL PROFILES
echo ============================================================
if exist "%LOCALAPPDATA%\Mozilla\Firefox\Profiles" (
    for /d %%P in ("%LOCALAPPDATA%\Mozilla\Firefox\Profiles\*") do (
        call :CleanDir "%%~fP\cache2"
        call :CleanDir "%%~fP\startupCache"
    )
)
call :CleanDir "%LOCALAPPDATA%\Mozilla\Firefox\Crash Reports"

echo.
echo ============================================================
echo [15/18] DISCORD CACHE
echo ============================================================
call :CleanDir "%APPDATA%\discord\Cache"
call :CleanDir "%APPDATA%\discord\Code Cache"
call :CleanDir "%APPDATA%\discord\GPUCache"
call :CleanDir "%APPDATA%\discord\Service Worker\CacheStorage"
call :CleanDir "%APPDATA%\discord\DawnCache"

echo.
echo ============================================================
echo [16/18] DEVELOPER PACKAGE CACHES
echo ============================================================
where npm >nul 2>&1
if "%errorlevel%"=="0" (
    echo Cleaning npm cache...
    call npm cache clean --force >nul 2>&1
)

where pip >nul 2>&1
if "%errorlevel%"=="0" (
    echo Cleaning pip cache...
    call pip cache purge >nul 2>&1
)

echo.
echo ============================================================
echo [17/18] DNS + RECYCLE BIN
echo ============================================================
echo Flushing DNS resolver cache...
ipconfig /flushdns

echo Emptying Recycle Bin...
powershell -NoProfile -ExecutionPolicy Bypass -Command "Clear-RecycleBin -Force -ErrorAction SilentlyContinue"

echo.
echo ============================================================
echo [18/18] WINDOWS BUILT-IN CLEANUP
echo ============================================================

echo Running Windows Disk Cleanup...
cleanmgr.exe /verylowdisk

echo Running Windows SilentCleanup task...
schtasks.exe /Run /TN "\Microsoft\Windows\DiskCleanup\SilentCleanup" >nul 2>&1

echo Running Windows component-store cleanup...
DISM.exe /Online /Cleanup-Image /StartComponentCleanup

echo.
echo ============================================================
echo                    CLEANUP COMPLETE
echo ============================================================
echo.

:: --- Calculate approximate free-space change ---
for /f "tokens=3" %%A in ('fsutil volume diskfree C: ^| findstr /i "Total # of free bytes"') do set "FREE_AFTER=%%A"

echo Cleanup has finished.
echo.
echo Some locked files may have been skipped. This is normal.
echo Caches will naturally grow again as Windows and applications
echo are used.
echo.
echo NOT automatically deleted:
echo   - Browser history/cookies/passwords/bookmarks
echo   - VS Code extensions/settings/projects
echo   - Personal files
echo   - Rust/Cargo caches
echo   - NuGet packages
echo   - node_modules
echo   - Prefetch
echo   - pagefile.sys / hiberfil.sys
echo.
echo For Windows.old, old installations, large unused apps, and
echo other large files, use Windows Storage/Disk Cleanup and review
echo the categories before deleting them.
echo.
pause
exit /b


:: ============================================================
:: FUNCTION: CleanDir
:: Deletes contents of a directory but leaves the directory itself.
:: ============================================================
:CleanDir
if not exist "%~1" exit /b
echo Cleaning: %~1
del /f /s /q "%~1\*" >nul 2>&1
for /d %%D in ("%~1\*") do rd /s /q "%%D" >nul 2>&1
exit /b


:: ============================================================
:: FUNCTION: CleanChromium
:: Cleans cache-like folders from every Chromium profile.
:: It does NOT delete History, Cookies, Login Data, Bookmarks, etc.
:: ============================================================
:CleanChromium
set "CHROME_ROOT=%~1"
if not exist "%CHROME_ROOT%" exit /b

for /d %%P in ("%CHROME_ROOT%\*") do (
    call :CleanDir "%%~fP\Cache"
    call :CleanDir "%%~fP\Code Cache"
    call :CleanDir "%%~fP\GPUCache"
    call :CleanDir "%%~fP\DawnCache"
    call :CleanDir "%%~fP\Service Worker\CacheStorage"
    call :CleanDir "%%~fP\Service Worker\ScriptCache"
    call :CleanDir "%%~fP\ShaderCache"
)
exit /b
