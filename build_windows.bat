@echo off
setlocal EnableExtensions EnableDelayedExpansion
cd /d "%~dp0"

rem ============================================================
rem  DS5Dongle BL618 - Windows build script
rem
rem  Usage: build_windows.bat [build|rebuild|both|clean|flash [COMx]]
rem    build   - incremental build (default, USB_SPEED=fs)
rem    rebuild - clean + full build
rem    both    - build Full-Speed then High-Speed variants
rem    clean   - remove build directory
rem    flash   - flash via serial, e.g. build_windows.bat flash COM5
rem
rem  Environment overrides:
rem    BL_SDK_BASE     path to the DS5Dongle BL618 SDK fork
rem    TOOLCHAIN_PATH  path to the T-Head Windows toolchain
rem    BOARD_TYPE      aim61 (default) | lctech616 | m0sdock
rem    USB_SPEED       fs (default) | hs
rem    DS5_LOG_LEVEL   0..3 (default 2; use 3 only for bounded diagnostics)
rem ============================================================

if "%BL_SDK_BASE%"=="" set "BL_SDK_BASE=%~dp0..\bouffalo_sdk"
if "%TOOLCHAIN_PATH%"=="" set "TOOLCHAIN_PATH=%USERPROFILE%\Desktop\toolchain_gcc_t-head_windows"
if "%BOARD_TYPE%"=="" set "BOARD_TYPE=aim61"
if "%USB_SPEED%"=="" set "USB_SPEED=fs"
if "%DS5_LOG_LEVEL%"=="" set "DS5_LOG_LEVEL=2"
if "%NUMBER_OF_PROCESSORS%"=="" set "NUMBER_OF_PROCESSORS=8"

rem ---- sanity checks ----
if not exist "%BL_SDK_BASE%\project.build" (
    echo [ERROR] SDK not found at "%BL_SDK_BASE%"
    echo         Set BL_SDK_BASE to the DS5Dongle BL618 SDK fork ^(see README step 1^).
    exit /b 1
)
if not exist "%TOOLCHAIN_PATH%\bin\riscv64-unknown-elf-gcc.exe" (
    echo [ERROR] RISC-V toolchain not found at "%TOOLCHAIN_PATH%"
    echo         git clone https://gitee.com/bouffalolab/toolchain_gcc_t-head_windows.git
    echo         or set TOOLCHAIN_PATH to override.
    exit /b 1
)

rem ---- PATH ----
set "PATH=%~dp0tools;%TOOLCHAIN_PATH%\bin;%BL_SDK_BASE%\tools\make;%BL_SDK_BASE%\tools\cmake\bin;%BL_SDK_BASE%\tools\ninja;%PATH%"
rem GitHub-hosted Windows runners can pass Git Bash to GNU make as a command-line
rem variable.  Environment-only overrides lose to that value, so every top-level
rem make invocation below explicitly sets SHELL=cmd.exe.  Recursive $(MAKE)
rem calls inherit it and keep native drive-letter paths intact.

set "ACTION=%~1"
if "%ACTION%"=="" set "ACTION=build"

if /i "%ACTION%"=="clean" goto :clean
if /i "%ACTION%"=="flash" goto :flash
if /i "%ACTION%"=="both"  goto :both

rem build / rebuild
if /i "%ACTION%"=="rebuild" (
    if exist build make SHELL=cmd.exe clean 2>nul
)
call :build_one
if errorlevel 1 exit /b !errorlevel!
goto :done

:both
set "USB_SPEED=fs"
call :build_one
if errorlevel 1 exit /b !errorlevel!
set "USB_SPEED=hs"
call :build_one
if errorlevel 1 exit /b !errorlevel!
goto :done

:build_one
    rem ---- board mapping ----
    set "BOARD_LCTECH_616="
    set "BOARD_M0S_DOCK="
    if /i "%BOARD_TYPE%"=="m0sdock" (
        set "BOARD_M0S_DOCK=1"
    ) else if /i "%BOARD_TYPE%"=="aim61" (
        set "BOARD_LCTECH_616="
    ) else if /i "%BOARD_TYPE%"=="lctech616" (
        set "BOARD_LCTECH_616=1"
    ) else (
        echo [ERROR] Unknown BOARD_TYPE "%BOARD_TYPE%". Use aim61, lctech616, or m0sdock.
        exit /b 2
    )
    rem ---- usb speed mapping ----
    set "FORCE_FS="
    set "SPEED_SUFFIX="
    if /i "%USB_SPEED%"=="fs" (
        set "FORCE_FS=1"
    ) else if /i "%USB_SPEED%"=="hs" (
        set "SPEED_SUFFIX=-hs"
    ) else (
        echo [ERROR] Unknown USB_SPEED "%USB_SPEED%". Use fs or hs.
        exit /b 2
    )
    if not "%DS5_LOG_LEVEL%"=="0" if not "%DS5_LOG_LEVEL%"=="1" if not "%DS5_LOG_LEVEL%"=="2" if not "%DS5_LOG_LEVEL%"=="3" (
        echo [ERROR] Invalid DS5_LOG_LEVEL "%DS5_LOG_LEVEL%". Use 0, 1, 2, or 3.
        exit /b 2
    )
    rem ---- auto clean when board/usb/log config changes ----
    set "BUILD_KEY=%BOARD_TYPE%-%USB_SPEED%-log%DS5_LOG_LEVEL%"
    set "BOARD_STAMP=build\.board_type"
    if exist "%BOARD_STAMP%" (
        set "PREV_KEY="
        set /p PREV_KEY=<"%BOARD_STAMP%"
        if not "!PREV_KEY!"=="%BUILD_KEY%" (
            echo [build] Config changed ^(!PREV_KEY! -^> %BUILD_KEY%^), forcing clean...
            if exist build make SHELL=cmd.exe clean 2>nul
        )
    ) else if exist "build\CMakeCache.txt" (
        echo [build] Build stamp missing, forcing CMake reconfigure...
        make SHELL=cmd.exe clean 2>nul
    )
    echo [build] Target: %BOARD_TYPE%  USB: %USB_SPEED%  LOG: %DS5_LOG_LEVEL%
    make SHELL=cmd.exe -j%NUMBER_OF_PROCESSORS%
    if errorlevel 1 exit /b 1
    if not exist build mkdir build
    >"%BOARD_STAMP%" echo %BUILD_KEY%
    set "OUT_DIR=firmware\%BOARD_TYPE%"
    if not exist "%OUT_DIR%" mkdir "%OUT_DIR%"
    copy /y "build\build_out\ds5dongle_bl618_bl616.bin" "%OUT_DIR%\ds5dongle-%BOARD_TYPE%%SPEED_SUFFIX%.bin" >nul
    copy /y "build\build_out\boot2_bl616_isp_release_v8.1.8.bin" "%OUT_DIR%\" >nul
    copy /y "build\build_out\partition.bin" "%OUT_DIR%\" >nul
    echo [build] Output: %OUT_DIR%\ds5dongle-%BOARD_TYPE%%SPEED_SUFFIX%.bin
    goto :eof

:clean
make SHELL=cmd.exe clean
goto :done

:flash
set "COMX=%~2"
if "%COMX%"=="" set "COMX=COM5"
make SHELL=cmd.exe flash COMX=%COMX%
goto :done

:done
echo [build] Done.
exit /b 0
