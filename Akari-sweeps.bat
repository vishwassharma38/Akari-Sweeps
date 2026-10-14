@echo off
setlocal EnableExtensions EnableDelayedExpansion
title Akari-Sweeps_V0.1

:: ============================================================
:: Akari-Sweeps_V0.1
::
:: Designed for Windows 10.
::
:: EXECUTION FLOW:
::   1. Confirmation prompt, then the junk-cleanup phase.
::   2. The cleanup phase ends. Nothing else runs automatically.
::   3. A separate Y/N prompt asks whether to run the Windows OS
::      image repair phase. Repair starts ONLY after an explicit Y.
::   4. A final execution summary is always displayed before exit.
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
:: OPTIONAL REPAIR PHASE (only after explicit confirmation):
::   - DISM component-store health check
::   - DISM component-store corruption scan
::   - DISM component-store repair
::   - SFC protected system-file scan and repair
::   - CHKDSK online filesystem health scan
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
::   The repair phase does NOT intentionally modify personal files.
::   DISM /RestoreHealth may use Windows Update as a repair source
::   when a repair source is otherwise unavailable.
:: ============================================================

:: --- Request Administrator privileges ---
:: The relaunched copy receives the argument ELEVATED. If that copy
:: is still not elevated, it stops instead of relaunching again.
fltmc >nul 2>&1
if not "%errorlevel%"=="0" (
    if /i "%~1"=="ELEVATED" (
        echo ERROR: Administrator privileges could not be obtained.
        echo Right-click the script and choose Run as administrator.
        pause
        exit /b 1
    )
    echo Requesting Administrator privileges...
    powershell -NoProfile -ExecutionPolicy Bypass -Command "Start-Process -FilePath '%~f0' -ArgumentList 'ELEVATED' -Verb RunAs"
    exit /b
)

:: --- Initialise run state used by the final summary ---
set "CLEANUP_STATUS=not started"
set "REPAIR_STATUS=not offered"
set "REPAIR_ENTERED="
set "FREE_BEFORE_KB="
set "FREE_AFTER_KB="
set /a OK_COUNT=0, ERR_COUNT=0, WARN_COUNT=0
set /a DIRS_DONE=0, DIRS_MISSING=0, FILES_REMOVED=0, KB_REMOVED=0, FILES_LEFT=0, MEASURE_FAIL=0
set "R_NPM=not run"
set "R_PIP=not run"
set "R_DNS=not run"
set "R_BIN=not run"
set "R_CLEANMGR=not run"
set "R_SILENT=not run"
set "R_COMPCLEAN=not run"
set "R_CHECK=not run"
set "R_SCAN=not run"
set "R_RESTORE=not run"
set "R_SFC=not run"
set "R_CHKDSK=not run"
set "OVERALL_RC=0"

echo.
echo ============================================================
echo                      Akari-Sweeps_V0.1
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
echo After the cleanup you will be asked separately whether to run
echo the optional Windows OS image repair. It never starts on its own.
echo.

:: ============================================================
:: MAIN FLOW
:: Cleanup phase -> (stop) -> repair confirmation -> summary.
:: choice accepts only Y or N and ignores every other key, so an
:: invalid key simply keeps the prompt waiting. If no answer can be
:: read at all, the safe default is to NOT run the phase.
:: ============================================================
choice /C YN /N /M "Start cleanup? [Y/N]: "
set "CH=%errorlevel%"
if "%CH%"=="1" (
    call :PhaseCleanup
    call :RepairConfirm
) else (
    set "CLEANUP_STATUS=not run - cleanup was not confirmed"
    echo.
    echo Cleanup was not started. No changes were made.
)

call :ShowSummary
echo.
pause
exit /b %OVERALL_RC%


:: ============================================================
:: PHASE 1: JUNK CLEANUP
:: ============================================================
:PhaseCleanup
set "CLEANUP_STATUS=running"
call :GetFreeKB FREE_BEFORE_KB

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
echo Cleaning Microsoft Store cache...
call :CleanDir "%LOCALAPPDATA%\Packages\Microsoft.WindowsStore_8wekyb3d8bbwe\LocalCache"
echo Microsoft Store cache cleanup complete.

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
    set "STEP_RC=!errorlevel!"
    if "!STEP_RC!"=="0" (
        set "R_NPM=cache cleaned - exit code 0"
        set /a OK_COUNT+=1
    ) else (
        set "R_NPM=FAILED - exit code !STEP_RC!"
        call :AddError "npm cache clean failed with exit code !STEP_RC!."
    )
) else (
    set "R_NPM=npm not installed - skipped"
)

