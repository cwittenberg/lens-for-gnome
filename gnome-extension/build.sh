#!/usr/bin/env bash
set -euo pipefail

UUID="gnome-lens@cwittenberg"
BUILD_DIR="build"
EXTENSION_DIR="$HOME/.local/share/gnome-shell/extensions/$UUID"
PROJECT_DIR="$(pwd)"
PACKAGE_PATH="$PROJECT_DIR/$BUILD_DIR/$UUID.shell-extension.zip"
SHEXLI_VENV="$PROJECT_DIR/.shexli-venv"

echo "Cleaning up previous builds..."
rm -rf "$BUILD_DIR"
rm -f "$UUID.zip"
rm -f "$UUID.shell-extension.zip"

echo "Creating build directory structure..."
mkdir -p "$BUILD_DIR/schemas"
mkdir -p "$BUILD_DIR/locale"
mkdir -p "po"

echo "Validating extension files..."
for file in metadata.json extension.js ui.js ui_search.js ui_results.js ui_status.js indicator.js daemon.js prefs.js prefs_main.js prefs_ai.js prefs_about.js schemas/org.gnome.shell.extensions.gnome-lens.gschema.xml; do
    if [ ! -f "$file" ]; then
        echo "Error: $file not found in the current directory. Please make sure all files exist."
        exit 1
    fi
done

echo "Compiling GSettings schema locally..."
glib-compile-schemas --strict schemas/

echo "Extracting strings and generating translation template..."
if command -v xgettext &> /dev/null; then
    xgettext --from-code=UTF-8 --language=JavaScript --keyword=_ --add-comments -o po/gnome-lens.pot extension.js ui.js ui_search.js ui_results.js ui_status.js indicator.js daemon.js prefs.js prefs_main.js prefs_ai.js prefs_about.js
    echo "Translation template generated at po/gnome-lens.pot"
else
    echo "Warning: xgettext not found, skipping string extraction."
fi

