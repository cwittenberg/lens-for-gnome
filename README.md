# Lens for GNOME

**Lens for GNOME** is an AI powered local-first desktop search engine.

It serves as a fast launcher and smart search solution. It indexes local apps and files, extracts searchable text, watches folders for changes, runs local embeddings, supports OCR, syncs mail, and can use local GGUF language models through llama.cpp (such as those from HuggingFace).

The goal is simple: fast, private desktop search that helps you find information inside your files _without_ sending your documents to a remote service. Lens for GNOME is 100% privacy-first and local-first.

[![Get it from the Snap Store](https://snapcraft.io/en/dark/install.svg)](https://snapcraft.io/lens-for-gnome)

The GNOME extension used to interface with this backend-service is published on the GNOME store here: https://extensions.gnome.org/extension/10337/lens/

## What Lens Does

Lens turns content into a powerful searchable index on your computer.

It can:

- Index configured folders recursively.
- Watch folders for changes using inotify.
- Store searchable content in SQLite.
- Search exact text using SQLite FTS5.
- Search by meaning using local embeddings.
- Merge keyword and semantic results with Reciprocal Rank Fusion.
- Extract text from documents, images, PDFs, office documents, mail, and videos.
- Run local LLM-powered queries when enabled.
- Keep indexed data, embeddings, metadata, and models on your machine.
- Fast preview videos or images on hover, multi-monitor capable.

## Screenshots

Filtering based on your data using AI

<img width="500" alt="Screencast From 2026-07-10 22-31-15 00:00:06" src="https://github.com/user-attachments/assets/755f71a4-4fd8-4fb9-bb4e-d06ba675044e" />

Questions about your data, powered by local AI - even if it's from i.e. screenshots/PDFs (OCR capable)

<img width="500" alt="Screencast From 2026-07-10 22-44-50 00:00:35" src="https://github.com/user-attachments/assets/6e746b11-3fac-41a1-b4d7-17acacd5c9f6" />

Filebrowsing and fast video preview. 

<img width="500" alt="Screencast From 2026-07-10 22-44-50 00:01:05" src="https://github.com/user-attachments/assets/4efd1947-1b5f-46de-89da-ace8cb7caa19" />

Support for different AI models, which can be turned off

<img width="500" alt="Screencast From 2026-07-10 22-44-50 00:02:16" src="https://github.com/user-attachments/assets/7a0e5dc1-3cb3-4565-9d3a-c3a25d07fa5d" />

Different themes available:

<img width="500" alt="Screencast From 2026-07-10 22-44-50 00:04:01" src="https://github.com/user-attachments/assets/30824bfc-2dc6-47d4-96ec-7594591bd2ef" />


## Why Lens Exists

Most desktop search tools are good at finding file names.

Lens is built for finding information inside your files.

You can search for exact words, ask more natural questions, or find related content even when the document does not use the same wording as your query.

Example searches:

```text
meeting notes about renewable energy pricing
```

```text
documents mentioning my lease termination date
```

```text
emails with an IBAN from last year
```

```text
invoices less than 300 usd
```


```text
source files related to the indexing pipeline
```

## Features

### Hybrid Search

Lens combines traditional full-text search with semantic search.

Keyword search is fast and precise. Semantic search helps when you know what you are looking for, but not the exact words used in the document.

Lens uses:

| Capability | Technology |
| --- | --- |
| Exact search | SQLite FTS5 |
| Semantic search | Local MiniLM embeddings through fastembed |
| Result merging | Reciprocal Rank Fusion |
| Local storage | SQLite |

### Local LLM Support

Lens can run local GGUF models through llama.cpp.

When enabled, local models can help with:

- Answering questions from indexed content.
- Summarizing matching documents.
- Routing natural-language queries.
- Handling more complex search requests.
- Turning vague searches into more useful retrieval steps.

The model manager can download supported GGUF models and stores them locally.

Hardware acceleration is detected when available, depending on the build and platform support. Supported backends may include:

- CUDA
- ROCm
- Apple Silicon
- Vulkan
- Other llama.cpp-supported backends

### File Ingestion

Lens extracts text and metadata from common local content types, including:

- Plain text files
- Source code
- PDFs
- Images
- Office documents
- Spreadsheets
- Presentations
- Videos
- Local mail files

PDFs with embedded text are read directly. Scanned pages can fall back to OCR.

Videos are scanned for metadata and embedded subtitles through ffmpeg.

Images can be processed with OCR and QR-code extraction.

### Mail Indexing

Lens can sync IMAP mail into local `.eml` files and index them like regular documents.

This allows local mail search without relying on a remote mail provider search API.

Mail indexing is useful when you want the same search experience across files, documents, and email.

### Smart Extraction

During indexing, Lens can detect structured values and store them with the indexed content.

Supported extracted values include:

- URLs
- Email addresses
- IP addresses
- MAC addresses
- IBANs
- Dates

This makes it easier to search for specific technical, financial, or contact-related information.

### Fast-Path Plugins

Some queries do not need the full search or LLM pipeline.

Lens includes fast paths for simple actions, including:

- Calculator queries
- Desktop application launching
- Filesystem browsing

These are handled directly to keep common actions instant.

## Privacy Model

Lens is designed to be local-first.

By default:

- Document content is indexed locally.
- Embeddings are created locally.
- Search data is stored locally.
- Models are stored locally.
- No indexed document content is sent to an external AI service.

Network access is only needed for features that explicitly require it, such as:

- Downloading local models.
- Syncing configured IMAP mail.
- Accessing user-configured remote resources.

## Installation and Setup

### System Dependencies

Install the required system tools first:

```bash
sudo apt install tesseract-ocr ffmpeg poppler-utils curl
```

| Dependency | Used For |
| --- | --- |
| `tesseract-ocr` | OCR for images and scanned PDFs |
| `ffmpeg` / `ffprobe` | Video metadata, subtitles, and thumbnails |
| `poppler-utils` | PDF rasterization for OCR through `pdftoppm` |
| `curl` | Model downloads |

### Increase the Inotify Watch Limit

Large folders may hit the default Linux inotify watch limit.

Increase it with:

```bash
echo 'fs.inotify.max_user_watches=524288' | sudo tee -a /etc/sysctl.conf
sudo sysctl -p
```

## Building from Source

Install Rust and Cargo first.

Then build Lens:

```bash
git clone https://github.com/cwittenberg/gnome-lens.git
cd gnome-lens
cargo build --release
```

The release binary is created here:

```text
target/release/gnome-lens
```

## Running Lens

Start the daemon manually:

```bash
./target/release/gnome-lens
```

The GNOME extension can also manage the daemon automatically.

## Command-Line Usage

### Index a Folder

```bash
./target/release/gnome-lens index /path/to/folder
```

### Force a Full Reindex

```bash
./target/release/gnome-lens reindex /path/to/folder
```

### Query the Running Daemon

```bash
./target/release/gnome-lens "what are the terms of my apartment lease?"
```

## Architecture

Lens runs as a local daemon and exposes search functionality to the GNOME desktop integration.

At a high level, the system contains:

| Component | Purpose |
| --- | --- |
| File watcher | Tracks filesystem changes using inotify |
| Ingestion pipeline | Extracts text, metadata, and structured values |
| Search index | Stores searchable text in SQLite FTS5 |
| Embedding pipeline | Creates local semantic vectors |
| Ranking layer | Merges exact and semantic results |
| Model manager | Handles local GGUF model downloads and paths |
| LLM runtime | Runs local llama.cpp queries when enabled |
| IPC layer | Serves requests from the desktop integration |

## Runtime Paths

Lens stores its local state under standard user directories.

| Component | Default Path |
| --- | --- |
| IPC socket | `~/.local/state/gnome-lens/gnome_lens.sock` |
| Configuration | `~/.config/gnome-lens/` |
| Database, models, and local data | `~/.local/share/gnome-lens/` |

When running inside Flatpak, Lens uses the matching Flatpak and XDG paths instead.

## Storage

Lens stores the following locally:

- Search index
- Extracted text
- Extracted metadata
- Embeddings
- Model files
- Local mail cache, when IMAP sync is configured

No document content is sent to an external service by default.

## Project Status

Lens for GNOME is under active development.

The project is focused on:

- Fast local indexing.
- Reliable folder watching.
- Strong GNOME desktop integration.
- Useful local AI-assisted search.
- Private-by-default behavior.
- Practical performance on normal desktop hardware.

## License

This project is licensed under the GNU General Public License v3.0.

See the `LICENSE` file for details.