where pip >nul 2>&1
if "%errorlevel%"=="0" (
    echo Cleaning pip cache...
    call pip cache purge >nul 2>&1
    set "STEP_RC=!errorlevel!"
    if "!STEP_RC!"=="0" (
        set "R_PIP=cache purged - exit code 0"
        set /a OK_COUNT+=1
    ) else (
        set "R_PIP=FAILED - exit code !STEP_RC!"
        call :AddError "pip cache purge failed with exit code !STEP_RC!."
    )
) else (
    set "R_PIP=pip not installed - skipped"
)

echo.
echo ============================================================
echo [17/18] DNS + RECYCLE BIN
echo ============================================================
echo Flushing DNS resolver cache...
ipconfig /flushdns
set "STEP_RC=%errorlevel%"
if "%STEP_RC%"=="0" (
    set "R_DNS=flushed - exit code 0"
    set /a OK_COUNT+=1
) else (
    set "R_DNS=FAILED - exit code %STEP_RC%"
    call :AddError "DNS cache flush failed with exit code %STEP_RC%."
)

echo Emptying Recycle Bin...
powershell -NoProfile -ExecutionPolicy Bypass -Command "Clear-RecycleBin -Force -ErrorAction SilentlyContinue"
set "STEP_RC=%errorlevel%"
if "%STEP_RC%"=="0" (
    set "R_BIN=command issued - exit code 0 - result not independently verified"
    set /a OK_COUNT+=1
) else (
    set "R_BIN=command returned exit code %STEP_RC% - result unknown"
    call :AddWarning "Recycle Bin command returned exit code %STEP_RC%; the result could not be verified."
)

echo.
echo ============================================================
echo [18/18] WINDOWS BUILT-IN CLEANUP
echo ============================================================
echo Running Windows Disk Cleanup...
start "" /wait cleanmgr.exe /verylowdisk
set "STEP_RC=%errorlevel%"
if "%STEP_RC%"=="0" (
    set "R_CLEANMGR=process finished - exit code 0 - amount freed is not reported by Windows"
    set /a OK_COUNT+=1
) else (
    set "R_CLEANMGR=process finished with exit code %STEP_RC% - result unknown"
    call :AddWarning "Disk Cleanup exited with code %STEP_RC%; its exit codes are not documented, so the result is unknown."
)

echo Running Windows SilentCleanup task...
schtasks.exe /Run /TN "\Microsoft\Windows\DiskCleanup\SilentCleanup" >nul 2>&1
set "STEP_RC=%errorlevel%"
if "%STEP_RC%"=="0" (
    echo Waiting for the SilentCleanup task to finish...
    call :WaitSilentCleanup
) else (
    set "R_SILENT=NOT STARTED - schtasks exit code %STEP_RC%"
    call :AddWarning "SilentCleanup task could not be started; schtasks exit code %STEP_RC%."
)

echo Running Windows component-store cleanup...
DISM.exe /Online /Cleanup-Image /StartComponentCleanup
set "STEP_RC=%errorlevel%"
call :RecordDism "StartComponentCleanup" "%STEP_RC%" R_COMPCLEAN

echo.
echo ============================================================
echo                    CLEANUP COMPLETE
echo ============================================================
echo.

:: --- Free space after cleanup, captured before any repair work runs ---
call :GetFreeKB FREE_AFTER_KB

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

set /a OK_COUNT+=1
if %FILES_LEFT% GTR 0 call :AddWarning "%FILES_LEFT% file[s] in the cleaned folders could not be deleted [locked or in use]."
if %MEASURE_FAIL% GTR 0 call :AddWarning "Size measurement failed for %MEASURE_FAIL% folder[s]; the folder totals may be understated."
set "CLEANUP_STATUS=completed"
exit /b


