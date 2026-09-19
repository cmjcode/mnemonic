## MNEMONIC — Bahasa Indonesia
## Setiap key di sini WAJIB ada juga di en-US/main.ftl (dicek oleh tes).

## ── Sambutan & vault ────────────────────────────────────────────────
welcome-title = Selamat datang di MNEMONIC
welcome-subtitle = Catatan, kanvas, dan PDF Anda — dengan asisten AI yang berjalan di komputer sendiri.
welcome-create-vault = Buat Vault Baru
welcome-open-folder = Buka Folder…
welcome-default-vault-name = Catatan Saya
welcome-feature-notes = Tulis catatan Markdown, gambar di kanvas, dan anotasi PDF
welcome-feature-ai = Cari berdasarkan makna dan tanya asisten AI tentang isi vault
welcome-feature-private = Semua data tersimpan sebagai berkas biasa di komputer Anda
welcome-note-title = Selamat Datang 👋
welcome-note-body =
    # Mulai dari sini

    Vault ini hanyalah folder biasa di komputer Anda. Setiap catatan adalah berkas Markdown, jadi data Anda selalu milik Anda.

    ## Dasar-dasar
    - [ ] Buat catatan baru dengan ⌘N atau tombol "Catatan Baru" di kiri
    - [ ] Ketik / di awal baris untuk menyisipkan judul, daftar centang, tabel, dan lainnya
    - [ ] Hubungkan catatan dengan mengetik [[ lalu pilih judulnya
    - [ ] Cari apa saja dengan ⌘F — hasil yang mirip maknanya juga muncul
    - [ ] Buka palet perintah dengan ⌘K untuk melompat ke mana pun

    ## Tips
    - Catatan tersimpan otomatis saat Anda berhenti mengetik.
    - Salah hapus? Klik "Urungkan" pada notifikasi, atau pulihkan dari Sampah.
    - Klik kanan pada catatan atau folder untuk melihat semua aksi.
    - Tekan ⌘/ untuk melihat semua pintasan keyboard.

    Silakan hapus catatan ini kapan saja.
vault-pick-folder = Pilih Folder Vault

## ── Bar atas & pengaturan ───────────────────────────────────────────
topbar-show-sidebar = Tampilkan sidebar
topbar-hide-sidebar = Sembunyikan sidebar
topbar-ai-assistant = Asisten AI
topbar-settings = Pengaturan
topbar-indexing = Mengindeks { $count }…
topbar-indexing-hint = Menyiapkan pencarian makna & asisten AI. Anda tetap bisa bekerja seperti biasa.
settings-appearance = Tampilan
settings-theme-light = Terang
settings-theme-dark = Gelap
settings-language = Bahasa
settings-switch-vault = Buka vault lain…
settings-command-palette = Palet perintah
settings-shortcuts = Pintasan keyboard

## ── Sidebar ─────────────────────────────────────────────────────────
sidebar-switch-vault = Ganti vault
sidebar-recent-vaults = Vault terbaru
sidebar-open-other-vault = Buka folder lain…
sidebar-rescan = Muat ulang berkas
sidebar-new-other = Buat yang lain…
sidebar-new-canvas = Kanvas baru
sidebar-new-folder = Folder baru
sidebar-library = Pustaka
sidebar-all = Semua Dokumen
sidebar-notes-only = Catatan
sidebar-whiteboards-only = Kanvas
sidebar-pdfs-only = PDF
sidebar-archived = Arsip
sidebar-trash = Sampah
sidebar-folders = Folder
sidebar-folders-empty = Belum ada berkas. Buat catatan pertama Anda di atas.
sidebar-folder-empty = Folder kosong
sidebar-expand-all = Buka semua folder
sidebar-collapse-all = Tutup semua folder
sidebar-more-actions = Aksi lainnya
sidebar-new-note-here = Catatan baru di sini
sidebar-new-canvas-here = Kanvas baru di sini
sidebar-new-subfolder = Subfolder baru
sidebar-open = Buka
sidebar-move-to = Pindahkan ke…
sidebar-tags = Label
sidebar-tags-empty = Tambahkan tag di frontmatter catatan untuk mengelompokkannya.
sidebar-manage-tags = Kelola label

## ── Beranda (grid) ──────────────────────────────────────────────────
grid-item-count = { $count } item
grid-sort = Urutkan
sort-modified = Terakhir diubah
sort-created = Tanggal dibuat
sort-title = Judul (A–Z)
sort-color = Warna
grid-search-results = { $count } hasil untuk “{ $query }”
grid-clear-search = Hapus pencarian
grid-show-all = Lihat semua dokumen
grid-empty-trash = Kosongkan Sampah
grid-trash-info = Item di Sampah dihapus permanen otomatis setelah 30 hari.
grid-semantic-title = Hasil pencarian AI
grid-semantic-hint = gabungan makna dan kata kunci di isi catatan & PDF
grid-semantic-match = { $percent }% cocok

selection-mode-on = Pilih beberapa
selection-mode-off = Selesai memilih
selection-count = { $count } dipilih
selection-select-all = Pilih semua
selection-archive = Arsipkan
selection-trash = Pindahkan ke Sampah

notes-new = Catatan Baru
notes-pin = Sematkan di atas
notes-unpin = Lepas sematan
notes-pinned = Disematkan
card-archive = Arsipkan
card-unarchive = Keluarkan dari arsip
card-trash = Pindahkan ke Sampah
card-restore = Pulihkan
card-delete-permanent = Hapus permanen
card-color = Warna
card-color-none = Tanpa warna

time-just-now = baru saja
time-minutes-ago = { $count } menit lalu
time-hours-ago = { $count } jam lalu
time-days-ago = { $count } hari lalu

empty-vault-title = Mulai catatan pertama Anda
empty-vault-body = Tulis ide, buat daftar tugas, atau gambar di kanvas. Semuanya tersimpan otomatis.
empty-vault-tip = Tip: tekan ⌘N kapan saja untuk membuat catatan baru.
empty-search-title = Tidak ada hasil untuk “{ $query }”
empty-search-body = Coba kata lain yang lebih umum, atau periksa ejaannya.
empty-trash-title = Sampah kosong
empty-trash-body = Item yang Anda hapus akan muncul di sini dan bisa dipulihkan selama 30 hari.
empty-archive-title = Belum ada arsip
empty-archive-body = Arsipkan catatan yang sudah tidak aktif agar beranda tetap rapi.
empty-pdf-title = Belum ada PDF
empty-pdf-body = Impor PDF untuk membacanya, memberi anotasi, dan mencarinya bersama catatan Anda.
empty-filter-title = Tidak ada dokumen di sini
empty-filter-body = Belum ada dokumen yang cocok dengan filter ini.

## ── Editor ──────────────────────────────────────────────────────────
editor-back = Kembali
editor-untitled = Tanpa Judul
editor-rename-hint = Klik untuk mengganti judul
editor-mode-note = Catatan
editor-mode-edgeless = Kanvas
editor-mode-split = Teks + Diagram
editor-mode-hint = Beralih antara catatan, teks + diagram, dan kanvas
editor-saved = Tersimpan
editor-saving = Menyimpan…
editor-save-failed = Gagal menyimpan
editor-undo = Urungkan
editor-redo = Ulangi
editor-outline = Daftar isi
editor-outline-toggle = Tampilkan/sembunyikan daftar isi
editor-outline-empty = Tambahkan judul (# Judul) untuk membuat daftar isi.
editor-backlinks = Ditautkan dari
editor-linked-mentions = Tautan masuk
editor-unlinked-mentions-empty = Tidak ada penyebutan yang belum ditautkan.
editor-outgoing-links = Tautan keluar
editor-outgoing-empty = Belum ada [[tautan]] di catatan ini.
editor-local-graph-empty = Belum ada tautan; graf muncul setelah catatan saling terhubung.
editor-properties = Properti
editor-tags = Label
editor-aliases = Alias
editor-add-tag = + label
editor-add-alias = + alias
editor-remove = Hapus
editor-created = Dibuat
editor-modified = Diubah
editor-status-backlinks = { $count } tautan masuk
editor-char-count = { $count } karakter
editor-backlinks-empty = Belum ada catatan lain yang menautkan ke sini.
editor-word-count = { $count } kata
editor-reading-time = ~{ $minutes } menit baca
editor-placeholder = Mulai menulis… Ketik / untuk menyisipkan elemen, [[ untuk menautkan catatan.
editor-slash-header = Sisipkan
editor-link-header = Tautkan ke catatan
editor-popup-hint = ↑↓ pilih · Enter sisipkan · Esc tutup

slash-heading-1 = Judul besar
slash-heading-2 = Subjudul
slash-checklist = Daftar centang
slash-bullet-list = Daftar poin
slash-quote = Kutipan
slash-code-block = Blok kode
slash-callout-note = Kotak catatan
slash-callout-warning = Kotak peringatan
slash-table = Tabel
slash-divider = Garis pemisah

## ── Kanvas ──────────────────────────────────────────────────────────
canvas-untitled = Kanvas Tanpa Judul
canvas-empty-hint = Pilih alat di kiri, lalu klik atau seret untuk mulai menggambar
canvas-new-sticky = Catatan baru
canvas-edit-hint = Esc untuk selesai
canvas-edit-hint-bound = Terikat ke catatan · teks ini juga diubah di Markdown
canvas-bind = Ikat ke catatan
canvas-unbind = Lepas dari catatan
canvas-import-bound = { $count } teks masuk ke Markdown
canvas-edit-done = Selesai
canvas-tool-select = Pilih & pindahkan
canvas-tool-pan = Geser kanvas
canvas-tool-sticky = Catatan tempel
canvas-tool-rectangle = Persegi
canvas-tool-rounded = Persegi membulat
canvas-tool-ellipse = Elips
canvas-tool-diamond = Belah ketupat
canvas-tool-connector = Panah penghubung
canvas-tool-pen = Pena
canvas-tool-eraser = Penghapus
canvas-import-drawio = Impor dari Draw.io
canvas-export-drawio = Ekspor ke Draw.io
canvas-import-success = { $count } elemen berhasil diimpor
canvas-import-skipped = { $count } item tidak dapat ditampilkan
canvas-import-empty = Tidak ada isi diagram di file ini
canvas-import-failed = Gagal mengimpor diagram
canvas-export-success = Diagram berhasil diekspor
canvas-export-failed = Gagal mengekspor diagram
canvas-zoom-in = Perbesar
canvas-zoom-out = Perkecil
canvas-zoom-reset = Kembalikan ke 100%
canvas-width-thin = Tipis
canvas-width-medium = Sedang
canvas-width-thick = Tebal
color-yellow = Kuning
color-blue = Biru
color-green = Hijau
color-pink = Merah muda
color-purple = Ungu
color-orange = Oranye
color-red = Merah
color-graphite = Grafit

## ── Pencarian, palet, asisten AI ────────────────────────────────────
search-placeholder = Cari catatan, PDF, atau topik…
command-palette-hint = Ketik perintah atau judul catatan…
command-palette-empty = Tidak ada yang cocok
command-palette-footer = ↑↓ pilih · Enter jalankan · Esc tutup
palette-cat-actions = Aksi
palette-cat-navigate = Buka
palette-cat-documents = Dokumen
palette-cat-view = Tampilan
palette-search = Cari di vault
palette-ask-ai = Tanya asisten AI
palette-toggle-sidebar = Tampilkan/sembunyikan sidebar
palette-toggle-theme = Ganti tema terang/gelap
palette-toggle-language = Switch to English

chat-title = Asisten AI
chat-subtitle = Menjawab berdasarkan catatan & PDF di vault Anda — berjalan lokal.
chat-close = Tutup
chat-clear = Mulai percakapan baru
chat-empty-title = Tanyakan apa saja tentang vault Anda
chat-empty = Jawaban disertai sumber yang bisa langsung Anda buka.
chat-starter-summary = Ringkas poin penting dari catatan terbaru saya
chat-starter-related = Catatan mana saja yang membahas topik yang sama?
chat-starter-ideas = Bantu saya menyusun ide dari catatan-catatan ini
chat-placeholder = Tanyakan sesuatu…
chat-send = Kirim (Enter)
chat-sources = Sumber
chat-open-source = Buka sumber ini
chat-thinking = Sedang mencari di vault dan menyusun jawaban…
chat-error = Gagal memproses pertanyaan
chat-citation-page = { $name } · hal. { $page }

## ── PDF ─────────────────────────────────────────────────────────────
pdf-import = Impor PDF
pdf-page-of = Halaman { $current } dari { $total }
pdf-prev-page = Halaman sebelumnya
pdf-next-page = Halaman berikutnya
pdf-more = Aksi halaman & dokumen
pdf-section-page = Halaman ini
pdf-section-document = Dokumen
pdf-rotate-left = Putar ke kiri
pdf-rotate-right = Putar ke kanan
pdf-delete-page = Hapus halaman
pdf-delete-page-confirm = Halaman ini akan dihapus dan hasilnya disimpan sebagai berkas PDF baru. Berkas asli tidak berubah.
pdf-split = Ambil halaman
pdf-split-to = sampai
pdf-split-go = Simpan sebagai PDF baru…
pdf-merge = Gabungkan dengan PDF lain…
pdf-op-success = Berhasil disimpan sebagai berkas baru
pdf-op-error = memproses PDF
pdf-render-unavailable = Halaman PDF tidak bisa ditampilkan
pdf-annotate-none = Jelajah
pdf-annotate-highlight = Sorot
pdf-annotate-underline = Garis bawah
pdf-annotate-sticky = Catatan tempel
pdf-annotate-text = Sisipkan teks
pdf-annotate-color = Warna anotasi
pdf-annotate-sticky-prompt = Isi catatan tempel
pdf-annotate-text-prompt = Teks yang disisipkan
pdf-annotate-add = Tambahkan
pdf-annotate-cancel = Batal
pdf-unsaved-annotations = { $count } anotasi belum disimpan
pdf-metadata-button = Info dokumen
pdf-metadata-window-title = Info dokumen
pdf-metadata-field-title = Judul
pdf-metadata-field-author = Penulis
pdf-metadata-field-keywords = Kata kunci
pdf-metadata-hint = Perubahan diterapkan saat Anda menekan Simpan atau Ekspor.
pdf-metadata-close = Selesai
pdf-save = Simpan
pdf-save-confirm = Anotasi dan info dokumen akan disimpan ke berkas asli. Cadangan (.bak) dibuat otomatis.
pdf-save-success = Tersimpan. Cadangan: { $backup }
pdf-export = Ekspor sebagai berkas baru…

## ── Dialog ──────────────────────────────────────────────────────────
confirm-cancel = Batal
confirm-yes = Hapus permanen
confirm-delete-title = Hapus permanen?
confirm-delete-body = Catatan ini akan dihapus selamanya dan tidak bisa dipulihkan.
confirm-empty-trash-title = Kosongkan Sampah?
confirm-empty-trash-body = Semua catatan di Sampah akan dihapus selamanya. Tindakan ini tidak bisa dibatalkan.
confirm-empty-trash-yes = Kosongkan Sampah
conflict-title = Berkas berubah di luar aplikasi
conflict-body = “{ $name }” diubah oleh program lain sejak dibuka di sini, dan ada perubahan yang belum tersimpan. Pilih versi mana yang dipakai.
conflict-reload = Muat ulang dari disk
conflict-overwrite = Timpa dengan versi ini
conflict-copy = Simpan sebagai salinan
folder-new-title = Folder baru
folder-new-message = Beri nama folder baru.
folder-new-placeholder = mis. Proyek, Kuliah, Resep
folder-new-confirm = Buat folder
rename-folder-title = Ganti nama folder
rename-file-title = Ganti nama
rename-message = Nama baru untuk “{ $name }”.
rename-placeholder = Nama baru
rename-confirm = Simpan
move-modal-title = Pindahkan “{ $name }”
move-modal-search = Cari folder…
move-modal-root = Folder utama vault
tag-manager-title = Kelola label
tag-manager-empty = Belum ada label. Tambahkan tag di frontmatter catatan, misalnya: tags: [kerja, ide]
tag-note-count = { $count } catatan
tag-rename = Ganti nama
tag-delete = Hapus label

## ── Pintasan ────────────────────────────────────────────────────────
shortcut-palette = Palet perintah
shortcut-new-note = Catatan baru
shortcut-search = Cari
shortcut-save = Simpan sekarang
shortcut-toggle-source = Beralih ke sumber Markdown penuh
shortcut-print = Cetak catatan
shortcut-sidebar = Tampilkan/sembunyikan sidebar
shortcut-ai = Asisten AI
shortcut-back = Kembali ke beranda
shortcut-slash = Sisipkan elemen (di awal baris)
shortcut-wikilink = Tautkan ke catatan lain

## ── Notifikasi ──────────────────────────────────────────────────────
toast-undo = Urungkan
toast-note-trashed = “{ $title }” dipindahkan ke Sampah
toast-item-trashed = “{ $title }” dipindahkan ke Sampah
toast-note-restored = “{ $title }” dipulihkan
toast-restored = Berhasil dipulihkan
toast-archived = Catatan diarsipkan
toast-unarchived = Catatan dikeluarkan dari arsip
toast-batch-archived = { $count } catatan diarsipkan
toast-deleted-permanently = { $count } catatan dihapus permanen
toast-moved = Dipindahkan ke { $folder }
toast-name-taken = Nama “{ $name }” sudah dipakai di folder ini
toast-open-failed = Berkas tidak bisa dibuka — mungkin sudah dipindah atau dihapus
toast-pdfs-imported = { $count } PDF berhasil diimpor

## ── Kesalahan ───────────────────────────────────────────────────────
error-banner = Gagal { $context }: { $error }
error-context-autosave = menyimpan otomatis
error-context-indexing = mengindeks catatan untuk pencarian
error-context-save-note = menyimpan catatan
error-context-delete-note = menghapus berkas
error-context-move-note = memulihkan catatan
error-context-create-note = membuat catatan
error-context-open-vault = membuka vault
error-context-create-folder = membuat folder
error-context-rename-folder = mengganti nama folder
error-context-move-file = memindahkan berkas
error-context-delete-folder = memindahkan folder ke Sampah
error-context-trash-note = memindahkan catatan ke Sampah

## ── Tambahan / Additional ──
toast-close-unsaved = Perubahan terakhir gagal disimpan. Tutup sekali lagi untuk keluar tanpa menyimpan.
canvas-count-sticky = { $count } catatan tempel
canvas-count-shapes = { $count } bentuk
canvas-count-connectors = { $count } penghubung
canvas-count-strokes = { $count } coretan
canvas-count-empty = Kanvas kosong

## Graph relasi & tautan
graph-title = Grafik relasi
graph-empty = Belum ada catatan untuk ditampilkan.
graph-filter = Saring node…
graph-show-orphans = Tampilkan catatan tanpa tautan
graph-show-ghosts = Tampilkan tautan ke catatan yang belum ada
graph-show-pdfs = Tampilkan PDF
graph-show-semantic = Hubungan AI (kemiripan makna)
graph-show-semantic-hint = Garis putus-putus menghubungkan dokumen yang isinya mirip tetapi belum saling ditautkan.
graph-counts = { $nodes } node · { $edges } hubungan
graph-fit = Paskan ke layar
graph-kind-note = Catatan
graph-kind-canvas = Kanvas
graph-kind-pdf = PDF
graph-kind-ghost = Belum dibuat
graph-node-links = { $count } hubungan
shortcut-graph = Grafik relasi
shortcut-daily = Catatan harian hari ini
shortcut-cheatsheet = Daftar pintasan ini
editor-unlinked-mentions = Disebut tanpa tautan
editor-link-mention = Jadikan tautan
editor-related = Terkait (AI)
editor-related-empty = Belum ada dokumen lain yang mirip.
editor-insert-link = Tautkan ke catatan ini
editor-local-graph = Grafik lokal
toast-links-updated = Tautan di { $count } catatan diperbarui
toast-mention-linked = Tautan ditambahkan di "{ $title }"
toast-link-pdf-missing = PDF "{ $name }" tidak ditemukan di vault
toast-rerank-on = Pengurutan ulang AI aktif (model diunduh saat pencarian pertama)
toast-rerank-off = Pengurutan ulang AI dimatikan
toast-note-reloaded = “{ $title }” dimuat ulang dari disk
toast-conflict-copy-saved = Versi ini disimpan sebagai “{ $title }”
toast-filenames-migrated = { $count } berkas diganti namanya sesuai judul
palette-migrate-filenames = Ganti nama berkas UUID sesuai judul
palette-daily-note = Buka catatan harian hari ini
palette-insert-template = Sisipkan templat: { $name }
palette-cat-templates = Templat
palette-rerank-on = Aktifkan pengurutan ulang AI untuk pencarian
palette-rerank-off = Matikan pengurutan ulang AI untuk pencarian
grid-match-keyword = kata kunci
grid-match-both = { $percent }% · kata kunci

## Sheet — CSV / XLSX (§3.8)
sidebar-new-sheet = Sheet baru
sidebar-new-sheet-here = Sheet baru di sini
sheet-import = Impor spreadsheet
sheet-import-filter = Spreadsheet
sheet-untitled = Sheet Tanpa Judul
sheet-read-only = Hanya-baca
sheet-read-only-workbook = Workbook dibuka hanya-baca agar formula, format, dan grafiknya tidak pernah hilang. Gunakan “Konversi ke CSV” untuk mengedit salinannya.
sheet-read-only-encoding = Berkas ini bukan UTF-8, jadi dibuka hanya-baca — menyimpannya bisa merusak karakter.
sheet-filter = Saring baris…
sheet-add-row = Tambah baris
sheet-add-column = Tambah kolom
sheet-export-xlsx = Ekspor sebagai XLSX
sheet-convert-csv = Konversi ke CSV (salinan yang bisa diedit)
sheet-size = { $rows } baris × { $cols } kolom
sheet-showing = { $shown } ditampilkan
sheet-unsaved = belum disimpan
sheet-empty = Sheet ini belum punya kolom.
sheet-sort-hint = Klik untuk mengurutkan · klik kanan untuk opsi lain
sheet-sort-asc = Urutkan naik
sheet-sort-desc = Urutkan turun
sheet-rename-column = Ganti nama kolom
sheet-insert-column-left = Sisipkan kolom di kiri
sheet-insert-column-right = Sisipkan kolom di kanan
sheet-delete-column = Hapus kolom
sheet-insert-row-above = Sisipkan baris di atas
sheet-insert-row-below = Sisipkan baris di bawah
sheet-delete-row = Hapus baris
sheet-stats = { $col } — jumlah data { $count } · total { $sum } · rata-rata { $avg } · min { $min } · maks { $max }
sheet-stats-text = { $col } — { $filled } terisi
chat-citation-row = { $name } · baris { $row }
toast-sheet-exported = Berhasil mengekspor { $name }
toast-sheet-converted = Berhasil membuat { $name }
toast-sheet-conflict = Berkas berubah di disk, jadi editanmu disimpan ke { $name }
error-context-open-sheet = membuka sheet
error-context-save-sheet = menyimpan sheet
error-context-export-sheet = mengekspor sheet
error-context-import-sheet = mengimpor spreadsheet
toast-link-sheet-missing = Sheet "{ $name }" tidak ditemukan di vault
graph-kind-sheet = Sheet

## Live editor & reading themes (§3.2.1, §3.2.5)
editor-live-hint = Tulis Markdown… / untuk sisipan, [[ untuk tautan, Esc untuk selesai
reading-theme = Tema baca
reading-theme-open-folder = Buka folder plugin tema
reading-theme-reload = Muat ulang tema
reading-theme-reloaded = { $count } tema dimuat
reading-theme-note-override = Catatan ini memakai tema “{ $name }” (frontmatter theme:)
theme-load-problems = { $count } masalah saat memuat tema — lihat log
theme-folder-failed = Folder tema tidak bisa dibuka
export-menu = Cetak & ekspor
export-print = Cetak…
export-pdf = Ekspor PDF…
export-html = Ekspor HTML…
export-print-opened = Halaman cetak dibuka di browser — warna tema ikut tercetak
export-pdf-started = Membuat PDF…
export-done = Tersimpan: { $file }
export-failed = Ekspor gagal
export-no-browser = PDF butuh Chrome, Edge, Chromium, atau Brave. Pakai Cetak → Simpan sebagai PDF, atau ekspor HTML.
palette-print = Cetak catatan ini
palette-export-pdf = Ekspor catatan ini ke PDF
palette-export-html = Ekspor catatan ini ke HTML
palette-toggle-source = Beralih ke sumber Markdown penuh
palette-reading-theme = Tema baca: { $name }
palette-open-theme-folder = Buka folder plugin tema
