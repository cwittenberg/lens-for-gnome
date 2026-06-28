# Gnome Lens Backend (gnome-lens-daemon)

This is the Rust-based backend daemon for the Gnome Lens shell extension. It handles file indexing, local vector embeddings, OCR, IMAP synchronization, and local LLM execution to provide a privacy-first, AI-powered spotlight search for the GNOME desktop.

All data stays on your machine. Models run entirely locally using llama.cpp and fastembed.

## Features

* **Hybrid Search Engine**: Combines traditional exact-match text search (SQLite FTS5) with semantic vector search (Paraphrase MiniLM) using Reciprocal Rank Fusion (RRF) for fast retrieval
* **Local LLM Integration**: Uses llama.cpp to run local models (e.g., Qwen 2.5) It automatically detects hardware acceleration (CUDA, ROCm, Apple Silicon, NPUs) and manages GGUF download to let the user run the AI model locally.
* **Agentic Query Routing**: The LLM parses natural language queries and can:
* Synthesize direct answers from your documents (RAG).
* Compile on-the-fly scripts to filter results (e.g., "Find receipts from last month where the total is over $500").


* **Deep File Ingestion**: Watches your configured directories using inotify and extracts text from:
* Plain text & code
* PDFs (uses embedded text, falls back to OCR via Tesseract)
* Images (OCR and QR code extraction)
* Office documents (DOCX, PPTX, XLSX, XLS)
* Videos (extracts metadata and embedded subtitles via ffmpeg)
* Local Mail (IMAP syncs Gmail directly to local .eml files)


* **Smart Data Extraction**: Automatically detects and extracts IPs, MAC addresses, URLs, emails, IBANs, and dates from file contents.
* **Fast-Path Plugins**: Bypasses the AI pipeline for instantaneous results when calculating math equations or launching desktop applications.

## Dependencies

The daemon relies on a few external system tools for file parsing and OCR. Make sure these are installed on your system:

sudo apt install tesseract-ocr ffmpeg poppler-utils curl

* tesseract-ocr: For image and scanned PDF text extraction.
* ffmpeg / ffprobe: For video metadata and thumbnail extraction.
* poppler-utils: Specifically pdftoppm for rasterizing PDFs before OCR.
* curl: Used by the model manager to download GGUF weights.

If you are hitting the OS limit for inotify watches during indexing, you may need to increase it:

echo 'fs.inotify.max_user_watches=524288' | sudo tee -a /etc/sysctl.conf && sudo sysctl -p

## Building from Source

Ensure you have Rust and Cargo installed, then build in release mode for optimal LLM and embedding performance.

git clone [https://github.com/cwittenberg/gnome-lens](https://www.google.com/search?q=https://github.com/cwittenberg/gnome-lens)
cd gnome-lens-daemon
cargo build --release

The binary will be located at target/release/gnome-lens.

## Running the Daemon

The daemon is designed to be run at system start but can also be managed by the GNOME extension, you can run it manually for debugging or manual indexing.

Start the background service:

./target/release/gnome-lens

Trigger a manual recursive index on a specific directory:

./target/release/gnome-lens index /path/to/your/folder

Force a complete database re-index (resets timestamps):

./target/release/gnome-lens reindex /path/to/your/folder

Send a test query via CLI (requires the daemon to be running):

./target/release/gnome-lens "what are the terms of my apartment lease?"

## Architecture Notes

* **IPC**: The daemon listens on a Unix domain socket located at ~/.local/state/gnome-lens/gnome_lens.sock (or the equivalent Flatpak XDG path).
* **Storage**:
* Configs: ~/.config/gnome-lens/
* SQLite DB & Models: ~/.local/share/gnome-lens/


* **Sandbox Support**: Automatically detects if running inside a Flatpak container and routes XDG paths and xdg-open commands accordingly.

## License

MIT License