:: ============================================================
:: PHASE 2: OPTIONAL WINDOWS OS IMAGE REPAIR - CONFIRMATION
:: This is the ONLY place the repair phase can be started.
:: ============================================================
:RepairConfirm
echo.
echo ============================================================
echo          WINDOWS OS IMAGE REPAIR - CONFIRMATION
echo ============================================================
echo.
echo The junk cleanup phase is finished. Nothing else will run
echo unless you choose to continue.
echo.
echo Windows OS image repair would run these steps, in this order:
echo   1. DISM /CheckHealth
echo   2. DISM /ScanHealth
echo   3. DISM /RestoreHealth   - may download repair files via Windows Update
echo   4. SFC /scannow
echo   5. CHKDSK %SystemDrive% /scan
echo.
echo This can take a long time. Each step runs exactly once.
echo DISM and SFC redraw their progress on one line while they work;
echo that is normal and is not a repeat of the step.
echo.
echo Press Y to start the repair, or N to skip it.
echo.
choice /C YN /N /M "Proceed with Windows OS image repair? [Y/N]: "
set "CH=%errorlevel%"
if "%CH%"=="1" (
    call :PhaseRepair
) else if "%CH%"=="2" (
    set "REPAIR_STATUS=skipped by user"
    echo.
    echo Windows OS image repair skipped.
) else (
    set "REPAIR_STATUS=skipped - no valid answer could be read"
    echo.
    echo No valid answer could be read. Windows OS image repair skipped.
    call :AddWarning "Repair confirmation could not be read, so the repair phase was skipped."
)
exit /b


:: ============================================================
:: PHASE 2: WINDOWS SYSTEM HEALTH / REPAIR
:: Reached only through :RepairConfirm after an explicit Y.
:: Each operation below appears exactly once and runs exactly once.
:: ============================================================
:PhaseRepair
if defined REPAIR_ENTERED (
    call :AddWarning "The repair phase had already started in this run and was not started a second time."
    exit /b
)
set "REPAIR_ENTERED=1"
set "REPAIR_STATUS=started"
set "ERR_AT_REPAIR_START=%ERR_COUNT%"

echo.
echo ============================================================
echo          WINDOWS SYSTEM HEALTH / REPAIR
echo ============================================================
echo.
echo Windows will now check and repair system components
echo and protected system files.
echo.
echo This may take some time. Do not close this window.
echo.

echo ============================================================
echo [Repair 1/5] DISM - COMPONENT STORE HEALTH CHECK
echo ============================================================
echo Started at %TIME%
echo Running DISM /CheckHealth...
echo.
DISM.exe /Online /Cleanup-Image /CheckHealth
set "STEP_RC=%errorlevel%"
echo.
call :RecordDism "CheckHealth" "%STEP_RC%" R_CHECK

echo.
echo ============================================================
echo [Repair 2/5] DISM - COMPONENT STORE DEEP SCAN
echo ============================================================
echo Started at %TIME%
echo Running DISM /ScanHealth...
echo.
DISM.exe /Online /Cleanup-Image /ScanHealth
set "STEP_RC=%errorlevel%"
echo.
call :RecordDism "ScanHealth" "%STEP_RC%" R_SCAN

echo.
echo ============================================================
echo [Repair 3/5] DISM - COMPONENT STORE REPAIR
echo ============================================================
echo Started at %TIME%
echo Running DISM /RestoreHealth...
echo.
DISM.exe /Online /Cleanup-Image /RestoreHealth
set "STEP_RC=%errorlevel%"
echo.
call :RecordDism "RestoreHealth" "%STEP_RC%" R_RESTORE

echo.
echo ============================================================
echo [Repair 4/5] SYSTEM FILE CHECKER - SFC
echo ============================================================
echo Started at %TIME%
echo Running SFC /scannow...
echo.
sfc /scannow
set "STEP_RC=%errorlevel%"
echo.
if "%STEP_RC%"=="0" (
    set "R_SFC=completed - exit code 0"
    set /a OK_COUNT+=1
    echo SFC completed with exit code 0.
) else (
    set "R_SFC=problem reported - exit code %STEP_RC% - review SFC output and CBS.log"
    call :AddError "SFC returned exit code %STEP_RC%; review its output and CBS.log."
    echo SFC returned exit code %STEP_RC%.
)

