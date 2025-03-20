# Windows Compatibility for Probe

This document outlines Windows-specific requirements and troubleshooting steps for the Probe code search tool.

## Windows Requirements

On Windows, the probe binary requires Microsoft Visual C++ Redistributable 2015-2022 (x64) to run properly. The package will attempt to detect if this is installed during the installation process.

If you encounter errors like `The program can't start because VCRUNTIME140.dll is missing from your computer` or similar, you need to:

1. Download and install the Microsoft Visual C++ Redistributable from: https://aka.ms/vs/17/release/vc_redist.x64.exe
2. Restart your terminal after installation
3. Try running probe again

For advanced users or enterprise environments, you can set the environment variable `PROBE_AUTO_INSTALL_MSVC=true` to attempt automatic download of the required redistributable during installation.

## Troubleshooting Windows-Specific Issues

If you encounter errors when running probe on Windows:

### Missing VCRUNTIME140.dll or similar files
- Install Microsoft Visual C++ Redistributable 2015-2022 (x64) from: https://aka.ms/vs/17/release/vc_redist.x64.exe

### Access Denied or Permission Issues
- Try running your terminal as Administrator
- Check Windows Defender or antivirus software, which might be blocking the executable

### Other Windows-Specific Errors
- Make sure your Windows is up to date
- If running in a restricted environment, check with your system administrator about execution policies

### GCC Compiler Issues (For Developers)

When building from source, if you encounter the following error:
```
error occurred in cc-rs: failed to find tool "gcc.exe": program not found (see https://docs.rs/cc/latest/cc/#compile-time-requirements for help)
```

You need to install a C/C++ compiler toolchain:

1. Install MSYS2 using Windows Package Manager:
   ```
   winget install MSYS2.MSYS2
   ```

2. Open the "MSYS2 MSYS2" shortcut from the Start Menu

3. Install the MinGW-w64 toolchain:
   ```
   pacman -S --needed base-devel mingw-w64-ucrt-x86_64-toolchain
   ```

4. Add to PATH environment variable:
   - Add `C:\msys64\ucrt64\bin` to your system PATH
   - Restart any open command prompts or terminals

For more details, see [cc-rs documentation](https://docs.rs/cc/latest/cc/#compile-time-requirements) or [VS Code MinGW setup guide](https://code.visualstudio.com/docs/cpp/config-mingw#_installing-the-mingww64-toolchain).

## Technical Details

### Build Configuration

The probe binary for Windows is now built with the following optimizations:
- Standard optimization level (opt-level = 2) for better compatibility
- Thin LTO for faster linking and better compatibility
- More codegen units for better compatibility with Windows toolchain
- Static linking of C runtime to reduce dependency issues

### Installation Process

During installation, the npm package now:
1. Checks if the required Microsoft Visual C++ Redistributable is installed
2. Warns the user if it's missing
3. Optionally attempts to download the redistributable (if PROBE_AUTO_INSTALL_MSVC=true)
4. Provides clear error messages if runtime issues are encountered