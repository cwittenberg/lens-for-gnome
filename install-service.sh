#!/bin/bash
set -euo pipefail

echo "Deploying Release version via Flatpak Engine..."
./build-flatpak.sh

echo "Setting up secure host directories..."
mkdir -p ~/.local/state/gnome-lens
mkdir -p ~/.config/gnome-lens

if [ -f "models.json" ]; then
    echo "Deploying models.json configuration..."
    cp models.json ~/.config/gnome-lens/models.json
fi

echo "Copying Flatpak-adapted service file to systemd user directory..."
mkdir -p ~/.config/systemd/user/
cp lens.service ~/.config/systemd/user/gnome-lens.service

echo "Reloading systemd daemon..."
systemctl --user daemon-reload

echo "Enabling service to start on boot..."
systemctl --user enable gnome-lens.service

echo "Restarting service..."
systemctl --user restart gnome-lens.service

echo "Install Complete! Gnome Lens daemon is running in the background as a sandboxed Flatpak."
echo "You can view logs at any time using: tail -f ~/.local/state/gnome-lens/daemon.log"