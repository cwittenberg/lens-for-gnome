#!/bin/bash
# publish-snap.sh
set -euo pipefail

echo "=========================================================="
echo " Lens for GNOME - Strict Snap Publisher"
echo "=========================================================="

# 1. Ensure Snapcraft is installed
if ! command -v snapcraft &> /dev/null; then
    echo "Installing Snapcraft..."
    sudo snap install snapcraft --classic
fi

# 2. Ensure LXD is installed
if ! snap list lxd &> /dev/null; then
    echo "Installing LXD container manager..."
    sudo snap install lxd
fi

# 3. Ensure LXD is initialized
echo "Initializing LXD..."
sudo lxd init --auto

# 4. Enforce LXD group permissions
if ! id -nG "$USER" | grep -qw "lxd"; then
    echo "Adding $USER to the 'lxd' group (requires sudo)..."
    sudo usermod -aG lxd "$USER"
    
    echo "Restarting LXD service to forcefully apply socket ACLs..."
    sudo snap restart lxd
    sleep 2
fi

# 5. Clean up legacy snap/gui
if [ -d "snap/gui" ]; then
    echo "Cleaning up legacy snap/gui directory..."
    rm -rf snap/gui
fi

echo "Generating strictly confined snapcraft.yaml..."
cat << 'EOF' > snapcraft.yaml
name: lens-for-gnome
title: 'Lens for GNOME'
base: core24
version: '0.4.5'
summary: AI-assisted local desktop search engine
description: |
  Powerful local-first semantic search service. Use AI locally to semantically search through your system and email.

contact: 'https://github.com/cwittenberg/lens-for-gnome'
website: 'https://github.com/cwittenberg/lens-for-gnome'
issues: 'https://github.com/cwittenberg/lens-for-gnome/issues'
source-code: 'https://github.com/cwittenberg/lens-for-gnome'
license: 'MIT'

grade: stable
confinement: strict

layout:
  /usr/share/tesseract-ocr:
    bind: $SNAP/usr/share/tesseract-ocr

apps:
  daemon:
    command: bin/start-daemon.sh
    daemon: simple
    daemon-scope: user
    restart-condition: on-failure
    extensions: [gnome]
    plugs:
      - home
      - network
      - network-bind
      - opengl
      - removable-media
      - gsettings
      - hardware-observe
      - desktop
      - desktop-legacy
  manager:
    command: bin/lens-for-gnome-manager
    extensions: [gnome]
    desktop: usr/share/applications/lens-for-gnome.desktop
    plugs:
      - home
      - network
      - network-bind
      - opengl
      - removable-media
      - hardware-observe
      - desktop
      - desktop-legacy

