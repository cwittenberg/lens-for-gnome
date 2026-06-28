#!/bin/bash
# run.sh

echo "=========================================================="
echo " Lens for GNOME - Universal Hardware Auto-Detect Build Script"
echo "=========================================================="

# Auto-detect missing Vulkan and GTK4 build dependencies
NEEDS_DEPS=0
if ! command -v glslangValidator &> /dev/null; then NEEDS_DEPS=1; fi
if ! command -v cmake &> /dev/null; then NEEDS_DEPS=1; fi
if ! pkg-config --exists gtk4 &> /dev/null; then NEEDS_DEPS=1; fi

if [ "$NEEDS_DEPS" -eq 1 ] || [[ "$1" == "--setup" ]]; then
    echo "-> CRITICAL: Missing SDK build tools (Vulkan / GTK4 / SPIR-V). Auto-installing..."
    if command -v apt-get &> /dev/null; then
        sudo apt-get update
        sudo apt-get install -y spirv-headers spirv-tools glslang-tools glslang-dev libvulkan-dev cmake build-essential tesseract-ocr ffmpeg poppler-utils libgtk-4-dev pkg-config
    elif command -v dnf &> /dev/null; then
        sudo dnf install -y spirv-headers spirv-tools glslang glslang-devel vulkan-loader-devel cmake gcc-c++ tesseract ffmpeg poppler-utils gtk4-devel pkgconf-pkg-config
    elif command -v pacman &> /dev/null; then
        sudo pacman -Syu --needed --noconfirm spirv-headers spirv-tools glslang vulkan-headers vulkan-icd-loader cmake base-devel tesseract tesseract-data-eng ffmpeg poppler gtk4 pkgconf
    elif command -v zypper &> /dev/null; then
        sudo zypper install -y spirv-headers spirv-tools glslang glslang-devel vulkan-devel cmake gcc-c++ tesseract-ocr ffmpeg poppler-tools gtk4-devel pkgconf
    else
        echo "Error: Unsupported package manager. Please manually install Vulkan and GTK4 dev tools."
        exit 1
    fi
    echo "-> Dependencies installed. Wiping CMake cache to register new headers..."
    rm -rf target/release/build/llama-cpp* target/debug/build/llama-cpp*
    cargo clean
fi

# Ensure state directory exists early so we can track build targets
STATE_DIR="$HOME/.local/state/lens-for-gnome"
mkdir -p "$STATE_DIR"
STATE_FILE="$STATE_DIR/last_hw_target"

# Clear previous hardware flags and enforce CMake generator to prevent Ninja/Make deadlocks
unset LLAMA_CUDA
unset LLAMA_VULKAN
unset LLAMA_METAL
unset LLAMA_HIPBLAS
unset CMAKE_ARGS
unset GGML_VULKAN
export CMAKE_GENERATOR="Unix Makefiles"

BACKEND_NAME="CPU_V2"
CARGO_FEATURES=""

# 1. Detect macOS / Apple Silicon (Metal)
if [ "$(uname)" == "Darwin" ]; then
    if [ "$(uname -m)" == "arm64" ]; then
        export LLAMA_METAL=1
        BACKEND_NAME="METAL_V2"
        CARGO_FEATURES="--features llama-cpp-2/metal"
    fi
# 2. Detect Linux Hardware
elif [ "$(uname)" == "Linux" ]; then
    if command -v nvidia-smi &> /dev/null && nvidia-smi -L &> /dev/null; then
        export LLAMA_CUDA=1
        BACKEND_NAME="CUDA_V2"
        CARGO_FEATURES="--features llama-cpp-2/cuda"
    elif command -v lspci &> /dev/null; then
        if lspci | grep -iE 'vga|display|3d|npu' | grep -i 'amd\|radeon' &> /dev/null; then
            export LLAMA_VULKAN=1
            export GGML_VULKAN=1
            export CMAKE_ARGS="-DGGML_VULKAN=1"
            BACKEND_NAME="VULKAN_AMD_V2"
            CARGO_FEATURES="--features llama-cpp-2/vulkan"
        elif lspci | grep -iE 'vga|display|3d|npu' | grep -i 'intel' &> /dev/null; then
            export LLAMA_VULKAN=1
            export GGML_VULKAN=1
            export CMAKE_ARGS="-DGGML_VULKAN=1"
            BACKEND_NAME="VULKAN_INTEL_V2"
            CARGO_FEATURES="--features llama-cpp-2/vulkan"
        fi
    fi
fi

echo "-> Hardware Detected: $BACKEND_NAME"

LAST_TARGET="NONE"
if [ -f "$STATE_FILE" ]; then
    LAST_TARGET=$(cat "$STATE_FILE")
fi

if [ "$BACKEND_NAME" != "$LAST_TARGET" ] || [ "$1" == "--clean" ]; then
    echo "-> Target changed or clean requested. Nuking C++ build artifacts..."
    rm -rf target/release/build/llama-cpp* target/debug/build/llama-cpp*
    cargo clean
    echo "$BACKEND_NAME" > "$STATE_FILE"
    if [ "$1" == "--clean" ]; then shift; fi
else
    echo "-> Hardware target unchanged ($BACKEND_NAME). Skipping cache purge."
fi

echo "-> Building Lens for GNOME (Optimized Release Mode)..."
cargo build --release $CARGO_FEATURES

echo "-> Stopping any existing instances..."
killall lens-for-gnome 2>/dev/null || true
pkill -f lens-for-gnome 2>/dev/null || true

echo "-> Starting daemon in the background..."
export DEBUG_VISION_OCR=1
nohup ./target/release/lens-for-gnome > "$STATE_DIR/daemon.log" 2>&1 &

echo "-> Tailing logs (Press Ctrl+C to exit logs, daemon will keep running)..."
sleep 0.5
tail -f "$STATE_DIR/daemon.log"