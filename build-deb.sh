#!/usr/bin/env bash
# build-deb.sh
# Meticulous software engine release script for Debian/Ubuntu upstream tracking.
set -euo pipefail

# 1. Project Definitions
PACKAGE_NAME="lens-for-gnome"
VERSION="1.0.0"
ARCHITECTURE="amd64"
MAINTAINER="Your Name <your.email@example.com>"
DESCRIPTION="Intelligent Semantic File Search Engine and Local Data Assistant for GNOME"

BUILD_ROOT="/tmp/${PACKAGE_NAME}-build-deb-root"
DEB_DIR="${BUILD_ROOT}/DEBIAN"

echo "[Stage 1/6] Cleaning up workspace and creating directory matrices..."
rm -rf "${BUILD_ROOT}"
mkdir -p "${BUILD_ROOT}"
mkdir -p "${DEB_DIR}"

# 2. Build Release Target safely using Rust Cargo toolchain
echo "[Stage 2/6] Compiling optimized release binary via Cargo toolchain..."
if ! command -v cargo &> /dev/null; then
    echo "Error: Cargo toolchain is required to execute this deployment pipeline." >&2
    exit 1
fi

cargo build --release

# 3. Provisioning File System Layout Boundaries
echo "[Stage 3/6] Populating architecture file layout structures..."
# Create binary paths, systemd user configurations, and app indicators
mkdir -p "${BUILD_ROOT}/usr/bin"
mkdir -p "${BUILD_ROOT}/usr/lib/systemd/user"
mkdir -p "${BUILD_ROOT}/usr/share/applications"

# Copy verified compilation artifacts
cp target/release/lens-for-gnome "${BUILD_ROOT}/usr/bin/lens-for-gnome"
chmod 755 "${BUILD_ROOT}/usr/bin/lens-for-gnome"

# Copy the GUI Manager App artifact
cp target/release/lens-for-gnome-manager "${BUILD_ROOT}/usr/bin/lens-for-gnome-manager"
chmod 755 "${BUILD_ROOT}/usr/bin/lens-for-gnome-manager"

# Generate GUI Application Desktop Entry (AppCenter Compliance)
cat << 'EOF' > "${BUILD_ROOT}/usr/share/applications/lens-for-gnome-manager.desktop"
[Desktop Entry]
Name=Lens for GNOME
Comment=Service Manager and Log Viewer for Lens for GNOME
Exec=/usr/bin/lens-for-gnome-manager
Icon=system-search
Terminal=false
Type=Application
Categories=System;Utility;
EOF

chmod 644 "${BUILD_ROOT}/usr/share/applications/lens-for-gnome-manager.desktop"


# 4. Generate Integrated Systemd Service Block Configuration
echo "[Stage 4/6] Embedding Systemd User Service unit description..."
cat << 'EOF' > "${BUILD_ROOT}/usr/lib/systemd/user/lens-for-gnome.service"
[Unit]
Description=Lens for GNOME Daemon Engine
Documentation=https://github.com/cwittenberg/lens-for-gnome
After=graphical-session.target

[Service]
Type=simple
ExecStart=/usr/bin/lens-for-gnome
Restart=on-failure
RestartSec=5s
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF

chmod 644 "${BUILD_ROOT}/usr/lib/systemd/user/lens-for-gnome.service"

# 5. Provisioning Debian Core Tracking Control Blocks
echo "[Stage 5/6] Writing Debian package manifest tracking files..."

# Generate standard metadata control block file
cat << EOF > "${DEB_DIR}/control"
Package: ${PACKAGE_NAME}
Version: ${VERSION}
Architecture: ${ARCHITECTURE}
Maintainer: ${MAINTAINER}
Depends: libc6, tesseract-ocr, tesseract-ocr-eng, ffmpeg, poppler-utils, libgtk-4-1
Description: ${DESCRIPTION}
EOF

chmod 644 "${DEB_DIR}/control"

# Post-installation integration controller script
cat << 'EOF' > "${DEB_DIR}/postinst"
#!/bin/sh
set -e

if [ "$1" = "configure" ]; then
    echo "========================================================================="
    echo " Lens for GNOME daemon successfully integrated into system paths."
    echo " Enabling and starting the service globally for active users..."
    echo "========================================================================="
    
    # Enable globally for all users on next login
    systemctl --global enable lens-for-gnome.service || true
    
    # Start for currently logged in users
    for uid_dir in /run/user/*; do
        if [ -d "$uid_dir" ]; then
            uid=$(basename "$uid_dir")
            user=$(id -un "$uid" 2>/dev/null || true)
            if [ -n "$user" ]; then
                sudo -u "$user" XDG_RUNTIME_DIR="$uid_dir" systemctl --user daemon-reload || true
                sudo -u "$user" XDG_RUNTIME_DIR="$uid_dir" systemctl --user start lens-for-gnome.service || true
            fi
        fi
    done
fi
exit 0
EOF

chmod 755 "${DEB_DIR}/postinst"

# Pre-removal automated lifecycle controller script
cat << 'EOF' > "${DEB_DIR}/prerm"
#!/bin/sh
set -e

# Disable the service before removing binaries to prevent terminal hung socket boundaries
if [ "$1" = "remove" ] || [ "$1" = "deconfigure" ]; then
    if command -v systemctl >/dev/null 2>&1; then
        echo "Deactivating daemon components for active graphical user spaces..."
        
        # Disable globally
        systemctl --global disable lens-for-gnome.service || true
        
        # Stop for currently logged in users
        for uid_dir in /run/user/*; do
            if [ -d "$uid_dir" ]; then
                uid=$(basename "$uid_dir")
                user=$(id -un "$uid" 2>/dev/null || true)
                if [ -n "$user" ]; then
                    sudo -u "$user" XDG_RUNTIME_DIR="$uid_dir" systemctl --user stop lens-for-gnome.service || true
                fi
            fi
        done
    fi
fi
exit 0
EOF

chmod 755 "${DEB_DIR}/prerm"

# 6. Compiling the final Binary Artifact archive (.deb)
echo "[Stage 6/6] Packaging target release context via dpkg-deb tool chain..."
mkdir -p dist
OUTPUT_DEB="dist/${PACKAGE_NAME}_${VERSION}_${ARCHITECTURE}.deb"
dpkg-deb --build "${BUILD_ROOT}" "${OUTPUT_DEB}"

# Clean up build root metrics safely
rm -rf "${BUILD_ROOT}"

echo "------------------------------------------------------------------------"
echo " Build successful! Output target file compiled: ${OUTPUT_DEB}"
echo " Install locally using: sudo apt install ./${OUTPUT_DEB}"
echo "------------------------------------------------------------------------"