# MNEMONIC 

##  Software Development Plan: Personal Note Vault, Semantic RAG, PDF Editor & AI Assistant (Rust + egui + Candle + Qwen 2.5)

Dokumen ini memuat perencanaan arsitektur, spesifikasi teknis, alur data, rancangan antarmuka pengguna, serta tahapan implementasi untuk aplikasi produktivitas berbasis **Rust**, **eframe/egui**, **FastEmbed**, **Hugging Face Candle (Qwen 2.5-Coder/Instruct)**, dan **PDF Manipulation Engine**.

---

## 1. Executive Summary & Visi Produk

### 1.1 Deskripsi Produk
Aplikasi ini adalah tool produktivitas desktop/mobile mandiri (*100% offline & private*) yang mengintegrasikan empat pilar utama:
1. **Personal Note-Taking Vault (Google Keep UX + Obsidian File Model)**: Mencatat ide secara cepat dalam tampilan kartu (*grid/masonry*) seperti Google Keep, namun setiap catatan disimpan sebagai file `.md` mandiri di folder *vault* milik pengguna sendiri — portable, bisa dibuka editor lain, tidak terkunci di database proprietary.
2. **Obsidian-Quality Markdown Rendering & Editing**: Live preview WYSIWYG yang cantik (heading, checklist interaktif, tabel, code block bersyntax-highlight, wikilink `[[catatan]]`, backlink, callout) — bukan sekadar textarea polos.
3. **Local Semantic Retrieval Engine (FastEmbed)**: Mengindeks dan mencari seluruh catatan `.md` dan `.pdf` berdasarkan makna konteks kalimat panjang via *Dense Vector Embeddings*.
4. **Local Generative AI Assistant (Hugging Face Candle + Qwen 2.5) & Integrated PDF Editor**: Menjawab pertanyaan pengguna berbasis dokumen (RAG lokal), serta membuka, menganotasi, dan mengedit PDF (*merge, split, rotate, text injection, metadata*) langsung dari hasil pencarian.

### 1.2 Nilai Utama (Key Value Propositions)
* **100% Local & Privacy-First:** Tidak ada data catatan, dokumen, atau prompt user yang keluar ke cloud/server pihak ketiga.
* **File-Based, Bukan Database-Locked:** Isi catatan selalu berupa file `.md` biasa di disk pengguna (folder *vault*, seperti Obsidian) — database (SQLite) hanya dipakai sebagai *index cache* untuk pencarian & metadata UI, bukan sumber kebenaran utama.
* **Cepat Menangkap Ide (Keep-style Quick Capture)** sekaligus **Terstruktur & Bertaut (Obsidian-style Linking)** — menggabungkan kecepatan Keep dengan kedalaman knowledge-base Obsidian dalam satu aplikasi.
* **Dual-AI Architecture:**
  * **Retriever:** Model embedding ringkas (~80 MB, `all-MiniLM-L6-v2`) untuk pencarian instan berkecepatan tinggi.
  * **Generator:** LLM native Rust murni via `candle` (`Qwen 2.5 1.5B/3B Quantized`) untuk penalaran mendalam dan sintesis jawaban bahasa alami (sangat fasih bahasa Indonesia & teknis).
* **High Performance & Single Binary:** Menggunakan ekosistem Rust murni tanpa dependensi runtime Python/C++ eksternal yang rumit.

---

## 2. Arsitektur Sistem & Tech Stack

```
+-----------------------------------------------------------------------------------+
|                                 egui / eframe UI                                  |
|  +--------------------+  +----------------------+  +---------------------------+  |
|  |  Notes Grid View   |  |  Markdown Live Editor |  |  AI Chat / Q&A Assistant  |  |
|  |  (Keep-style Cards)|  |  (Obsidian-style)     |  |  (Streaming Response)     |  |
|  +--------------------+  +----------------------+  +---------------------------+  |
|  |  Semantic Search   |  |   PDF Viewer          |  |   Sidebar: Vault/Tags/    |  |
|  |  & Ranking Panel   |  |   (Continuous Scroll) |  |   Backlinks/Trash         |  |
|  +--------------------+  +----------------------+  +---------------------------+  |
|  |                 Interactive PDF Editor & Canvas Annotator                   |  |
|  +-----------------------------------------------------------------------------+  |
+-----------------------------------------+-----------------------------------------+
                                          | Tokio / Crossbeam Channel (Async IPC)
+-----------------------------------------+-----------------------------------------+
|                              Backend Core Services                                |
|  +-------------------+  +--------------------+  +-------------------------------+ |
|  |  Vault & Note     |  |  Markdown Engine    |  |  File Ingestion               | |
|  |  Manager          |  |  - Parser (AST)     |  |  - PDF parser                 | |
|  |  - Frontmatter    |  |  - Syntax highlight |  |  - MD CommonMark              | |
|  |  - File watcher   |  |  - Wikilink resolver|  |                               | |
|  +-------------------+  +--------------------+  +-------------------------------+ |
|  +-------------------+  +--------------------+  +-------------------------------+ |
|  |   FastEmbed Engine |  | Candle LLM Engine (Qwen 2.5)                        | |
|  | - all-MiniLM-L6-v2 |  | - Qwen2.5-1.5B/3B (Q4/GGUF/SF)                       | |
|  | - Cosine Similarity|  | - Streaming Token Generator                          | |
|  +-------------------+  +--------------------------------------------------------+ |
|  |                     Storage & PDF Manipulation Services                      | |
|  | - SQLite Index Cache  - lopdf / pdf-extract    - Canvas Overlay Engine       | |
|  +------------------------------------------------------------------------------+ |
+-----------------------------------------------------------------------------------+
```

