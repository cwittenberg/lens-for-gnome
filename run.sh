#!/bin/bash
# run.sh
echo "Building Gnome Lens (Optimized Release Mode)..."
cargo build --release

echo "Stopping any existing instances..."
killall gnome-lens
pkill -f gnome-lens

echo "Setting up secure directories..."
mkdir -p ~/.local/state/gnome-lens

echo "Starting daemon in the background..."
DEBUG_VISION_OCR=1 nohup ./target/release/gnome-lens > ~/.local/state/gnome-lens/daemon.log 2>&1 &

echo "Tailing logs (Press Ctrl+C to exit logs, daemon will keep running)..."
sleep 0.5
tail -f ~/.local/state/gnome-lens/daemon.log