parts:
  lens-for-gnome:
    plugin: nil
    source: .
    build-snaps:
      - rustup
    build-packages:
      - pkg-config
      - libgtk-4-dev
      - cmake
      - g++
      - libvulkan-dev
      - glslang-tools
      - spirv-tools
      - spirv-headers
      - glslc
      - libssl-dev
      - clang
      - libclang-dev
    stage-packages:
      - tesseract-ocr
      - tesseract-ocr-eng
      - ffmpeg
      - poppler-utils
      - curl
      - libssl3
      - procps
      - dconf-cli
      - libnotify-bin
    prime:
      - "-usr/lib/*/libharfbuzz*"
      - "-usr/lib/*/librsvg*"
      - "-usr/lib/*/libgdk_pixbuf*"
      - "-usr/lib/*/libcairo*"
      - "-usr/lib/*/libpango*"
      - "-usr/lib/*/libfreetype*"
      - "-usr/lib/*/libfontconfig*"
      - "-usr/lib/*/libcaca++*"
      - "-usr/lib/*/libcjson_utils*"
      - "-usr/lib/*/libfreebl*"
      - "-usr/lib/*/libhwy*"
      - "-usr/lib/*/libnss*"
      - "-usr/lib/*/libsoftokn*"
      - "-usr/lib/*/libsphinxad*"
      - "-usr/lib/*/libtheora*"
      - "-usr/lib/*/libzvbi*"
      - "-usr/lib/*/libfftw3_omp*"
      - "-usr/lib/*/libfftw3_threads*"
      - "-usr/lib/*/libflite_cmu_grapheme_lang*"
      - "-usr/lib/*/libflite_cmu_grapheme_lex*"
      - "-usr/lib/*/libflite_cmu_indic_lang*"
      - "-usr/lib/*/libflite_cmu_indic_lex*"
      - "-usr/lib/*/libflite_cmu_time_awb*"
      - "-usr/lib/*/libjacknet*"
      - "-usr/lib/*/libjackserver*"
      - "-usr/lib/*/libpulse-simple*"
      - "-usr/lib/*/libssl3.so"
    override-build: |
      ORIGINAL_LD_LIBRARY_PATH="${LD_LIBRARY_PATH:-}"
      unset LD_LIBRARY_PATH
      
      rustup default stable
      rustup toolchain install stable --component rustc,cargo
      
      cd $CRAFT_PART_SRC
      
      rm -rf target/release/build/llama-cpp* target/debug/build/llama-cpp*
      
      export CMAKE_GENERATOR="Unix Makefiles"
      export LLAMA_VULKAN=1
      export GGML_VULKAN=1
      export CMAKE_ARGS="-DGGML_VULKAN=1"
      
      cargo build --release --features llama-cpp-2/vulkan
      
      export LD_LIBRARY_PATH="$ORIGINAL_LD_LIBRARY_PATH"
      
      mkdir -p $CRAFT_PART_INSTALL/bin
      cp target/release/lens-for-gnome $CRAFT_PART_INSTALL/bin/
      cp target/release/lens-for-gnome-manager $CRAFT_PART_INSTALL/bin/
      
      mkdir -p $CRAFT_PART_INSTALL/usr/share/glib-2.0/schemas
      cp -r gnome-extension/schemas/* $CRAFT_PART_INSTALL/usr/share/glib-2.0/schemas/
      glib-compile-schemas $CRAFT_PART_INSTALL/usr/share/glib-2.0/schemas/
      
      mkdir -p $CRAFT_PART_INSTALL/usr/share/applications
      mkdir -p $CRAFT_PART_INSTALL/usr/share/pixmaps
      
      cp metadata/io.github.cwittenberg.Lens.icon.svg $CRAFT_PART_INSTALL/usr/share/pixmaps/lens-for-gnome.svg
      
      echo "[Desktop Entry]" > $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Version=1.0" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Type=Application" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Name=Lens for GNOME" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Exec=lens-for-gnome.manager" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Icon=\${SNAP}/usr/share/pixmaps/lens-for-gnome.svg" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Terminal=false" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "StartupNotify=true" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      echo "Categories=Utility;" >> $CRAFT_PART_INSTALL/usr/share/applications/lens-for-gnome.desktop
      
      echo '#!/bin/bash' > $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo 'mkdir -p "$HOME/.local/state/lens-for-gnome"' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      
      echo 'EXT_ENABLED=$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || dconf read /org/gnome/shell/enabled-extensions 2>/dev/null || echo "")' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo 'if [[ "$EXT_ENABLED" != *"lens-for-gnome@cwittenberg"* ]]; then' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo "    gdbus call --session --dest org.freedesktop.Notifications --object-path /org/freedesktop/Notifications --method org.freedesktop.Notifications.Notify -- \"'Lens for GNOME'\" \"uint32 0\" \"'\$SNAP/usr/share/pixmaps/lens-for-gnome.svg'\" \"'Lens for GNOME'\" \"'The GNOME Shell extension is missing or disabled. Please enable it in your browser.'\" \"@as []\" \"@a{sv} {}\" \"int32 -1\" || true" >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo '    gio open "https://extensions.gnome.org/?search=lens-for-gnome" || true' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo 'fi' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      
      echo 'export GSETTINGS_SCHEMA_DIR="$SNAP/usr/share/glib-2.0/schemas"' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      echo 'exec "$SNAP/bin/lens-for-gnome" 2>&1 | tee "$HOME/.local/state/lens-for-gnome/daemon.log"' >> $CRAFT_PART_INSTALL/bin/start-daemon.sh
      chmod +x $CRAFT_PART_INSTALL/bin/start-daemon.sh
EOF

echo "Building Snap package..."

if sg lxd -c "lxc list" &> /dev/null; then
    sg lxd -c "snapcraft pack"
else
    echo "CRITICAL ERROR: Failed to acquire LXD permissions dynamically."
    echo "The system requires a hard session reset to apply the 'lxd' group."
    echo "Please log out of your computer completely, log back in, and run this script again."
    exit 1
fi

echo "=========================================================="
echo "Build complete."
echo ""
echo "To test locally:"
echo "Run: sudo snap install --dangerous lens-for-gnome_0.4.5_amd64.snap"
echo "=========================================================="