### 2.1 Pilihan Dependensi Crate (Rust)

| Kategori | Crate | Versi Rekomendasi | Fungsi Utama |
| :--- | :--- | :--- | :--- |
| **GUI Framework** | `eframe` / `egui` | `^0.27` | Immediate mode GUI framework, performa tinggi & hemat memori |
| **Styling & Widgets** | `egui_extras`, `egui_phosphor` | Latest | Ikon antarmuka, loading gambar, custom chat bubble |
| **Markdown Parser** | `pulldown-cmark` | `^0.10` | Parsing Markdown ke AST untuk rendering format teks & chunking |
| **Markdown Rendering (UI)** | `egui_commonmark` | `^0.16` | Rendering AST Markdown menjadi widget egui bergaya rapi (heading, tabel, list, blockquote) sebagai basis tampilan ala Obsidian |
| **Syntax Highlighting** | `syntect` | `^5.2` | Highlight kode pada fenced code block (tema mirip VSCode/Obsidian) |
| **Frontmatter Metadata** | `serde_yaml` | `^0.9` | Parsing & penulisan YAML frontmatter (pin, warna, tag, arsip, trash) pada tiap file `.md` |
| **File Watcher** | `notify` | `^6.1` | Deteksi perubahan file eksternal (real-time reload, mencegah konflik overwrite) |
| **Timestamp & ID** | `chrono`, `uuid` | `^0.4` / `^1.8` | Waktu dibuat/diubah, ID unik catatan |
| **Clipboard & Paste-Image** | `arboard` | `^3.4` | Quick capture: paste gambar langsung ke catatan (ala Keep) |
| **Image Handling** | `image` | `^0.25` | Decode/encode gambar untuk lampiran & thumbnail |
| **Local Retriever** | `fastembed` | `^3.9` | ONNX runtime murni Rust untuk dense vector embeddings |
| **Local Generator (LLM)** | `candle-core`, `candle-transformers`, `candle-nn` | `^0.8` | Inference framework native Rust untuk menjalankan **Qwen 2.5** |
| **PDF Manipulation** | `lopdf` + `pdf-extract` | `^0.32` | Parsing PDF, ekstraksi teks, manipulasi objek teks, split & merge |
| **PDF Rendering** | `pdfium-render` (opsional) | Latest | Render halaman PDF ke bitmap texture visual untuk kanvas egui |
| **Concurrency & Async** | `tokio`, `rayon` | `^1.36` | Multithreaded chunking batch, asynchronous token streaming, file watch async |
| **Local Storage** | `rusqlite` | `^0.31` | *Index cache* metadata catatan/dokumen, vector index, dan history percakapan (bukan sumber isi catatan) |
| **File Picker** | `rfd` | `^0.14` | Dialog native file & folder selector OS (pemilihan folder Vault) |
| **Internationalization (i18n)** | `fluent-bundle`, `fluent-syntax`, `unic-langid` | `^0.15` / `^0.11` / `^0.9` | Sistem penerjemahan UI multi-bahasa via Project Fluent (`fluent-rs`), mendukung pluralization/gender per-bahasa dan pergantian bahasa saat runtime |

---

## 3. Spesifikasi Fitur Detail

### 3.1 Modul 1: Personal Note Vault (Google Keep UX + Obsidian File Model)

#### 3.1.1 Vault & Penyimpanan Berbasis File
* Saat pertama kali dijalankan, pengguna memilih/membuat folder **Vault** di disk (persis konsep Obsidian) — folder ini bisa disinkronkan sendiri oleh pengguna via Syncthing/Cloud Drive/Git, di luar tanggung jawab aplikasi.
* Setiap catatan = satu file `.md` di dalam vault (atau sub-folder sebagai "Notebook"/kategori). Isi file adalah **YAML Frontmatter** (metadata non-konten) diikuti body Markdown murni:
  ```yaml
  ---
  id: 8f3a1c2e-91b4-4d2f-9a7e-1234567890ab
  title: Belanja Mingguan
  type: checklist        # note | checklist
  created: 2026-08-20T09:15:00+07:00
  modified: 2026-08-23T10:02:00+07:00
  pinned: true
  color: yellow
  tags: [rumah, belanja]
  archived: false
  trashed: false
  reminder: null
  ---
  - [ ] Beli beras 5kg
  - [x] Bayar listrik
  - [ ] Servis kompor
  ```
* Karena isi murni Markdown standar, file tetap terbuka & terbaca normal walau dibuka di Obsidian, VSCode, atau editor teks apapun (tidak *lock-in*).
* **SQLite** hanya menyimpan *index cache* (untuk pencarian cepat & render grid tanpa parse ulang semua file tiap buka app) — dibangun ulang otomatis dari file `.md` bila hilang/korup, sehingga tidak pernah menjadi single point of failure.
* **File Watcher** (`notify`) memantau perubahan eksternal pada vault; jika file diedit dari luar aplikasi, UI me-reload otomatis. Jika terjadi tabrakan (file berubah di luar *dan* di dalam app hampir bersamaan), tampilkan dialog resolusi konflik (muat ulang / timpa / gabungkan manual).

