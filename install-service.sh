#!/bin/bash

echo "Building Release version..."
cargo build --release

echo "Setting up secure log directory..."
mkdir -p ~/.local/state/gnome-lens

echo "Copying existing service file to systemd user directory..."
mkdir -p ~/.config/systemd/user/
cp lens.service ~/.config/systemd/user/gnome-lens.service

echo "Reloading systemd daemon..."
systemctl --user daemon-reload

echo "Enabling service to start on boot..."
systemctl --user enable gnome-lens

echo "Restarting service..."
systemctl --user restart gnome-lens

echo "Install Complete! Gnome Lens daemon is running in the background."
echo "You can view logs at any time using: tail -f ~/.local/state/gnome-lens/daemon.log"