#!/bin/bash
# run-manager.sh

# Exit immediately if a command exits with a non-zero status
set -e

echo "=========================================================="
echo " Lens for GNOME - Manager UI Build & Run Script"
echo "=========================================================="

echo "-> Building Lens for GNOME Manager (Optimized Release Mode)..."
cargo build --release --bin lens-for-gnome-manager

echo "-> Launching Lens for GNOME Manager..."
exec ./target/release/lens-for-gnome-manager