#### 3.1.2 Tampilan Grid Ala Google Keep
* Tampilan utama berupa **Masonry Grid** kartu catatan (kolom otomatis menyesuaikan lebar window, mirip Pinterest/Keep), bukan daftar linear ala file explorer.
* Setiap kartu menampilkan: judul, cuplikan isi yang **sudah dirender** (bukan raw markdown mentah), warna latar kartu, ikon pin, badge tag/label, progress checklist (mis. "3/5 selesai").
* Tipe catatan: **Text Note** dan **Checklist/Todo Note** (checkbox interaktif langsung di kartu, klik toggle tanpa membuka editor penuh).
* **Quick Capture bar** di bagian atas grid — input satu baris untuk langsung membuat catatan baru instan (klik untuk memperluas jadi editor penuh), termasuk paste gambar langsung dari clipboard.
* Toolbar hover per-kartu: Pin/Unpin, ganti Warna (palet 8–12 warna pastel ala Keep), tambah Label, Arsipkan, pindah ke Trash, atur Pengingat (opsional, untuk checklist/reminder).
* Drag-and-drop untuk mengurutkan ulang kartu secara manual (urutan custom disimpan di index cache, tidak memodifikasi isi file).
* Mode multi-select (checkbox massal) untuk aksi batch: arsipkan/hapus/label banyak catatan sekaligus.

#### 3.1.3 Organisasi & Navigasi
* Sidebar kiri: **Semua Catatan**, **Berlabel** (per tag), **Diarsipkan**, **Sampah**, dan daftar **Notebook** (sub-folder vault sebagai kategori).
* Label/Tag manager: buat, ganti nama, hapus, dan beri warna tag — tag juga bisa ditulis inline di isi catatan via `#tag` (lihat 3.2.2).
* Search bar gabungan: keyword search (judul & isi) dengan opsi toggle **Semantic Search** (memakai engine yang sama dengan Modul 3.3).
* Opsi pengurutan: Terakhir diubah, Tanggal dibuat, Judul, Urutan manual, Warna.

#### 3.1.4 Trash & Keamanan Data
* Soft-delete: `trashed: true` di frontmatter, file dipindah ke sub-folder `.trash/` di dalam vault, dihapus permanen otomatis setelah 30 hari (dapat dikonfigurasi).
* Auto-backup `.bak` sebelum operasi tulis besar (mis. edit massal, migrasi format) untuk mencegah kehilangan data akibat crash saat penulisan.

---

### 3.2 Modul 2: Markdown Editor & Renderer (Kualitas Setara Obsidian)