echo "Merging and compiling translations..."
for po_file in po/*.po; do
    if [ -f "$po_file" ]; then
        if command -v msgmerge &> /dev/null && [ -f "po/gnome-lens.pot" ]; then
            msgmerge --update --quiet "$po_file" po/gnome-lens.pot
        fi

        if ! command -v msgfmt &> /dev/null; then
            echo "Error: msgfmt not found. Install gettext to compile translations."
            exit 1
        fi

        lang=$(basename "$po_file" .po)
        mkdir -p "$BUILD_DIR/locale/$lang/LC_MESSAGES"
        msgfmt "$po_file" -o "$BUILD_DIR/locale/$lang/LC_MESSAGES/gnome-lens.mo"
        echo "Compiled locale: $lang"
    fi
done

echo "Copying files to build directory..."
cp metadata.json extension.js ui.js ui_search.js ui_results.js ui_status.js indicator.js daemon.js prefs.js prefs_main.js prefs_ai.js prefs_about.js "$BUILD_DIR/"
cp -r schemas "$BUILD_DIR/"

rm -f "$BUILD_DIR/schemas/gschemas.compiled"

if [ -f stylesheet.css ]; then
    cp stylesheet.css "$BUILD_DIR/"
fi

if [ -f trayicon.svg ]; then
    cp trayicon.svg "$BUILD_DIR/"
fi

if [ -f LICENSE ]; then
    cp LICENSE "$BUILD_DIR/"
fi

if [ -f README.md ]; then
    cp README.md "$BUILD_DIR/"
fi

echo "Packaging extension..."
if command -v gnome-extensions &> /dev/null; then
    PACK_ARGS=(
        "--extra-source=extension.js"
        "--extra-source=ui.js"
        "--extra-source=ui_search.js"
        "--extra-source=ui_results.js"
        "--extra-source=ui_status.js"
        "--extra-source=indicator.js"
        "--extra-source=daemon.js"
        "--extra-source=prefs.js"
        "--extra-source=prefs_main.js"
        "--extra-source=prefs_ai.js"
        "--extra-source=prefs_about.js"
        "--extra-source=schemas"
    )

    if find "$BUILD_DIR/locale" -type f -name '*.mo' | grep -q .; then
        PACK_ARGS+=("--extra-source=locale")
    fi

    if [ -f "$BUILD_DIR/trayicon.svg" ]; then
        PACK_ARGS+=("--extra-source=trayicon.svg")
    fi

    if [ -f "$BUILD_DIR/stylesheet.css" ]; then
        PACK_ARGS+=("--extra-source=stylesheet.css")
    fi

    if [ -f "$BUILD_DIR/LICENSE" ]; then
        PACK_ARGS+=("--extra-source=LICENSE")
    fi

    if [ -f "$BUILD_DIR/README.md" ]; then
        PACK_ARGS+=("--extra-source=README.md")
    fi

    gnome-extensions pack "$BUILD_DIR" "${PACK_ARGS[@]}" --force

    mv "$UUID.shell-extension.zip" "$PACKAGE_PATH"
else
    echo "gnome-extensions CLI not found, falling back to zip..."

    if ! command -v zip &> /dev/null; then
        echo "Error: zip not found. Install zip or gnome-shell-extension-prefs / gnome-shell-common."
        exit 1
    fi

    (cd "$BUILD_DIR" && zip -r "../$UUID.shell-extension.zip" .)
    mv "$UUID.shell-extension.zip" "$PACKAGE_PATH"
fi

echo "Running Shexli EGO checks..."
if command -v shexli &> /dev/null; then
    shexli "$PROJECT_DIR/$BUILD_DIR" --format text
    shexli "$PACKAGE_PATH" --format text
    shexli "$PACKAGE_PATH" --format json > "$PROJECT_DIR/shexli-report.json"
    echo "Shexli JSON report written to: $PROJECT_DIR/shexli-report.json"
else
    echo "Shexli not found in PATH."

    if command -v python3.12 &> /dev/null; then
        echo "Creating local Shexli virtual environment..."
        python3.12 -m venv "$SHEXLI_VENV"
        source "$SHEXLI_VENV/bin/activate"

        python -m pip install --upgrade pip
        python -m pip install "git+https://github.com/GNOME/extensions-web.git#subdirectory=shexli"

        shexli "$PROJECT_DIR/$BUILD_DIR" --format text
        shexli "$PACKAGE_PATH" --format text
        shexli "$PACKAGE_PATH" --format json > "$PROJECT_DIR/shexli-report.json"
        echo "Shexli JSON report written to: $PROJECT_DIR/shexli-report.json"
    else
        echo "Warning: python3.12 not found, skipping Shexli checks."
        echo "Install Python 3.12 or activate your existing shexli-venv before running this script."
    fi
fi

echo "Installing extension locally..."
rm -rf "$EXTENSION_DIR"
mkdir -p "$EXTENSION_DIR"

cp "$BUILD_DIR/metadata.json" "$EXTENSION_DIR/"
cp "$BUILD_DIR/extension.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/ui.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/ui_search.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/ui_results.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/ui_status.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/indicator.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/daemon.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/prefs.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/prefs_main.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/prefs_ai.js" "$EXTENSION_DIR/"
cp "$BUILD_DIR/prefs_about.js" "$EXTENSION_DIR/"
cp -r "$BUILD_DIR/schemas" "$EXTENSION_DIR/"

if [ -f "$BUILD_DIR/stylesheet.css" ]; then
    cp "$BUILD_DIR/stylesheet.css" "$EXTENSION_DIR/"
fi

if [ -f "$BUILD_DIR/trayicon.svg" ]; then
    cp "$BUILD_DIR/trayicon.svg" "$EXTENSION_DIR/"
fi

if [ -f "$BUILD_DIR/LICENSE" ]; then
    cp "$BUILD_DIR/LICENSE" "$EXTENSION_DIR/"
fi

if [ -f "$BUILD_DIR/README.md" ]; then
    cp "$BUILD_DIR/README.md" "$EXTENSION_DIR/"
fi

if find "$BUILD_DIR/locale" -type f -name '*.mo' | grep -q .; then
    cp -r "$BUILD_DIR/locale" "$EXTENSION_DIR/"
fi

echo "Compiling schemas for local installation..."
glib-compile-schemas "$EXTENSION_DIR/schemas/"

echo "Upload package created at: $PACKAGE_PATH"

echo "Attempting to enable extension in GNOME Shell..."
# Wait briefly to allow GNOME Shell's directory monitor to detect the new files
sleep 2

if command -v gnome-extensions &> /dev/null; then
    if gnome-extensions enable "$UUID" 2>/dev/null; then
        echo "Extension enabled successfully via CLI."
    else
        echo "GNOME Shell has not registered the extension in memory yet."
        echo "Falling back to gsettings DBus injection..."
        
        # Fetch current extensions array from dconf
        CURRENT_EXTENSIONS=$(gsettings get org.gnome.shell enabled-extensions)
        
        # Check if UUID is already in the array
        if [[ "$CURRENT_EXTENSIONS" != *"$UUID"* ]]; then
            if [ "$CURRENT_EXTENSIONS" = "@as []" ]; then
                NEW_EXTENSIONS="['$UUID']"
            else
                # Strip the trailing bracket and append the UUID
                NEW_EXTENSIONS=$(echo "$CURRENT_EXTENSIONS" | sed "s/]$/, '$UUID']/")
            fi
            gsettings set org.gnome.shell enabled-extensions "$NEW_EXTENSIONS"
            echo "Successfully injected $UUID into gsettings enabled-extensions."
        else
            echo "$UUID is already present in gsettings enabled-extensions."
        fi

        echo "========================================================================"
        echo "WARNING: You are likely running a Wayland session."
        echo "GNOME Shell strictly prevents loading newly registered extensions without"
        echo "a compositor restart. You MUST log out and log back in for the extension"
        echo "to appear."
        echo "========================================================================"
    fi
else
    echo "gnome-extensions CLI not found. Please enable it manually."
fi

rm -rf "$SHEXLI_VENV"