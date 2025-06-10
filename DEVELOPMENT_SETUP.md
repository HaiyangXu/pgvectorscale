# PostgreSQL Development Environment Setup for Windows

This guide provides step-by-step instructions for setting up a complete PostgreSQL development environment on Windows for pgvectorscale development.

## Prerequisites

- Windows 10/11 with PowerShell
- Administrator access for installing software
- At least 4GB free disk space

## Step 1: Install Required Software

### 1.1 Install PostgreSQL with Development Headers

Download and install PostgreSQL from the official source:

1. Go to https://www.postgresql.org/download/windows/
2. Download the PostgreSQL installer (version 15 or later recommended)
3. **Important**: During installation, check "PostgreSQL Server" and "pgAdmin 4" AND "Stack Builder"
4. Note the installation directory (typically `C:\Program Files\PostgreSQL\17\`)

### 1.2 Install Visual Studio Build Tools

PostgreSQL extensions require Microsoft Visual C++ compiler:

1. Download "Build Tools for Visual Studio 2022" from Microsoft
2. Install with these workloads:
   - C++ build tools
   - Windows 10/11 SDK (latest version)
   - CMake tools for Visual Studio

### 1.3 Install LLVM/Clang (Already Done)

You already have LLVM installed at `C:\Program Files\LLVM\`. Verify version:

```powershell
clang --version
```

### 1.4 Install Git and Rust (Already Done)

Your Rust environment is already set up.

## Step 2: Install Missing Dependencies

### 2.1 Install gettext (for libintl.h)

The missing `libintl.h` file is part of gettext. Install it:

**Option A: Using vcpkg (Recommended)**

```powershell
# Install vcpkg if not already installed
git clone https://github.com/Microsoft/vcpkg.git C:\vcpkg
cd C:\vcpkg
.\bootstrap-vcpkg.bat

# Install gettext
.\vcpkg.exe install gettext:x64-windows
```

**Option B: Manual Installation**

1. Download gettext for Windows from: https://mlocati.github.io/articles/gettext-iconv-windows.html
2. Extract to `C:\gettext`
3. The `libintl.h` file should be in `C:\gettext\include\`

### 2.2 Install Additional PostgreSQL Development Files

Some distributions may not include all headers. Download the complete PostgreSQL source:

```powershell
# Create development directory
mkdir C:\postgres-dev
cd C:\postgres-dev

# Download PostgreSQL source (replace with your version)
Invoke-WebRequest -Uri "https://ftp.postgresql.org/pub/source/v17.0/postgresql-17.0.tar.gz" -OutFile "postgresql-17.0.tar.gz"

# Extract (you may need 7-Zip or similar)
tar -xzf postgresql-17.0.tar.gz
```

## Step 3: Configure Environment Variables

### 3.1 Set PostgreSQL Environment Variables

Add these to your system environment variables:

```powershell
# Add to PATH
$env:PATH += ";C:\Program Files\PostgreSQL\17\bin"
$env:PATH += ";C:\Program Files\PostgreSQL\17\lib"

# PostgreSQL specific
$env:PGROOT = "C:\Program Files\PostgreSQL\17"
$env:PGDATA = "C:\Program Files\PostgreSQL\17\data"
$env:PGUSER = "postgres"

# Development headers
$env:PGSQL_INCLUDE = "C:\Program Files\PostgreSQL\17\include"
$env:PGSQL_INCLUDE_SERVER = "C:\Program Files\PostgreSQL\17\include\server"
$env:PGSQL_LIB = "C:\Program Files\PostgreSQL\17\lib"
```

### 3.2 Set Compiler Environment Variables

```powershell
# LLVM/Clang
$env:LLVM_CONFIG_PATH = "C:\Program Files\LLVM\bin\llvm-config.exe"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# gettext (if using vcpkg)
$env:INCLUDE += ";C:\vcpkg\installed\x64-windows\include"
$env:LIB += ";C:\vcpkg\installed\x64-windows\lib"

# gettext (if manual installation)
# $env:INCLUDE += ";C:\gettext\include"
# $env:LIB += ";C:\gettext\lib"

# Additional PostgreSQL includes
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server\port\win32"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server\port\win32_msvc"
```

### 3.3 Set PGRX Configuration

Configure PGRX to use your PostgreSQL installation:

```powershell
# Initialize PGRX with your PostgreSQL installation
cargo install cargo-pgrx --version 0.12.9
cargo pgrx init --pg17 "C:\Program Files\PostgreSQL\17\bin\pg_config.exe"
```

## Step 4: Verify Installation

### 4.1 Check PostgreSQL Installation

```powershell
# Verify PostgreSQL is running
pg_config --version
pg_config --includedir-server

# Test connection
psql -U postgres -c "SELECT version();"
```

### 4.2 Check Development Headers

```powershell
# Verify critical headers exist
Test-Path "C:\Program Files\PostgreSQL\17\include\server\postgres.h"
Test-Path "C:\Program Files\PostgreSQL\17\include\server\fmgr.h"

# Check for libintl.h (this was missing)
Test-Path "C:\vcpkg\installed\x64-windows\include\libintl.h"
# OR
Test-Path "C:\gettext\include\libintl.h"
```

### 4.3 Test Compilation

```powershell
cd C:\src\pgvectorscale

# Try a simple build first
cargo check

# If successful, try building with PostgreSQL integration
cargo build --features pg_test
```

## Step 5: Troubleshooting Common Issues

### Issue 1: "libintl.h not found"

**Solution**: Ensure gettext is properly installed and environment variables are set.

```powershell
# Check if libintl.h is in the include path
where.exe libintl.h

# If not found, verify gettext installation
dir "C:\vcpkg\installed\x64-windows\include\libintl.h"
```

### Issue 2: "Cannot find postgres.h"

**Solution**: PostgreSQL server development headers are missing.

```powershell
# Reinstall PostgreSQL with development components
# OR manually copy headers from source distribution
```

### Issue 3: "Link errors with PostgreSQL libraries"

**Solution**: Check library paths and ensure PostgreSQL libraries are accessible.

```powershell
# Verify library files exist
dir "C:\Program Files\PostgreSQL\17\lib\*.lib"

# Check if lib directory is in PATH
echo $env:LIB
```

### Issue 4: "PGRX configuration errors"

**Solution**: Reinitialize PGRX configuration.

```powershell
# Clean PGRX configuration
Remove-Item -Recurse -Force "$env:USERPROFILE\.pgrx" -ErrorAction SilentlyContinue

# Reinitialize
cargo pgrx init --pg17 "C:\Program Files\PostgreSQL\17\bin\pg_config.exe"
```

## Step 6: Permanent Environment Setup

### 6.1 Create PowerShell Profile Script

Create a PowerShell profile to automatically set environment variables:

```powershell
# Check if profile exists
Test-Path $PROFILE

# Create profile directory if needed
New-Item -ItemType Directory -Path (Split-Path $PROFILE) -Force

# Edit profile
notepad $PROFILE
```

Add this content to your PowerShell profile:

```powershell
# PostgreSQL Development Environment
$env:PATH += ";C:\Program Files\PostgreSQL\17\bin"
$env:PATH += ";C:\Program Files\PostgreSQL\17\lib"
$env:PATH += ";C:\Program Files\LLVM\bin"

# PostgreSQL
$env:PGROOT = "C:\Program Files\PostgreSQL\17"
$env:PGSQL_INCLUDE = "C:\Program Files\PostgreSQL\17\include"
$env:PGSQL_INCLUDE_SERVER = "C:\Program Files\PostgreSQL\17\include\server"
$env:PGSQL_LIB = "C:\Program Files\PostgreSQL\17\lib"

# Compiler
$env:LLVM_CONFIG_PATH = "C:\Program Files\LLVM\bin\llvm-config.exe"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# gettext (adjust path as needed)
$env:INCLUDE += ";C:\vcpkg\installed\x64-windows\include"
$env:LIB += ";C:\vcpkg\installed\x64-windows\lib"

# PostgreSQL includes
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server\port\win32"
$env:INCLUDE += ";C:\Program Files\PostgreSQL\17\include\server\port\win32_msvc"

Write-Host "PostgreSQL development environment loaded" -ForegroundColor Green
```

### 6.2 System Environment Variables (Alternative)

Alternatively, set these as permanent system environment variables through Windows System Properties.

## Step 7: Testing the Setup

### 7.1 Build pgvectorscale

```powershell
cd C:\src\pgvectorscale

# Clean previous build artifacts
cargo clean

# Build the extension
cargo build --release

# Install the extension
cargo pgrx install --release
```

### 7.2 Run Tests

```powershell
# Run unit tests
cargo test

# Run PostgreSQL integration tests
cargo test --features pg_test

# Run specific bitmap filtering tests
cargo test --features pg_test test_bitmap_filtering_basic
```

## Step 8: IDE Configuration

### 8.1 Visual Studio Code

If using VS Code, add these settings to `.vscode/settings.json`:

```json
{
    "rust-analyzer.cargo.features": ["pg_test"],
    "rust-analyzer.cargo.extraEnv": {
        "PGSQL_INCLUDE": "C:\\Program Files\\PostgreSQL\\17\\include",
        "PGSQL_INCLUDE_SERVER": "C:\\Program Files\\PostgreSQL\\17\\include\\server",
        "LIBCLANG_PATH": "C:\\Program Files\\LLVM\\bin"
    }
}
```

## Next Steps

After completing this setup:

1. **Verify the build works**: `cargo build --features pg_test`
2. **Run the test suite**: `cargo test --features pg_test`
3. **Test the bitmap filtering functionality**: Run the specific tests in `bitmap_filtering_tests.rs`
4. **Performance testing**: Use the performance comparison tests to validate optimization benefits

## Common Commands Reference

```powershell
# Check PostgreSQL status
Get-Service postgresql*

# Start PostgreSQL service
Start-Service postgresql-x64-17

# Connect to PostgreSQL
psql -U postgres

# Build and install extension
cargo pgrx install --release

# Run all tests
cargo test --features pg_test -- --nocapture

# Run specific test
cargo test --features pg_test test_bitmap_filtering_basic -- --nocapture
```

This setup should resolve the `libintl.h` issue and provide a complete development environment for pgvectorscale on Windows.