#!/bin/bash
echo "Building Gnome Lens..."
cargo build

echo "Stopping any existing instances..."
pkill -f gnome-lens

echo "Setting up secure directories..."
mkdir -p ~/.local/state/gnome-lens

echo "Starting daemon in the background..."
nohup ./target/debug/gnome-lens > ~/.local/state/gnome-lens/daemon.log 2>&1 &

echo "Tailing logs (Press Ctrl+C to exit logs, daemon will keep running)..."
sleep 0.5
tail -f ~/.local/state/gnome-lens/daemon.log