@echo off
setlocal enabledelayedexpansion

echo.
echo =======================================
echo Probe Windows Build Script
echo =======================================
echo.

set "BUILD_LOG=%TEMP%\probe-build-windows.log"
echo Probe Windows Build Started at %date% %time% > "%BUILD_LOG%"

:: Set script directory and repo root
set "SCRIPT_DIR=%~dp0"
set "REPO_ROOT=%SCRIPT_DIR%"
cd /d "%REPO_ROOT%"

echo [1/5] Checking prerequisites...
echo [1/5] Checking prerequisites... >> "%BUILD_LOG%"

:: Check for Rust installation
echo   - Checking for Rust installation...
where /q rustc
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Rust is not installed or not in PATH >> "%BUILD_LOG%"
    echo [ERROR] Rust is not installed or not in PATH.
    echo         Please install Rust from https://www.rust-lang.org/tools/install
    echo         Run the following command in your browser:
    echo         https://static.rust-lang.org/rustup/rustup-init.exe
    exit /b 1
) else (
    rustc --version >> "%BUILD_LOG%" 2>&1
    echo   - Rust is installed: !ERRORLEVEL!
)

:: Check for MSYS2 installation
echo   - Checking for MSYS2 installation...
if exist "C:\msys64\usr\bin\bash.exe" (
    echo   - MSYS2 is installed.
    echo MSYS2 is installed >> "%BUILD_LOG%"
) else (
    echo   - [WARNING] MSYS2 does not appear to be installed at the default location.
    echo   - This is required for GCC compiler toolchain.
    echo   - Install MSYS2 using Windows Package Manager:
    echo   - winget install MSYS2.MSYS2
    echo   - Then install MinGW toolchain by running:
    echo   - pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain
    echo   - And add C:\msys64\ucrt64\bin to your PATH
    echo MSYS2 might not be installed >> "%BUILD_LOG%"
)

:: Check if GCC is available
echo   - Checking for GCC compiler...
where /q gcc
if %ERRORLEVEL% neq 0 (
    echo   - [WARNING] GCC compiler not found in PATH.
    echo   - If you encounter build errors related to 'gcc.exe', you will need to:
    echo   - 1. Ensure MSYS2 is installed: winget install MSYS2.MSYS2
    echo   - 2. Open MSYS2 MSYS shell from Start Menu
    echo   - 3. Run: pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain
    echo   - 4. Add C:\msys64\ucrt64\bin to your system PATH
    echo GCC not found in PATH >> "%BUILD_LOG%"
) else (
    gcc --version >> "%BUILD_LOG%" 2>&1
    echo   - GCC compiler found: !ERRORLEVEL!
)

:: Check for Visual Studio Build Tools
echo   - Checking for Visual Studio 2022 C++ Build Tools...
reg query "HKLM\SOFTWARE\Microsoft\VisualStudio\SxS\VS7" /v "17.0" >nul 2>&1
set VS2022_INSTALLED=%ERRORLEVEL%

reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\VisualStudio\SxS\VS7" /v "17.0" >nul 2>&1
set VS2022_INSTALLED_WOW=%ERRORLEVEL%

reg query "HKLM\SOFTWARE\Microsoft\VisualStudio\SxS\VC7" /v "17.0" >nul 2>&1
set VS2022_VC_INSTALLED=%ERRORLEVEL%

if %VS2022_INSTALLED% equ 0 (
    echo   - Visual Studio 2022 is installed.
    echo VS2022 is installed >> "%BUILD_LOG%"
) else if %VS2022_INSTALLED_WOW% equ 0 (
    echo   - Visual Studio 2022 is installed (WOW64).
    echo VS2022 is installed (WOW64) >> "%BUILD_LOG%"
) else if %VS2022_VC_INSTALLED% equ 0 (
    echo   - Visual Studio 2022 C++ components are installed.
    echo VS2022 C++ components are installed >> "%BUILD_LOG%"
) else (
    echo   - [WARNING] Visual Studio 2022 Build Tools might not be installed.
    echo   - This could cause compilation issues with some dependencies.
    echo   - Download and install Build Tools from:
    echo   - https://visualstudio.microsoft.com/visual-cpp-build-tools/
    echo   - Be sure to select "Desktop development with C++" workload.
    echo VS2022 Build Tools might not be installed >> "%BUILD_LOG%"
)