#### 3.2.1 Mode Editing
* **Satu mode catatan — Live** (default, tidak ada lagi mode Tulis/Baca terpisah): catatan selalu tampil ter-render. **Klik sebuah baris → hanya baris itu** berubah jadi Markdown mentah di tempatnya; baris lain tetap ter-render. Tabel, fenced code, ```` ```mermaid ````, blok `$$` dan callout disunting utuh sebagai satu blok (klik, atau tombol ✎ saat di-hover).
  * Keyboard ala Obsidian: **Enter** memecah baris (list & checklist berlanjut otomatis; Enter di butir kosong mengakhiri list), **Backspace** di awal baris / **Delete** di akhir baris menggabung dengan baris tetangga, **↑/↓** di tepi pindah ke blok tetangga, **Esc** atau klik di luar kembali membaca. Klik ruang kosong di bawah catatan = tulis baris baru.
  * Popup `/` dan `[[` tetap bekerja di baris yang sedang disunting; setiap perubahan hanya mengganti baris itu di file (`live_blocks::replace_lines`, line ending asli dipertahankan) lewat undo & autosave yang sama.
  * Implementasi: `markdown::live_blocks` (pemecahan blok, murni), `markdown::renderer` (render per blok, virtualisasi, tinggi blok diingat per isi), `app::editor::live` (penyuntingan).
* **Source Mode** (tersembunyi): seluruh body sebagai Markdown mentah, hanya lewat command palette / ⌘E — untuk power-user/debugging.
* Tab di top bar: **Catatan · Teks + Diagram · Kanvas**.

#### 3.2.2 Elemen Markdown yang Didukung
| Elemen | Perilaku Rendering |
| :--- | :--- |
| Heading H1–H6 | Skala tipografi jelas & konsisten |
| Bold, italic, strikethrough, `code` inline, `==highlight==` | Rendering inline langsung |
| List bertingkat & checklist `- [ ]` / `- [x]` | Checklist interaktif — klik langsung toggle & tersimpan ke file |
| Blockquote bertingkat | Garis vertikal + aksen warna |
| Fenced code block | Syntax highlighting via `syntect` + tombol copy + label bahasa |
| Tabel Markdown | Grid rapi, auto-width, alignment sesuai `:---:` |
| Horizontal rule | — |
| Gambar/lampiran `![[gambar.png]]` (Obsidian embed) & `![alt](path)` | Drag-drop atau paste-dari-clipboard otomatis disimpan ke `attachments/` di vault |
| Wikilink `[[Judul Catatan]]` | Autocomplete saat mengetik `[[`, klik untuk navigasi, otomatis membuat catatan baru bila judul belum ada |
| Tag inline `#tag` | Dirender sebagai chip berwarna, klik untuk filter grid |
| Callout `> [!note]`, `> [!warning]`, `> [!tip]` | Kotak beraksen warna & ikon ala Obsidian admonition |
| Matematika `$...$` / `$$...$$` | **Opsional/fase lanjutan** — rendering LaTeX ringan; ditandai sebagai risiko teknis (lihat §6) |

* **Backlinks Panel**: menampilkan daftar catatan lain yang mereferensikan catatan aktif via wikilink — dibangun dari graf tautan yang disimpan di index cache.
* **Outline/Table of Contents**: panel otomatis dari heading dokumen aktif, klik untuk scroll ke bagian terkait.

#### 3.2.3 Tema & Tipografi
* Font UI kustom (mis. Inter/SF Pro untuk teks, JetBrains Mono untuk code block) melalui *custom font embedding* di `egui`.
* Tema aplikasi **Light** & **Dark** bawaan; warna isi catatan diatur oleh **tema baca** (§3.2.5).
* Line-height & spacing paragraf diatur khusus untuk kenyamanan baca (desain *typography-first*, bukan tampilan monospace default egui).

#### 3.2.5 Tema Baca, Cetak & Ekspor
* **Tema baca** (`reading_theme`): satu set warna untuk isi catatan — latar, teks, bold, link, `#tag`, H1–H6, kode, quote, garis, tabel, highlight, checkbox, dan aksen callout — dengan varian `[light]`, `[dark]`, dan `[print]`. Dipakai di tampilan Live **dan** saat cetak/ekspor, sehingga kertas, PDF, dan layar berwarna sama.
* **Bawaan:** `mnemonic` (netral), `pelangi` (tiap level heading berbeda warna), `ocean`, `sunset`, `forest`, `print-classic` (hemat tinta, aksen navy). Dipilih lewat ikon palet di top bar atau command palette; per catatan lewat frontmatter `theme: <id>`.
* **Plugin tema:** file TOML deklaratif (tanpa kode, aman dibagikan) di `<config_dir>/mnemonic/themes/` atau `<vault>/.mnemonic/themes/`. Warna yang tidak diisi diwarisi dari `base` (default `mnemonic`); `[print]` default = `[light]`. File rusak dilaporkan & dilewati, tidak pernah membuat aplikasi gagal. Panduan: `docs/themes.md`.
* **Cetak (⌘P):** HTML bertema dibuka di browser dengan dialog cetak otomatis; CSS memakai `print-color-adjust: exact`, jadi latar & aksen ikut tercetak.
* **Ekspor PDF / HTML:** HTML mandiri (CSS dari tema, Mermaid sebagai SVG inline, gambar sebagai `data:` URI, callout/wikilink/tag/`==highlight==` ikut bergaya). PDF dibuat oleh browser Chromium (Chrome/Edge/Chromium/Brave, atau `MNEMONIC_BROWSER`) secara headless di thread latar; tanpa browser, pengguna diarahkan ke Cetak → Simpan sebagai PDF.
* Agen: `mnemonic-cli themes list`, `mnemonic-cli notes export`, tool MCP `list_themes` & `export_note` (lihat `docs/agent-interface.md`).
* Implementasi: `reading_theme` (model, loader, registry), `export` (`html`, `system`), `app::reading`.

#### 3.2.4 Produktivitas Editor
* Auto-save saat idle (debounce 500ms–1s) langsung ke file `.md` di disk.
* Undo/redo history per sesi editing.
* Slash command `/` untuk sisipan cepat (heading, tabel, checklist, code block, callout, wikilink).
* Word count & estimasi waktu baca di status bar.

---

### 3.3 Modul 3: Semantic Document Ingestion & Retrieval
1. **Recursive Multi-Format Ingestion:**
   * Memindai vault catatan (`.md`) dan direktori dokumen untuk berkas `.md`, `.markdown`, dan `.pdf`.
   * Ekstraksi teks PDF terstruktur per halaman beserta koordinat teks dasar.
2. **Chunking Engine:**
   * Sliding window: 300 token per chunk dengan 50 token overlap.
   * Metadata chunk: `{ doc_id, file_path, page_num, char_offset, text_content }`.
3. **Embedding Vectorization:**
   * Model default: `all-MiniLM-L6-v2` (vektor 384-D, float32).
   * Cosine similarity scoring untuk mengambil **Top-K Chunks** (default $K=3..5$) yang paling relevan dengan pertanyaan.

---

### 3.4 Modul 4: Local AI Assistant via Candle (Qwen 2.5)
1. **Model Selection & Optimization:**
   * **Model Utama:** `Qwen2.5-1.5B-Instruct` (atau `Qwen2.5-3B-Instruct` untuk perangkat dengan RAM $\ge$ 8 GB) dalam format quantized (GGUF / Safetensors Q4_K_M).
   * Menjalankan inferensi via CPU murni (AVX2/NEON) atau akselerasi Metal / CUDA jika tersedia di hardware.
2. **RAG Pipeline Workflow:**
   * User memasukkan pertanyaan bebas / instruksi di kolom chat.
   * Retriever mengambil Top-K chunk dokumen (termasuk catatan pribadi) yang relevan.
   * Template prompt RAG disusun secara terstruktur:
     ```text
     <|im_start|>system
     Anda adalah asisten cerdas. Jawablah pertanyaan pengguna HANYA berdasarkan konteks dokumen berikut. Jika informasi tidak ada di dokumen, katakan tidak tahu.
     [KONTEKS DOKUMEN]
     ---
     File: {file_path} (Halaman {page_num})
     {chunk_text}
     ---
     <|im_end|>
     <|im_start|>user
     {user_query}
     <|im_end|>
     <|im_start|>assistant
     ```
3. **Streaming Token Generation:**
   * Token hasil inferensi Candle dialirkan secara *real-time* (*token-by-token streaming*) melalui channel `tokio::sync::mpsc` ke UI `egui` agar respons terasa instan tanpa jeda rendering.
4. **Interactive Source Citation:**
   * Setiap jawaban menyertakan tombol rujukan (misal: `[Lihat: laporan.pdf Halaman 4]` atau `[Lihat: Belanja Mingguan.md]`). Menekan tombol ini akan langsung membuka viewer PDF pada halaman terkait, atau membuka catatan terkait di editor markdown.

---

### 3.5 Modul 5: PDF Viewer & Editor Engine
1. **Document Viewer & Navigation:**
   * Viewer responsif dengan zoom, continuous scroll, dan thumbnail sidebar.
   * *Jump-to-source:* Melompat langsung ke halaman tempat chunk teks ditemukan oleh RAG/Search.
2. **PDF Editing & Manipulation (`lopdf`):**
   * **Text Modification / Injection:** Menambah teks baru, mengisi form PDF, atau menimpa area teks.
   * **Visual Annotation:** Highlighter area teks, kotak penanda (*bounding box*), garis bawah (*underline*), dan catatan (*sticky notes*).
   * **Page Management:**
     * *Merge:* Menggabungkan beberapa file PDF.
     * *Split:* Memisahkan rentang halaman tertentu.
     * *Rotate & Reorder:* Mengubah orientasi dan menyusun ulang urutan halaman.
     * *Delete Page:* Menghapus halaman spesifik.
   * **Metadata Editor:** Mengubah metadata Title, Author, dan Keywords.
3. **Export & Save:**
   * Simpan langsung (*overwrite with auto-backup*) atau simpan sebagai dokumen PDF baru yang terkompresi.

---

### 3.6 Modul 6: Internationalization (i18n) via `fluent-rs`

1. **Sistem Penerjemahan Berbasis Project Fluent:**
   * Menggunakan `fluent-rs` (implementasi Rust dari **Project Fluent**, Mozilla), bukan skema key-value sederhana ala gettext — mendukung *pluralization*, gender, dan sintaks bahasa natural per-locale secara langsung di file terjemahan.
   * Bahasa default: **Bahasa Indonesia (`id-ID`)**; bahasa kedua bawaan: **Inggris (`en-US`)**. Struktur dirancang agar bahasa baru cukup ditambah sebagai file `.ftl` baru di folder `locales/` tanpa perlu recompile.
2. **Struktur File Locale (`.ftl`):**
   * Satu folder per locale (`locales/id-ID/`, `locales/en-US/`), string dikelompokkan per modul (notes, markdown editor, search, chat AI, pdf, settings) agar mudah dikelola tim penerjemah.
   * Contoh isi `main.ftl`:
     ```ftl
     notes-grid-empty = Belum ada catatan. Ketuk tombol + untuk membuat catatan baru.
     notes-count =
         { $count ->
             [one] { $count } catatan
            *[other] { $count } catatan
         }
     pdf-merge-success = Berhasil menggabungkan { $count } berkas PDF.
     ```
3. **Runtime Language Switching:**
   * Pengguna dapat mengganti bahasa dari menu Settings tanpa restart aplikasi — `FluentBundle` aktif disimpan di state global `App`, seluruh pemanggilan string di `ui/*.rs` melalui fungsi lookup terpusat (`i18n::t(key, args)`), sehingga perubahan bahasa langsung ter-render ulang oleh `egui`.
4. **Fallback Chain & Locale-Aware Formatting:**
   * Jika sebuah kunci tidak ditemukan di bahasa aktif, sistem otomatis fallback ke `en-US`, lalu bila tetap tidak ada, menampilkan kunci mentah sebagai penanda visual (memudahkan QA menemukan string yang belum diterjemahkan).
   * Format tanggal/waktu (`created`/`modified` pada frontmatter catatan) dan angka mengikuti locale aktif (bukan hardcode format Indonesia), selaras dengan `chrono` yang sudah dipakai di Modul 1.
5. **Batas Cakupan (Scope):**
   * `fluent-rs` hanya menerjemahkan **string antarmuka aplikasi** (label tombol, menu, pesan error/notifikasi). Isi catatan atau dokumen pengguna **tidak** diterjemahkan otomatis oleh sistem i18n ini — jika pengguna ingin isi catatan diterjemahkan, itu dilakukan sebagai permintaan eksplisit ke AI Assistant (Modul 4), bukan bagian dari lapisan i18n.

---

### 3.7 Modul 7: Diagram Mermaid Native (Rust murni)

Diagram disimpan sebagai fence ```` ```mermaid ```` di dalam catatan `.md` — format teks yang paling dikenal agent AI dan dirender native oleh Obsidian. Tidak ada file kedua; sidecar `.canvas` (JSON Canvas) tetap dipakai hanya untuk papan tulis bebas (sticky, freehand, block-binding), dan Draw.io hanya untuk impor/ekspor. Modul: `src/mermaid/`.

1. **Pipeline (§3.7.1):** `source::preprocess` → parser per tipe → layout → `Scene` (display list netral) → `paint` (egui) atau `svg`. Tidak ada JavaScript, WebView, maupun Node.
2. **Parser & diagnostik (§3.7.2):** parser recursive-descent tulisan tangan per tipe. Frontmatter YAML (`title`, `config`), direktif `%%{init: …}%%`, komentar `%%`, `accTitle`/`accDescr` ditangani bersama. Parser tidak pernah panic dan tidak berhenti di baris pertama yang salah: setiap masalah menjadi `Diagnostic { line, col, severity, message }` (1-based, relatif terhadap isi fence) sehingga editor dan agent tahu posisi persisnya.
3. **Layout (§3.7.3):** keluarga *layered* (Sugiyama, setara dagre) untuk flowchart, class, state, ER: penghapusan siklus DFS, ranking longest-path + balancing, rank digandakan agar setiap edge punya slot label, dummy node untuk edge panjang, minimisasi persilangan barycenter yang menjaga subgraph tetap kontigu (border node per rank, seperti dagre), koordinat via regresi isotonik (pool-adjacent-violators) per layer. Subgraph tanpa edge lintas-batas di-layout rekursif dengan `direction`-nya sendiri. Sequence, pie, dan tipe linear/chart memakai layout khusus yang kecil. Pengukuran teks lewat trait `TextMeasure`: di aplikasi memakai lebar glyph egui asli (`GlyphTable`, dikumpulkan sekali per diagram), di CLI/test memakai tabel aproksimasi deterministik.
4. **Tema & gaya (§3.7.4):** tema `default`/`dark`/`forest`/`neutral`/`base` + `themeVariables`; `classDef`/`class`/`:::`/`style`/`linkStyle`. Tanpa tema eksplisit, diagram mengikuti mode terang/gelap aplikasi.
5. **Rendering (§3.7.5):** `Scene` digambar langsung ke shape epaint (tanpa tekstur/SVG), dengan culling di luar layar, kuantisasi ukuran font, dan triangulasi poligon non-konveks. Hasil parse+layout di-cache per sumber (`RenderCache`), jadi frame yang tidak berubah tidak mem-parse ulang. Klik node dengan `click … href` membuka URL atau `[[wikilink]]`; hover menampilkan tooltip. Ekspor SVG untuk agent/CLI.
6. **Tipe yang didukung (§3.7.6):** flowchart/graph (semua bentuk klasik + `@{ shape }` v11, semua jenis link, subgraph bersarang), sequence, class, state, ER, pie. Tipe lain (gantt, journey, gitGraph, mindmap, timeline, quadrant, requirement, C4, sankey, xychart, block, packet, kanban, architecture, radar, treemap, zenuml) sudah dikenali; sampai diimplementasikan, catatan menampilkan sumbernya sebagai code block plus diagnostik — tidak pernah crash.
7. **Antarmuka agent (§3.7.7):** `mnemonic-cli diagram list|validate|render` (validate/render berkas `.mmd` atau stdin tanpa vault) dan tool MCP `list_diagrams`, `validate_diagram`, `render_diagram`. Lihat `docs/agent-interface.md`.
8. **Target performa:** parse+layout+SVG flowchart 550 node / 743 edge ≈ 20 ms (release, termasuk start proses); diagram tipikal < 1 ms. Mermaid.js membutuhkan ratusan ms–detik untuk ukuran yang sama karena mengukur teks lewat DOM.

---

### 3.8 Modul 8: Sheet — Data Tabular di Vault (CSV / XLSX)

Posisi: data tabular yang *hidup di vault* dan terhubung ke catatan, RAG dan agent — **bukan** tiruan Excel. Untuk database, gunakan aplikasi TABULAR.

1. **Format & Sumber Kebenaran:**
   * `.csv` / `.tsv` adalah format sheet yang **bisa diedit**. File-nya tetap sumber kebenaran (plain text, ramah git & agent), sama seperti `.md`.
   * Delimiter dideteksi otomatis (`,` `;` tab `|`) — CSV dari Excel ber-locale Indonesia sering memakai `;`. BOM UTF-8 dibuang saat baca dan dipertahankan saat tulis. Baris pertama = header.
   * `.xlsx` / `.xlsm` / `.xls` / `.xlsb` / `.ods` dibuka **read-only** (`calamine`), satu tab per worksheet. Aplikasi **tidak pernah** menimpa workbook: menulis ulang XLSX akan menghilangkan formula, style, chart dan macro. Untuk mengedit, gunakan *Convert to CSV* (membuat `<nama> - <sheet>.csv` baru).
   * *Export as XLSX* (`rust_xlsxwriter`) membuat workbook baru dari sheet CSV; angka diekspor sebagai angka.
2. **Editor Grid:**
   * Grid tervirtualisasi (hanya baris terlihat yang dirender), edit sel inline, tambah/hapus baris & kolom, ganti nama kolom, sort per kolom (numerik bila kolom berisi angka), filter teks, undo/redo, indikator *dirty*.
   * Simpan atomic (temp file + rename), dengan pengecekan mtime sebelum menulis seperti catatan (§6 poin 4).
   * Footer agregat per kolom numerik (COUNT, SUM, AVG, MIN, MAX) — **bukan** formula engine per sel.
3. **Integrasi Vault:**
   * Sheet tampil di sidebar, bisa di-link `[[data.csv]]` dan di-embed `![[data.csv]]` (tabel pratinjau N baris pertama di Reading mode), dan muncul sebagai node di graph seperti PDF.
4. **Indeks & RAG (§3.3):**
   * Sheet di-chunk per kelompok baris dengan format `Kolom: nilai` per baris agar embedding bermakna, lalu masuk FTS5 + vektor dengan `doc_type = "sheet"`. Sitasi menunjuk ke nomor baris.
   * Pertanyaan agregasi angka dijawab lebih andal oleh `sheet query` yang deterministik daripada oleh LLM kecil; RAG dipakai untuk menemukan baris yang relevan.
5. **Agent Interface (§Fase 2):**
   * `VaultService` + CLI + MCP: `sheets list`, `sheet read` (paging, pilih worksheet), `sheet query` (filter kolom sederhana), `sheet set-cell`, `sheet append-row` (hanya CSV/TSV).
6. **Batas Cakupan:** tidak ada formula per sel, styling sel, chart, pivot, merge cell, atau penulisan XLSX in-place.

---

## 4. Struktur Proyek (Directory Layout)

```
mnemonic/
├── Cargo.toml
├── assets/
│   ├── models/
│   │   ├── embedding/        # all-MiniLM-L6-v2 ONNX weights
│   │   └── llm/              # Qwen2.5-1.5B-Instruct quantized weights
│   ├── fonts/                 # Inter, JetBrains Mono (custom egui fonts)
│   └── icons/                # Icon SVG / UI assets
├── themes/                   # Tema baca bawaan (*.toml), sekaligus contoh plugin
├── locales/
│   ├── id-ID/
│   │   └── main.ftl           # String UI Bahasa Indonesia (default)
│   └── en-US/
│       └── main.ftl           # String UI Bahasa Inggris
├── src/
│   ├── main.rs               # Entry point eframe runner
│   ├── app.rs                # State manajemen utama & layout egui
│   ├── notes/
│   │   ├── mod.rs
│   │   ├── vault.rs          # Pemilihan/inisialisasi folder Vault, scan awal
│   │   ├── note.rs           # Model Note (CRUD file .md)
│   │   ├── frontmatter.rs    # Parsing & penulisan YAML frontmatter
│   │   ├── watcher.rs        # File watcher (notify) & resolusi konflik
│   │   └── trash.rs          # Soft-delete & auto-cleanup 30 hari
│   ├── markdown/
│   │   ├── mod.rs
│   │   ├── live_blocks.rs    # Pemecahan body jadi blok Live (per baris / blok atomik)
│   │   ├── renderer/         # Render Live per blok -> widget egui (egui_commonmark), warna tema baca
│   │   ├── editor.rs         # Sesi catatan: mode Live/Source/Kanvas, undo, autosave
│   │   ├── wikilink.rs       # Resolver [[wikilink]], autocomplete, backlink graph
│   │   └── syntax_highlight.rs # Integrasi syntect untuk code block
│   ├── reading_theme/        # Tema baca: model warna, loader plugin TOML (§3.2.5)
│   ├── export/               # Cetak & ekspor HTML/PDF bertema (§3.2.5)
│   ├── i18n/
│   │   ├── mod.rs
│   │   ├── loader.rs         # Load & parse file .ftl per locale (fluent-bundle)
│   │   └── locale_manager.rs # State bahasa aktif, fallback chain, runtime switching
│   ├── core/
│   │   ├── mod.rs
│   │   ├── chunker.rs        # Text chunking & markdown tokenizer
│   │   ├── embedding.rs      # FastEmbed wrapper & cosine similarity
│   │   └── storage.rs        # SQLite index cache (bukan source-of-truth) & conversation store
│   ├── llm/
│   │   ├── mod.rs
│   │   ├── candle_engine.rs  # Inisialisasi Candle runtime & Qwen 2.5 loader
│   │   ├── prompt.rs         # Prompt template builder & context injector
│   │   └── stream.rs         # Token streamer & async worker channel
│   ├── pdf/
│   │   ├── mod.rs
│   │   ├── extractor.rs      # Ekstraksi teks PDF per halaman
│   │   ├── editor.rs         # Operasi lopdf (insert text, split, merge)
│   │   ├── annotator.rs      # Anotasi visual & highlight
│   │   └── renderer.rs       # Render halaman PDF ke egui Texture
│   └── ui/
│       ├── mod.rs
│       ├── notes_grid_view.rs   # Grid kartu ala Google Keep
│       ├── markdown_editor_view.rs # Editor & preview markdown ala Obsidian
│       ├── search_view.rs    # Panel pencarian semantik & similarity ranking
│       ├── chat_view.rs      # Antarmuka Chatbot RAG (Bubble chat + citations)
│       ├── pdf_view.rs       # Kanvas PDF viewer & visual editor
│       └── components/       # Custom buttons, modals, sliders, color picker
└── tests/
    ├── notes_vault_test.rs
    ├── markdown_render_test.rs
    ├── embedding_test.rs
    ├── candle_rag_test.rs
    └── pdf_ops_test.rs
```

---

## 5. Roadmap Pengembangan (Phases & Milestones)

| Fase | Durasi | Sasaran & Deliverables |
| :--- | :--- | :--- |
| **Fase 1: Vault & Note Core** | Minggu 1 | Pemilihan folder Vault, model Note berbasis file `.md`, parsing/penulisan frontmatter YAML, SQLite index cache, file watcher dasar, serta scaffolding sistem i18n (`fluent-rs`) dengan bahasa default `id-ID`. |
| **Fase 2: Markdown Editor & Renderer** | Minggu 2–3 | Live preview editor, integrasi `egui_commonmark` + `syntect`, checklist interaktif, wikilink `[[...]]` + autocomplete, backlinks panel, callout, tema light/dark. |
| **Fase 3: Notes Grid UI (Keep-style)** | Minggu 4 | Masonry grid kartu, quick capture bar, warna/pin/arsip/trash, label manager, drag-reorder, multi-select batch action. |
| **Fase 4: Ingestion & FastEmbed Core** | Minggu 5–6 | Setup Cargo tambahan, ekstraktor PDF, model embedding FastEmbed, verifikasi Cosine Similarity atas catatan & dokumen. |
| **Fase 5: Storage & Indexing** | Minggu 7 | Perluasan SQLite untuk caching vektor embedding dan metadata dokumen/catatan. Background task pool. |
| **Fase 6: Candle Engine & Qwen 2.5 Integration** | Minggu 8–9 | Integrasi Candle, loading bobot `Qwen2.5-1.5B-Instruct` Q4, pipeline RAG, dan streaming token generator via channel. |
| **Fase 7: Search & Chat UI** | Minggu 10 | Implementasi UI tab pencarian semantik (gabungan keyword + semantic) dan panel Chat RAG interaktif dengan bubble chat dan tautan sumber kutipan. |
| **Fase 8: PDF Viewer & Operations Engine** | Minggu 11–12 | Render PDF ke tekstur egui. Operasi manipulasi halaman (Merge, Split, Rotate, Delete) menggunakan `lopdf`. |
| **Fase 9: PDF Annotation & Editor Canvas** | Minggu 13–14 | Kanvas anotasi (Highlight, Underline, Text Injection), metadata editing, dan fitur Export/Save PDF. |
| **Fase 10: Optimasi, Benchmarking & Polish** | Minggu 15–16 | Optimasi inferensi Candle (multi-threading SIMD/AVX2), performa render markdown pada dokumen panjang, memory profiling, error handling, dan build release cross-platform. |
| **Fase 11: Internationalization Rollout & Translation QA** | Minggu 17 | Audit seluruh string UI agar melalui kunci `fluent-rs`, lengkapi terjemahan `id-ID` & `en-US`, uji fallback chain, dan verifikasi format tanggal/angka locale-aware. |

---

## 6. Manajemen Risiko & Strategi Mitigasi

1. **Konsumsi RAM / VRAM dari LLM:**
   * *Mitigasi:* Gunakan kuantisasi 4-bit (Q4_K_M) dari model Qwen 2.5 1.5B sehingga konsumsi memori model hanya berkisar ~1.1–1.3 GB, memungkinkan aplikasi berjalan lancar di PC spek standar maupun mobile.
2. **Kelancaran UI saat LLM Menghasilkan Jawaban:**
   * *Mitigasi:* Jalankan seluruh inferensi Candle di thread pekerja terpisah (`std::thread::spawn` atau Tokio blocking thread). Kirim token ke UI thread melalui `crossbeam-channel` / `tokio::sync::mpsc` dengan `ctx.request_repaint()` untuk pembaruan instan.
3. **Halusinasi LLM pada Dokumen Kompleks:**
   * *Mitigasi:* Sistem prompt ketat (*strict grounding*), menyertakan potongan chunk teks yang relevan, serta menampilkan skor kemiripan (*similarity threshold cutoff*) agar LLM tidak menjawab jika tidak ada dokumen yang cocok.
4. **Sinkronisasi File Eksternal (Watcher Conflict):**
   * *Risiko:* Jika pengguna mengedit file `.md` langsung dari editor luar (VSCode/Obsidian, atau via sinkronisasi cloud) saat aplikasi terbuka, dapat terjadi race condition yang menimpa perubahan satu sama lain.
   * *Mitigasi:* File watcher (`notify`) + pengecekan hash/mtime sebelum menulis; jika terdeteksi perubahan eksternal setelah file dibuka di editor, tampilkan dialog resolusi konflik ("Muat ulang dari disk" / "Timpa dengan versi aplikasi" / "Gabungkan manual").
5. **Performa Live Preview Markdown pada Dokumen Panjang:**
   * *Risiko:* Rendering ulang seluruh AST pada setiap keystroke dapat menyebabkan lag pada catatan panjang atau banyak elemen kompleks (tabel besar, banyak gambar).
   * *Mitigasi:* Incremental re-render hanya pada paragraf/blok yang berubah (*dirty region tracking*), debounce rendering (~100–150ms), dan virtualized scrolling untuk dokumen panjang.
6. **Integritas Frontmatter YAML:**
   * *Risiko:* Kesalahan parsing atau penulisan YAML dapat merusak metadata catatan (pin, warna, tag) atau bahkan membuat file tidak terbaca.
   * *Mitigasi:* Validasi skema ketat dengan nilai default fallback saat parsing gagal, serta auto-backup (`.bak`) sebelum setiap operasi tulis pada frontmatter.
7. **Dukungan Matematika/LaTeX Terbatas di egui:**
   * *Risiko:* Tidak ada rendering LaTeX native yang matang di ekosistem egui, sehingga fitur rumus matematika (`$...$`) berisiko molor atau kualitas visualnya terbatas dibanding Obsidian/KaTeX web.
   * *Mitigasi:* Jadikan fitur ini opsional/fase lanjutan (bukan blocker rilis awal), dengan fallback menampilkan teks LaTeX mentah dengan styling monospace jika rendering penuh belum tersedia.
8. **String UI Tidak Terjemahan Lengkap (Missing i18n Keys):**
   * *Risiko:* Modul baru ditambahkan tanpa string diarahkan ke `fluent-rs`, atau bahasa kedua (`en-US`) tertinggal dari bahasa utama (`id-ID`) sehingga muncul teks campur bahasa di UI.
   * *Mitigasi:* Wajibkan seluruh string UI baru melalui pemanggilan kunci Fluent terpusat (`i18n::t(key, args)`) — hindari literal string langsung di `ui/*.rs`; fallback otomatis ke `en-US` lalu ke kunci mentah bila hilang; audit kelengkapan terjemahan rutin di Fase 11.
