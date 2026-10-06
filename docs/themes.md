# Membuat tema baca (plugin warna)

Tema baca menentukan warna isi catatan di MNEMONIC — heading, link, tag,
kode, quote, tabel, callout — **di layar, saat dicetak, dan saat diekspor
ke PDF/HTML** (spec §3.2.5). Satu tema adalah satu file TOML. Isinya hanya
warna, tanpa kode yang dijalankan, jadi aman dibagikan.

## Di mana file tema diletakkan

| Folder | Berlaku untuk |
| --- | --- |
| `<config_dir>/mnemonic/themes/` (macOS: `~/Library/Application Support/mnemonic/themes/`, Linux: `~/.config/mnemonic/themes/`, Windows: `%APPDATA%\mnemonic\themes\`) | Semua vault di komputer ini |
| `<vault>/.mnemonic/themes/` | Hanya vault itu, dan ikut terbawa kalau vault disalin atau disinkronkan. Menang kalau id-nya sama. |

Nama file tanpa `.toml` adalah **id** tema (`kopi.toml` → `kopi`). Di app,
buka ikon palet 🎨 di atas catatan → **Buka folder plugin tema**. Setelah
mengubah file, pilih **Muat ulang tema**. Tema juga bisa dipakai hanya
untuk satu catatan lewat frontmatter:

```yaml
---
title: Resep Kue
theme: kopi
---
```

## Format

```toml
name = "Kopi"                 # nama yang tampil di menu
author = "Nama Anda"          # opsional
description = "Cokelat hangat"  # opsional
base = "sunset"               # opsional: warna yang tidak diisi diambil dari tema ini (default: mnemonic)

[light]                       # layar, mode terang
background = "#fbf6ef"
h1 = "#6f4e37"
link = "#a0522d"

[dark]                        # layar, mode gelap
h1 = "#d7b899"

[print]                       # opsional: cetak & ekspor PDF/HTML (default: sama dengan [light])
background = "#ffffff"
```

Warna ditulis `#rgb`, `#rrggbb`, atau `#rrggbbaa`. Key yang salah ketik
atau warna yang tidak valid tidak membuat app gagal. Nilai itu dilewati,
lalu muncul peringatan (lihat juga `mnemonic-cli themes list`).

## Semua key warna

| Key | Dipakai untuk |
| --- | --- |
| `background` | Latar halaman (juga latar kertas saat dicetak) |
| `text` | Teks biasa |
| `muted` | Teks sekunder: quote, checklist selesai |
| `strong` | **Bold**, bullet dan nomor list |
| `link` | Link dan `[[wikilink]]` |
| `tag` | `#tag` |
| `h1` … `h6` | Heading per level (juga judul catatan untuk `h1`) |
| `code_text`, `code_bg` | `kode` inline dan blok kode |
| `quote_bar` | Garis kiri blockquote |
| `rule` | Garis `---` |
| `table_border`, `table_header_bg` | Tabel |
| `highlight_bg` | `==highlight==` |
| `checkbox` | Checkbox yang dicentang |
| `callout_note`, `callout_tip`, `callout_important`, `callout_warning`, `callout_caution`, `callout_quote` | Warna aksen callout `> [!type]`. Alias Obsidian ikut dipetakan: `info`/`todo` → note, `success`/`hint` → tip, `example`/`summary` → important, `question` → warning, `danger`/`bug`/`error` → caution, `cite` → quote. |

## Contoh lengkap

Tema bawaan ada di folder [`themes/`](../themes) repo ini (`pelangi.toml`,
`ocean.toml`, `sunset.toml`, `forest.toml`, `print-classic.toml`). Salin
salah satunya dengan nama baru, lalu ubah warnanya.

## Tips cetak

- Warna latar dan aksen ikut tercetak karena halaman memakai
  `print-color-adjust: exact`. Kalau mau hemat tinta, isi `[print]` dengan
  `background = "#ffffff"` dan warna teks yang gelap.
- Ekspor PDF memakai Chrome, Edge, Chromium, atau Brave dalam mode headless.
  Browser lain bisa dipilih lewat variabel `MNEMONIC_BROWSER=/path/ke/browser`.
  Tanpa browser, gunakan **Cetak…** lalu *Simpan sebagai PDF*.
