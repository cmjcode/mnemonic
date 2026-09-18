---
id: e5f6a7b8-c9d0-1234-efab-567890123456
title: Jurnal Riset NLP
type: note
created: 2026-07-10T09:00:00+07:00
modified: 2026-09-18T11:00:00+07:00
pinned: false
color: green
tags: [riset, NLP, AI, paper-review]
aliases: [NLP Research, Riset AI]
archived: false
trashed: false
---

# 📚 Jurnal Riset NLP — Survey Model Bahasa Lokal

## Abstrak

Dokumen ini merangkum temuan riset kami mengenai penggunaan model bahasa
berukuran kecil (1.5B–3B parameter) yang dijalankan secara lokal untuk
aplikasi RAG (*Retrieval-Augmented Generation*) berbahasa Indonesia. ^abstrak

> [!tip] Coba Tanya AI
> Buka sidebar chat dan tanyakan:
> - "Apa kelebihan Qwen 2.5 dibanding Llama 3?"
> - "Berapa konsumsi RAM untuk model 3B quantized?"
> - "Jelaskan perbedaan BM25 dan dense retrieval"
>
> AI akan menjawab **hanya berdasarkan catatan ini** dengan kutipan sumber!

## 1. Perbandingan Model

### 1.1 Qwen 2.5 1.5B Instruct (Q4_K_M)

**Kelebihan:**
- Performa bahasa Indonesia sangat baik — di-training dengan dataset
  multilingual termasuk korpus Bahasa Indonesia yang besar ^qwen-indo
- Ukuran quantized hanya ~1.1 GB RAM
- Kecepatan inferensi: ~15–25 token/detik pada CPU modern (AVX2)
- Mendukung konteks 4096 token (cukup untuk RAG pipeline)
- Licensing permissive (Apache 2.0)

**Kekurangan:**
- Kurang kuat untuk task coding kompleks vs model 7B+
- Hallucination rate lebih tinggi tanpa grounding yang ketat

**Benchmark internal:**

| Task                    | Akurasi | Latency (avg) | Catatan                    |
|-------------------------|---------|---------------|----------------------------|
| QA Bahasa Indonesia     | 82.3%   | 2.1s          | 150 pasang Q&A             |
| Summarization           | 78.5%   | 4.3s          | Dokumen 500-2000 kata      |
| Code Generation (Rust)  | 71.2%   | 3.8s          | Fungsi sederhana           |
| Translation ID→EN       | 85.1%   | 1.8s          | Kalimat umum               |
| Classification          | 89.4%   | 0.9s          | Kategori catatan           |

### 1.2 Perbandingan dengan Model Lain

| Model                    | Size (Q4) | RAM    | ID Quality | Speed (tok/s) |
|--------------------------|-----------|--------|------------|---------------|
| **Qwen 2.5 1.5B**       | 1.1 GB    | ~2 GB  | ⭐⭐⭐⭐⭐     | 15-25         |
| Qwen 2.5 3B             | 2.1 GB    | ~4 GB  | ⭐⭐⭐⭐⭐     | 8-15          |
| Llama 3.2 1B             | 0.7 GB    | ~1.5 GB| ⭐⭐⭐       | 20-30         |
| Llama 3.2 3B             | 2.0 GB    | ~4 GB  | ⭐⭐⭐       | 10-18         |
| Phi-3.5 Mini 3.8B        | 2.3 GB    | ~4.5 GB| ⭐⭐⭐⭐     | 8-14          |
| Gemma 2 2B               | 1.5 GB    | ~3 GB  | ⭐⭐⭐⭐     | 12-20         |

> [!important] Kesimpulan Pemilihan Model
> **Qwen 2.5** dipilih karena kombinasi terbaik antara:
> ukuran kompak, kecepatan tinggi, dan **kualitas bahasa Indonesia yang unggul**.

### 1.3 Dense Retrieval vs Keyword Search

Kami menggunakan **hybrid retrieval** yang menggabungkan: ^hybrid-retrieval

1. **BM25 (keyword)** — cepat, exact match, bagus untuk istilah teknis spesifik
2. **Dense Vector (fastembed)** — memahami makna semantik lintas bahasa

Skor akhir dihitung via **Reciprocal Rank Fusion (RRF)**:

```
RRF_score = Σ (1 / (k + rank_i))
```

dimana `k = 60` dan `rank_i` adalah posisi dokumen pada masing-masing retriever.

### 1.4 Chunking Strategy

Strategi chunking kami: ^chunking

- **Window size:** 300 token per chunk
- **Overlap:** 50 token (mencegah kehilangan konteks di batas chunk)
- **Metadata per chunk:** `doc_id`, `file_path`, `page_num`, `char_offset`

> [!warning] Catatan Penting
> Model embedding `multilingual-e5-small` (~120 MB) akan di-download otomatis
> saat pertama kali menjalankan pencarian semantik. Pastikan koneksi internet
> tersedia untuk download awal ini. Setelah itu, sepenuhnya offline.

## 2. Pipeline RAG di Mnemonic

```
Pertanyaan User
      ↓
┌─────────────────┐
│ Hybrid Retrieval │ ← BM25 + Dense Vector (Top-K=5)
└────────┬────────┘
         ↓
┌─────────────────┐
│ RRF Rank Fusion │ ← Gabung & urutkan ulang
└────────┬────────┘
         ↓
┌─────────────────┐
│ Prompt Template │ ← Konteks + Pertanyaan + Sistem prompt
└────────┬────────┘
         ↓
┌─────────────────┐
│ Qwen 2.5 (local)│ ← Streaming token-by-token
└────────┬────────┘
         ↓
   Jawaban + Kutipan Sumber [📎 file.md Baris 42]
```

### 2.1 Prompt Template RAG

```text
<|im_start|>system
Anda adalah asisten cerdas. Jawablah pertanyaan pengguna HANYA
berdasarkan konteks dokumen berikut. Jika informasi tidak ada
di dokumen, katakan tidak tahu.

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

## 3. Hasil Evaluasi

Pada 150 pasang pertanyaan-jawaban dari catatan Bahasa Indonesia: ^evaluasi

| Metrik          | Keyword Only | Semantic Only | Hybrid (RRF) |
|-----------------|-------------|---------------|---------------|
| Recall@5        | 62.3%       | 78.9%         | **89.7%**     |
| Recall@10       | 71.8%       | 85.2%         | **93.4%**     |
| MRR             | 0.51        | 0.68          | **0.82**      |
| NDCG@5          | 0.58        | 0.72          | **0.85**      |
| Avg Latency     | 12ms        | 85ms          | **92ms**      |

> Hybrid retrieval meningkatkan recall sebesar **44%** dibandingkan keyword-only
> dengan overhead latency yang minimal (+7ms di atas semantic-only).

## 4. Referensi

1. Lewis et al., "Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks", NeurIPS 2020
2. Karpukhin et al., "Dense Passage Retrieval for Open-Domain QA", EMNLP 2020
3. Qwen Team, "Qwen2.5 Technical Report", 2024
4. Robertson & Zaragoza, "The Probabilistic Relevance Framework: BM25 and Beyond", 2009

Terkait: [[Machine Learning 101]], [[Proyek Startup AI]], [[Algoritma Graph]]

#riset #NLP #AI #paper-review #benchmark
