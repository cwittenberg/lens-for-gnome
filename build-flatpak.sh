#!/usr/bin/env bash
# build-flatpak.sh - Production Flatpak Deployment Compilation Engine
set -euo pipefail

APP_ID="org.gnome.Lens"
BUILD_DIR="build-dir"
REPO_DIR="repo"

echo "====================================================================="
echo "   GNOME LENS COMPLIANT FLATPAK DISTRIBUTION ENGINE               "
echo "====================================================================="

# Validate essential toolchain presence (Host cargo is NOT required for Flatpak builds)
for tool in flatpak flatpak-builder; do
    if ! command -v "$tool" &> /dev/null; then
        echo "CRITICAL ERROR: Required tool '$tool' is not installed." >&2
        exit 1
    fi
done

# Force-inject missing Flatpak runtime paths into XDG_DATA_DIRS dynamically
# This eliminates the snap/flatpak path routing warning caused by running inside the VSCode Snap container
SNAP_CODE_DIR=$(ls -d "${HOME}"/snap/code/*/ 2>/dev/null | sort -rn | head -n 1 || echo "${HOME}/snap/code/current/")
export XDG_DATA_DIRS="${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:${HOME}/.local/share/flatpak/exports/share:/var/lib/flatpak/exports/share:${SNAP_CODE_DIR}.local/share/flatpak/exports/share"

# Ensure the Flathub remote repository is explicitly added to the USER scope
echo "[Step 1/5] Configuring Flathub remote repository..."
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo

# Explicitly download the active SDK and Platform into the user scope from flathub
echo "[Step 2/5] Ensuring GNOME Platform 50 Core Runtimes are present..."
flatpak install --user -y flathub org.gnome.Sdk//50 org.gnome.Platform//50

# Clean up stale build directory (We preserve the downloaded .flatpak-builder source caches)
echo "[Step 3/5] Purging previous build directory..."
rm -rf "$BUILD_DIR"

# Compile application using sandboxed dependency paths, auto-resolve SDK extensions, and bypass FUSE limits
echo "[Step 4/5] Launching containerized compilation loop via flatpak-builder..."
flatpak-builder \
    --user \
    --install \
    --force-clean \
    --ccache \
    --install-deps-from=flathub \
    --disable-rofiles-fuse \
    --repo="$REPO_DIR" \
    "$BUILD_DIR" \
    "${APP_ID}.json"

echo "[Step 5/5] Finalizing localized verification map..."
echo "---------------------------------------------------------------------"
echo " SUCCESS: ${APP_ID} has been successfully deployed and registered."
echo " To run the sandboxed daemon background loop manually, execute:"
echo "     flatpak run ${APP_ID}"
echo "====================================================================="