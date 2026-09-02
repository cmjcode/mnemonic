app-title = MNEMONIC

vault-empty = Belum ada catatan. Ketuk tombol + untuk membuat catatan baru.
vault-pick-folder = Pilih Folder Vault
vault-select-prompt = Pilih folder Vault untuk mulai mencatat.

notes-count =
    { $count ->
        [one] { $count } catatan
       *[other] { $count } catatan
    }

notes-new = Catatan Baru
notes-delete = Hapus
notes-pin = Sematkan
notes-unpin = Lepas Sematan

app-status-ready = Siap

editor-back = Kembali
editor-mode-source = Edit / Sumber
editor-mode-live-preview = Pratinjau
editor-mode-reading = Baca / Pratinjau
editor-undo = Urungkan
editor-redo = Ulangi
editor-outline = Daftar Isi
editor-backlinks = Tautan Balik
editor-backlinks-empty = Belum ada catatan lain yang menautkan ke sini.
editor-word-count = { $count } kata
editor-reading-time = ~{ $minutes } menit baca

sidebar-all = Semua Dokumen
sidebar-notes-only = Catatan Markdown
sidebar-pdfs-only = Dokumen PDF
sidebar-archived = Diarsipkan
sidebar-trash = Sampah
sidebar-tags = Label
sidebar-manage-tags = Kelola Label

sort-modified = Terakhir Diubah
sort-created = Tanggal Dibuat
sort-title = Judul
sort-color = Warna

selection-mode-on = Pilih Banyak
selection-mode-off = Batal Pilih
selection-archive = Arsipkan Terpilih
selection-trash = Sampah-kan Terpilih

card-archive = Arsipkan
card-unarchive = Batalkan Arsip
card-trash = Pindahkan ke Sampah
card-restore = Pulihkan
card-delete-permanent = Hapus Permanen

confirm-archive-title = Arsipkan Catatan?
confirm-archive-body = Catatan ini akan dipindahkan ke Arsip dan disembunyikan dari daftar utama.
confirm-archive-yes = Ya, Arsipkan

confirm-unarchive-title = Batalkan Arsip?
confirm-unarchive-body = Catatan ini akan dikembalikan ke daftar dokumen utama.
confirm-unarchive-yes = Batalkan Arsip

confirm-trash-title = Pindahkan ke Sampah?
confirm-trash-body = Catatan ini akan dipindahkan ke Tong Sampah. Anda dapat memulihkannya nanti.
confirm-trash-yes = Pindahkan ke Sampah

confirm-delete-title = Hapus Permanen?
confirm-delete-body = Catatan ini akan dihapus permanen dan tidak bisa dikembalikan.
confirm-yes = Ya, Hapus
confirm-cancel = Batal

tag-manager-title = Kelola Label
tag-rename = Ganti Nama
tag-delete = Hapus

grid-empty-filtered = Tidak ada catatan yang cocok.

nav-notes = Catatan
nav-search = Pencarian
nav-chat = Obrolan
nav-pdf = PDF

search-placeholder = Cari catatan atau dokumen...
search-button = Cari
search-prompt = Ketik kata kunci untuk mencari di seluruh vault (judul, isi, dan makna semantik).
search-no-results = Tidak ditemukan hasil yang cocok.
search-error = Pencarian gagal

chat-placeholder = Tanyakan sesuatu tentang catatan/dokumen Anda...
chat-send = Kirim
chat-empty = Mulai percakapan dengan bertanya tentang isi vault Anda.
chat-sources = Sumber:
chat-thinking = Memikirkan jawaban...
chat-error = Gagal memproses pertanyaan

pdf-import = Impor PDF
pdf-library-empty = Belum ada PDF yang diimpor. Ketuk "Impor PDF" untuk menambahkan.
pdf-open = Buka
pdf-remove = Hapus dari Daftar
pdf-back = ← Kembali
pdf-page-of = Halaman { $current } / { $total }
pdf-zoom = Perbesar
pdf-rotate-left = ↺ Putar Kiri
pdf-rotate-right = ↻ Putar Kanan
pdf-delete-page = Hapus Halaman
pdf-delete-page-confirm = Halaman ini akan dihapus ke berkas PDF baru. Berkas asli tidak berubah. Lanjutkan?
pdf-split = Pisahkan halaman
pdf-split-to = sampai
pdf-split-go = Pisahkan ke Berkas Baru
pdf-merge = Gabung dengan PDF Lain...
pdf-op-success = Berhasil disimpan sebagai berkas baru
pdf-op-error = Operasi PDF gagal
pdf-render-unavailable = Tidak bisa menampilkan halaman (pustaka PDFium tidak ditemukan)

pdf-annotate-none = Tidak ada (jelajah)
pdf-annotate-highlight = Sorot
pdf-annotate-underline = Garis Bawah
pdf-annotate-sticky = Catatan Tempel
pdf-annotate-text = Sisipkan Teks
pdf-annotate-color = Warna
pdf-annotate-sticky-prompt = Isi Catatan Tempel
pdf-annotate-text-prompt = Isi Teks yang Disisipkan
pdf-annotate-add = Tambahkan
pdf-annotate-cancel = Batal
pdf-metadata-button = Metadata
pdf-metadata-window-title = Edit Metadata PDF
pdf-metadata-field-title = Judul
pdf-metadata-field-author = Penulis
pdf-metadata-field-keywords = Kata Kunci
pdf-metadata-close = Tutup
pdf-save = Simpan
pdf-save-confirm = Berkas asli akan ditimpa (cadangan .bak dibuat otomatis). Lanjutkan?
pdf-save-success = Berhasil disimpan ke berkas asli (cadangan: { $backup })
pdf-export = Ekspor sebagai Baru...

# §Fase 10: banner kesalahan umum yang menggantikan kegagalan yang
# sebelumnya hanya dicatat di log (`log::warn!`) tanpa terlihat pengguna —
# lihat `MnemonicApp::report_error`.
error-banner = Kesalahan saat { $context }: { $error }
error-context-autosave = menyimpan otomatis
error-context-save-note = menyimpan catatan
error-context-delete-note = menghapus catatan
error-context-move-note = memindahkan catatan
error-context-create-note = membuat catatan
error-context-open-vault = membuka vault

nav-canvas = Whiteboard
editor-mode-page = Halaman
editor-mode-edgeless = Kanvas
sidebar-whiteboards-only = Whiteboards & Kanvas
command-palette-title = Perintah & Navigasi Cepat
command-palette-hint = Ketik judul catatan, nama PDF, atau perintah...
command-palette-new-note = ＋ Buat Catatan Baru
command-palette-open-canvas = 🎨 Buka Whiteboard Canvas
command-palette-search = 🔍 Cari Semantik Vault
command-palette-chat = 💬 Tanya AI Assistant (Local RAG)
command-palette-switch-vault = 📂 Buka Folder Vault Lain