echo.
echo ============================================================
echo [Repair 5/5] FILESYSTEM HEALTH CHECK
echo ============================================================
echo Started at %TIME%
echo Running CHKDSK online scan on %SystemDrive%...
echo.
chkdsk %SystemDrive% /scan
set "STEP_RC=%errorlevel%"
echo.
if "%STEP_RC%"=="0" (
    set "R_CHKDSK=completed - no errors found - exit code 0"
    set /a OK_COUNT+=1
    echo CHKDSK online scan completed. No errors were found.
) else if "%STEP_RC%"=="1" (
    set "R_CHKDSK=completed - errors were found and fixed - exit code 1"
    set /a OK_COUNT+=1
    call :AddWarning "CHKDSK reported that it found and fixed filesystem errors."
    echo CHKDSK completed. Errors were found and fixed.
) else if "%STEP_RC%"=="2" (
    set "R_CHKDSK=completed - informational exit code 2"
    set /a OK_COUNT+=1
    echo CHKDSK completed with informational exit code 2.
) else if "%STEP_RC%"=="3" (
    set "R_CHKDSK=NOT COMPLETE - exit code 3 - could not check or errors remain"
    call :AddError "CHKDSK could not complete or errors remain [exit code 3]."
    echo CHKDSK returned exit code 3.
) else (
    set "R_CHKDSK=FAILED - unexpected exit code %STEP_RC%"
    call :AddError "CHKDSK returned unexpected exit code %STEP_RC%."
    echo CHKDSK returned exit code %STEP_RC%.
)

echo.
echo ============================================================
echo         WINDOWS HEALTH / REPAIR COMPLETE
echo ============================================================
echo.
echo The Windows health and repair phase has finished.
echo.
echo IMPORTANT:
echo   - If DISM or SFC reported problems that could not be repaired,
echo     review their output before considering Windows fully healthy.
echo   - CHKDSK /scan performs an online filesystem scan. It does not
echo     force an offline repair or restart the computer.
echo.

if %ERR_COUNT% GTR %ERR_AT_REPAIR_START% (
    set "REPAIR_STATUS=ran - one or more operations reported a failure"
) else (
    set "REPAIR_STATUS=ran - every operation completed without a reported failure"
)
exit /b


:: ============================================================
:: FINAL EXECUTION SUMMARY
:: Always shown, whether cleanup was declined, repair was skipped,
:: repair succeeded, or an operation failed.
:: ============================================================
:ShowSummary
set "OVERALL_RC=0"
if "%CLEANUP_STATUS%"=="completed" (
    set "OVERALL=COMPLETED SUCCESSFULLY"
    if %WARN_COUNT% GTR 0 set "OVERALL=COMPLETED WITH WARNINGS"
) else (
    set "OVERALL=NOT RUN - no changes were made"
)
if %ERR_COUNT% GTR 0 (
    if %OK_COUNT% GTR 0 (
        set "OVERALL=COMPLETED PARTIALLY - failed operations: %ERR_COUNT%"
        set "OVERALL_RC=1"
    ) else (
        set "OVERALL=ENCOUNTERED AN ERROR - no operation succeeded"
        set "OVERALL_RC=2"
    )
)

echo.
echo ============================================================
echo              Akari-Sweeps_V0.1 - EXECUTION SUMMARY
echo ============================================================
echo.
echo  OVERALL STATUS : %OVERALL%
echo.
echo ------------------------------------------------------------
echo  JUNK CLEANUP
echo ------------------------------------------------------------
echo  Status                          : %CLEANUP_STATUS%
if "%CLEANUP_STATUS%"=="completed" call :SummaryCleanup
echo.
echo ------------------------------------------------------------
echo  WINDOWS OS IMAGE REPAIR
echo ------------------------------------------------------------
echo  Status                          : %REPAIR_STATUS%
if defined REPAIR_ENTERED call :SummaryRepair
echo.
echo ------------------------------------------------------------
echo  WARNINGS AND SKIPPED OPERATIONS
echo ------------------------------------------------------------
if %WARN_COUNT% EQU 0 (
    echo  None.
) else (
    call :SummaryWarnings
)
echo.
echo ------------------------------------------------------------
echo  ERRORS AND FAILURES
echo ------------------------------------------------------------
if %ERR_COUNT% EQU 0 (
    echo  None reported.
) else (
    call :SummaryErrors
)
echo.
echo  Values marked unavailable or not reported could not be
echo  measured reliably and are intentionally not estimated.
echo ============================================================
exit /b

