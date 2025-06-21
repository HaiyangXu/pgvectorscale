# Development Environment Setup Guide

## PostgreSQL Extension Development on Windows

This guide provides multiple approaches to set up a working development environment for pgvectorscale bitmap filtering.

## Current Issue

The build fails due to missing PostgreSQL development headers:
```
C:/PROGRA~1/POSTGR~1/17/include/server\c.h:75:10: fatal error: 'libintl.h' file not found
```

## Solution Options

### Option 1: Windows Native Setup (Recommended for Windows Development)

#### Install Dependencies
```powershell
# Install Git for Windows (if not already installed)
winget install Git.Git

# Install PostgreSQL development libraries
# Download and install PostgreSQL 17 with development files
# From: https://www.postgresql.org/download/windows/

# Install Microsoft C++ Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# Install gettext for libintl.h
# Download from: https://gnuwin32.sourceforge.net/packages/gettext.htm
# Or use vcpkg:
git clone https://github.com/Microsoft/vcpkg.git
cd vcpkg
.\bootstrap-vcpkg.bat
.\vcpkg integrate install
.\vcpkg install gettext:x64-windows
```

#### Set Environment Variables
```powershell
# Add to system PATH
$env:PATH += ";C:\vcpkg\installed\x64-windows\bin"
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# Set PostgreSQL paths
$env:PGRX_PG_CONFIG_PATH = "C:\Program Files\PostgreSQL\17\bin\pg_config.exe"
```

#### Test the Setup
```powershell
cd C:\src\pgvectorscale\pgvectorscale
cargo pgrx init --pg17="C:\Program Files\PostgreSQL\17\bin\pg_config.exe"
cargo test bitmap_filtering --features pg17
```

### Option 2: WSL2 Linux Environment (Recommended for Cross-Platform)

#### Setup WSL2
```powershell
# Enable WSL2
wsl --install

# Install Ubuntu
wsl --install -d Ubuntu-22.04
```

#### Install Dependencies in WSL2
```bash
# Update package list
sudo apt update

# Install PostgreSQL development packages
sudo apt install -y postgresql-server-dev-17 postgresql-17

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

# Install build dependencies
sudo apt install -y clang libclang-dev build-essential pkg-config

# Install pgrx
cargo install cargo-pgrx --version 0.12.9
```

#### Setup and Test
```bash
# Navigate to project (assuming WSL can access Windows files)
cd /mnt/c/src/pgvectorscale/pgvectorscale

# Initialize pgrx
cargo pgrx init --pg17=pg_config

# Run tests
cargo test bitmap_filtering --features pg17
```

### Option 3: Docker Development Environment

#### Create Dockerfile
```dockerfile
FROM rust:1.75

# Install PostgreSQL development packages
RUN apt-get update && apt-get install -y \
    postgresql-server-dev-17 \
    postgresql-17 \
    clang \
    libclang-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install pgrx
RUN cargo install cargo-pgrx --version 0.12.9

# Set working directory
WORKDIR /workspace

# Copy source code
COPY . .

# Initialize pgrx
RUN cd pgvectorscale && cargo pgrx init --pg17=pg_config

CMD ["bash"]
```

#### Build and Run
```powershell
# Build the development container
docker build -t pgvectorscale-dev .

# Run with source mounted
docker run -it -v "C:\src\pgvectorscale:/workspace" pgvectorscale-dev

# Inside container
cd pgvectorscale
cargo test bitmap_filtering --features pg17
```

## Validation Steps

Once the environment is set up, validate the implementation:

### 1. Syntax Validation
```bash
cargo check --features pg17
```

### 2. Unit Tests
```bash
cargo test bitmap_filtering --features pg17
```

### 3. Integration Tests
```bash
cargo pgrx test --features pg17
```

### 4. Installation Test
```bash
cargo pgrx install --features pg17
```

### 5. SQL Integration Test
```sql
-- Connect to test database
\c test_db

-- Load extension
CREATE EXTENSION vectorscale;

-- Run bitmap filtering demo
\i examples/bitmap_filtering_demo.sql
```

## Performance Benchmarking

After successful setup, benchmark the implementation:

```bash
# Run performance tests
cargo test --release --features pg17 performance

# Run benchmarks
cargo bench --features pg17
```

## Troubleshooting

### Common Issues

1. **libintl.h not found**
   - Install gettext development libraries
   - Ensure proper include paths

2. **pg_config not found**
   - Install PostgreSQL development packages
   - Add pg_config to PATH

3. **Clang/LLVM issues**
   - Verify LIBCLANG_PATH environment variable
   - Install compatible LLVM version

4. **Permission issues on Windows**
   - Run PowerShell as Administrator
   - Check Windows Defender exclusions

### Environment Variables Checklist
```powershell
# Verify these are set correctly
echo $env:LIBCLANG_PATH
echo $env:PGRX_PG_CONFIG_PATH
pg_config --version
clang --version
```

## Next Steps After Setup

1. **Validate Implementation**: Run all tests
2. **Performance Analysis**: Benchmark against baseline
3. **SQL Integration**: Test real-world queries
4. **Documentation**: Update based on test results
5. **Production Deployment**: Package for distribution

The bitmap filtering implementation is complete and ready for testing once the development environment is properly configured.
