# Lens for GNOME Backend

**Lens for GNOME** indexes local files, extracts searchable text, runs local embeddings, handles OCR, syncs IMAP mail, and serves search results.

The goal is simple: fast desktop search with local-first AI-assisted processing.

## What it does

* Indexes files from configured folders
* Watches folders for changes using inotify
* Stores searchable content in SQLite
* Supports exact text search through SQLite FTS5
* Supports semantic search using local embeddings
* Combines search results with Reciprocal Rank Fusion
* Extracts text from documents, images, PDFs, mail, and videos
* Runs local LLM queries through llama.cpp when enabled
* Keeps data and models on the local machine

## Features

### Hybrid search

Lens combines regular full-text search with semantic vector search.

* Exact keyword search: SQLite FTS5
* Semantic search: local MiniLM embeddings through fastembed
* Result merging: Reciprocal Rank Fusion

This keeps normal searches fast while still allowing more natural queries.

### Local LLM support

The daemon can run local GGUF models through llama.cpp.

It can be used for:
* Summarizing matching documents
* Answering questions from indexed content
* Routing natural-language queries
* Handling more complex searches

The model manager can download supported GGUF models and stores them locally.

Hardware acceleration is detected when available, including:
* CUDA
* ROCm
* Apple Silicon
* Vulkan / other supported llama.cpp backends, depending on the build

### File ingestion

The daemon extracts text and metadata from:

* Plain text files
* Source code
* PDFs
* Images
* Office documents
* Spreadsheets
* Presentations
* Videos
* Local mail files

PDFs are read directly when embedded text is available. Scanned pages can fall back to OCR.

Images are processed with OCR and QR-code extraction.

Videos are scanned for metadata and embedded subtitles through ffmpeg.

### Mail indexing

The daemon can sync IMAP mail into local `.eml` files and index them like regular documents.

This is used for local mail search without relying on a remote search API.

### Smart extraction

During indexing, Lens can detect useful structured values such as:

* URLs
* Email addresses
* IP addresses
* MAC addresses
* IBANs
* Dates

These values are stored with the indexed content so they can be searched directly.

### Fast-path plugins

Some queries skip the LLM and search pipeline completely.

Current fast paths include:

* Calculator queries
* Desktop application launching

This keeps simple actions instant.

## Dependencies

Install the required system tools first:

```bash
sudo apt install tesseract-ocr ffmpeg poppler-utils curl
```

Used by:

* `tesseract-ocr`: OCR for images and scanned PDFs
* `ffmpeg` / `ffprobe`: video metadata, subtitles, and thumbnails
* `poppler-utils`: PDF rasterization for OCR through `pdftoppm`
* `curl`: model downloads

## Inotify watch limit

Large folders may hit the default Linux inotify watch limit.

Increase it with:

```bash
echo 'fs.inotify.max_user_watches=524288' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

## Building from source

Install Rust and Cargo first.

Then build the daemon:

```bash
git clone https://github.com/cwittenberg/gnome-lens.git
cd gnome-lens
cargo build --release
```

The release binary is created at:

```bash
target/release/gnome-lens
```

## Running

Start the daemon manually:

```bash
./target/release/gnome-lens
```

The GNOME extension can also manage the daemon automatically.

## Manual indexing

Index a folder recursively:

```bash
./target/release/gnome-lens index /path/to/folder
```

Force a full reindex:

```bash
./target/release/gnome-lens reindex /path/to/folder
```

## Querying from the command line

Send a query to the running daemon:

```bash
./target/release/gnome-lens "what are the terms of my apartment lease?"
```

## Runtime paths

The daemon uses the following local paths by default.

### IPC socket

```bash
~/.local/state/gnome-lens/gnome_lens.sock
```

### Configuration

```bash
~/.config/gnome-lens/
```

### Database and models

```bash
~/.local/share/gnome-lens/
```

When running inside Flatpak, the daemon uses the matching Flatpak/XDG paths instead.

## Storage

Lens stores its index, embeddings, model files, and extracted metadata locally.

No document content is sent to an external service by default.

Network access is only needed for features that explicitly require it, such as downloading models or syncing configured IMAP mail.

## License

MIT License