:SummaryCleanup
set "S_FREED=unavailable"
set "S_FB=unavailable"
set "S_FA=unavailable"
set "S_NET=unavailable"
call :FormatKB "%KB_REMOVED%" S_FREED
if defined FREE_BEFORE_KB call :FormatKB "%FREE_BEFORE_KB%" S_FB
if defined FREE_AFTER_KB call :FormatKB "%FREE_AFTER_KB%" S_FA
if defined FREE_BEFORE_KB if defined FREE_AFTER_KB call :NetDelta
echo  Target folders cleaned          : %DIRS_DONE%
echo  Target folders not present      : %DIRS_MISSING% [skipped]
echo  Files removed [measured]        : %FILES_REMOVED%
echo  Space freed by folder cleanup   : %S_FREED% [measured]
echo  Files remaining in those folders: %FILES_LEFT% [locked or in use]
echo.
echo  Free space on %SystemDrive%
echo    Before cleanup                : %S_FB%
echo    After cleanup                 : %S_FA%
echo    Net change                    : %S_NET%
echo    [The net change includes any other activity on the drive, so it
echo     can differ from the folder total above.]
echo.
echo  Other cleanup operations [the amount freed is not reported]:
echo    npm cache clean               : %R_NPM%
echo    pip cache purge               : %R_PIP%
echo    DNS cache flush               : %R_DNS%
echo    Recycle Bin                   : %R_BIN%
echo    Disk Cleanup [cleanmgr]       : %R_CLEANMGR%
echo    SilentCleanup task            : %R_SILENT%
echo    Component-store cleanup [DISM]: %R_COMPCLEAN%
if %MEASURE_FAIL% GTR 0 echo  Measurement unavailable for %MEASURE_FAIL% folder[s]; folder totals may be understated.
exit /b