:: Check for MSVC installation
echo   - Checking for Microsoft Visual C++ Redistributable...
reg query "HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" /v Version >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo   - Microsoft Visual C++ Redistributable is installed.
    echo MSVC is installed >> "%BUILD_LOG%"
) else (
    reg query "HKLM\SOFTWARE\Classes\Installer\Dependencies\VC,redist.x64,amd64,14.29,bundle" /v Version >nul 2>&1
    if %ERRORLEVEL% equ 0 (
        echo   - Microsoft Visual C++ Redistributable is installed.
        echo MSVC is installed (alternate registry key) >> "%BUILD_LOG%"
    ) else (
        echo   - [WARNING] Microsoft Visual C++ Redistributable might not be installed.
        echo   - The build can proceed, but the binary may not run without installing:
        echo   - https://aka.ms/vs/17/release/vc_redist.x64.exe
        echo MSVC might not be installed, but continuing build >> "%BUILD_LOG%"
    )
)

echo.
echo [2/5] Cleaning previous build...
echo [2/5] Cleaning previous build... >> "%BUILD_LOG%"
cargo clean
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Failed to clean project >> "%BUILD_LOG%"
    echo [ERROR] Failed to clean project. See build log for details: "%BUILD_LOG%"
    exit /b 1
)

echo.
echo [3/5] Building Probe with Windows-optimized settings...
echo [3/5] Building Probe with Windows-optimized settings... >> "%BUILD_LOG%"
echo   - This may take a few minutes...
echo.

:: Build with release settings
cargo build --release >> "%BUILD_LOG%" 2>&1
set BUILD_ERROR=%ERRORLEVEL%

if %BUILD_ERROR% neq 0 (
    echo [ERROR] Build failed with code %BUILD_ERROR%. >> "%BUILD_LOG%"
    echo [ERROR] Build failed with code %BUILD_ERROR%.
    
    :: Check for specific error patterns in the log
    findstr /C:"failed to find tool \"gcc.exe\"" "%BUILD_LOG%" >nul 2>&1
    if %ERRORLEVEL% equ 0 (
        echo.
        echo [ERROR DETAILS] Missing GCC compiler.
        echo.
        echo This error occurs because the C/C++ compiler toolchain is missing.
        echo To fix this:
        echo.
        echo 1. Install MSYS2 using Windows Package Manager:
        echo    winget install MSYS2.MSYS2
        echo.
        echo 2. Open the "MSYS2 MSYS2" shortcut from the Start Menu
        echo.
        echo 3. Install the MinGW-w64 toolchain by running:
        echo    pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain
        echo.
        echo 4. Add the compiler to your PATH:
        echo    - Open Windows Settings ^> System ^> About ^> Advanced system settings
        echo    - Click on "Environment Variables"
        echo    - Edit the "Path" variable under either User or System variables
        echo    - Add C:\msys64\ucrt64\bin to the list
        echo    - Click OK and close all dialogs
        echo.
        echo 5. Restart your terminal/PowerShell and run this script again
        echo.
    ) else (
        findstr /C:"requires nightly" "%BUILD_LOG%" >nul 2>&1
        if %ERRORLEVEL% equ 0 (
            echo [ERROR DETAILS] This project requires the nightly Rust toolchain.
            echo To fix this, run:
            echo.
            echo rustup install nightly
            echo rustup default nightly
            echo.
            echo Then run this script again.
        ) else (
            echo See build log for details: "%BUILD_LOG%"
        )
    )
    
    exit /b 1
)

echo.
echo [4/5] Verifying build...
echo [4/5] Verifying build... >> "%BUILD_LOG%"

:: Check if executable exists
if not exist "target\release\probe.exe" (
    echo [ERROR] Build completed but executable not found >> "%BUILD_LOG%"
    echo [ERROR] Build completed but executable not found at target\release\probe.exe
    exit /b 1
)

echo   - Executable created successfully at target\release\probe.exe
echo   - Executable created successfully >> "%BUILD_LOG%"

echo.
echo [5/5] Testing executable...
echo [5/5] Testing executable... >> "%BUILD_LOG%"

:: Test if executable runs
target\release\probe.exe --help > nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [WARNING] Executable built but test run failed. You may need to install: >> "%BUILD_LOG%"
    echo [WARNING] Executable built but test run failed.
    echo           This may be due to missing Microsoft Visual C++ Redistributable.
    echo           Please install it from: https://aka.ms/vs/17/release/vc_redist.x64.exe
) else (
    echo   - Executable test successful
    echo   - Executable test successful >> "%BUILD_LOG%"
)

echo.
echo =======================================
echo Build completed successfully!
echo =======================================
echo.
echo The probe executable is available at:
echo   %REPO_ROOT%target\release\probe.exe
echo.
echo Build log saved to: "%BUILD_LOG%"
echo.
echo Next steps:
echo   - Run: .\target\release\probe.exe search "query" path\to\search
echo   - Start MCP server: cd mcp ^&^& npm run build ^&^& node build\index.js
echo.

endlocal