:SummaryRepair
echo  DISM /CheckHealth               : %R_CHECK%
echo  DISM /ScanHealth                : %R_SCAN%
echo  DISM /RestoreHealth             : %R_RESTORE%
echo  SFC /scannow                    : %R_SFC%
echo  CHKDSK /scan                    : %R_CHKDSK%
echo  [For CheckHealth and ScanHealth, completed means the command ran
echo   to completion. Whether corruption was found appears only in
echo   DISM's own output, which this script does not parse.]
exit /b

:SummaryWarnings
for /l %%i in (1,1,%WARN_COUNT%) do echo   - !WARNMSG_%%i!
exit /b

:SummaryErrors
for /l %%i in (1,1,%ERR_COUNT%) do echo   - !ERRMSG_%%i!
exit /b


:: ============================================================
:: HELPERS: result recording
:: ============================================================
:AddWarning
set /a WARN_COUNT+=1
set "WARNMSG_%WARN_COUNT%=%~1"
exit /b

:AddError
set /a ERR_COUNT+=1
set "ERRMSG_%ERR_COUNT%=%~1"
exit /b

:: Arguments: DISM operation name, DISM exit code, and the name of the
:: variable that receives the result text. 0 = success, 3010 = success
:: plus restart needed.
:RecordDism
if "%~2"=="0" (
    set "%~3=completed - exit code 0"
    set /a OK_COUNT+=1
    echo DISM %~1 completed successfully.
) else if "%~2"=="3010" (
    set "%~3=completed - exit code 3010 - restart required"
    set /a OK_COUNT+=1
    call :AddWarning "DISM %~1 completed, but Windows reports that a restart is required."
    echo DISM %~1 completed. A restart is required.
) else (
    set "%~3=FAILED - exit code %~2"
    call :AddError "DISM %~1 failed with exit code %~2."
    echo DISM %~1 FAILED with exit code %~2.
)
exit /b


:: ============================================================
:: HELPERS: waiting for the SilentCleanup task
:: schtasks /Run only starts the task and returns immediately, so this
:: waits for the task to leave the Running state before continuing.
:: Exit codes from the PowerShell wait: 0 finished, 2 still running
:: after 20 minutes, 1 state could not be read.
:: ============================================================
:WaitSilentCleanup
powershell -NoProfile -Command "try { Start-Sleep -Seconds 3; $sw = [Diagnostics.Stopwatch]::StartNew(); while ((Get-ScheduledTask -TaskPath '\Microsoft\Windows\DiskCleanup\' -TaskName 'SilentCleanup' -ErrorAction Stop).State -eq 'Running') { if ($sw.Elapsed.TotalMinutes -ge 20) { exit 2 }; Start-Sleep -Seconds 5 }; exit 0 } catch { exit 1 }"
set "STEP_RC=%errorlevel%"
if "%STEP_RC%"=="0" (
    set "R_SILENT=task finished - amount freed is not reported by Windows"
    set /a OK_COUNT+=1
) else if "%STEP_RC%"=="2" (
    set "R_SILENT=still running after 20 minutes - not waited for any longer"
    call :AddWarning "SilentCleanup was still running after 20 minutes; the script continued without waiting."
) else (
    set "R_SILENT=started - completion could not be verified"
    call :AddWarning "SilentCleanup was started, but its completion could not be verified."
)
exit /b


:: ============================================================
:: HELPERS: measurement
:: Sizes are handled in KB because batch arithmetic is 32-bit.
:: ============================================================

:: Argument: name of the variable that receives the free space of the
:: system drive in KB.
:: The variable is left undefined if the value cannot be read.
:GetFreeKB
set "%~1="
for /f "usebackq delims=" %%F in (`powershell -NoProfile -Command "[int64][math]::Floor((New-Object System.IO.DriveInfo -ArgumentList $env:SystemDrive).AvailableFreeSpace / 1024)"`) do set "%~1=%%F"
exit /b

:: Arguments: path, variable for file count, variable for size in KB.
:: Both variables are left undefined if the folder cannot be measured.
:MeasureDir
set "AS_MPATH=%~1"
set "%~2="
set "%~3="
for /f "usebackq tokens=1,2" %%A in (`powershell -NoProfile -Command "$m = Get-ChildItem -LiteralPath $env:AS_MPATH -Recurse -Force -File -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum; '{0} {1}' -f $m.Count, [int64]([math]::Floor(([double]$m.Sum) / 1024))"`) do (
    set "%~2=%%A"
    set "%~3=%%B"
)
exit /b

:: Arguments: size in KB (may be negative), variable that receives text.
:FormatKB
set "%~2=unavailable"
set "AS_KB=%~1"
if not defined AS_KB exit /b
for /f "usebackq delims=" %%F in (`powershell -NoProfile -Command "$k = [double]$env:AS_KB; if ([math]::Abs($k) -ge 1048576) { '{0:N2} GB' -f ($k / 1048576) } elseif ([math]::Abs($k) -ge 1024) { '{0:N2} MB' -f ($k / 1024) } else { '{0:N0} KB' -f $k }"`) do set "%~2=%%F"
exit /b

:: Net free-space change in KB, formatted into S_NET.
:NetDelta
set "AS_A=%FREE_AFTER_KB%"
set "AS_B=%FREE_BEFORE_KB%"
set "S_DELTA="
for /f "usebackq delims=" %%F in (`powershell -NoProfile -Command "([int64]$env:AS_A) - ([int64]$env:AS_B)"`) do set "S_DELTA=%%F"
if defined S_DELTA call :FormatKB "%S_DELTA%" S_NET
exit /b


:: ============================================================
:: FUNCTION: CleanDir
:: Deletes contents of a directory but leaves the directory itself.
:: Files and size are measured before and after so the summary can
:: report what was actually removed rather than what was attempted.
:: ============================================================
:CleanDir
if not exist "%~1" (
    set /a DIRS_MISSING+=1
    exit /b
)
echo Cleaning: %~1
set "CD_PATH=%~1"
call :MeasureDir "%CD_PATH%" CD_B_ITEMS CD_B_KB
del /f /s /q "%~1\*" >nul 2>&1
for /d %%D in ("%~1\*") do rd /s /q "%%D" >nul 2>&1
set /a DIRS_DONE+=1
call :TallyDir
exit /b

:: Compares the measurement taken before deletion with one taken after.
:TallyDir
set "CD_A_ITEMS="
set "CD_A_KB="
if not defined CD_B_ITEMS (
    set /a MEASURE_FAIL+=1
    exit /b
)
if "%CD_B_ITEMS%"=="0" exit /b
call :MeasureDir "%CD_PATH%" CD_A_ITEMS CD_A_KB
if not defined CD_A_ITEMS (
    set /a MEASURE_FAIL+=1
    exit /b
)
set /a CD_ITEMS=CD_B_ITEMS-CD_A_ITEMS
set /a CD_KB=CD_B_KB-CD_A_KB
if %CD_ITEMS% LSS 0 set "CD_ITEMS=0"
if %CD_KB% LSS 0 set "CD_KB=0"
set /a FILES_REMOVED+=CD_ITEMS
set /a KB_REMOVED+=CD_KB
set /a FILES_LEFT+=CD_A_ITEMS
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