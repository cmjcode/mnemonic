---
id: f6a7b8c9-d0e1-2345-fabc-678901234567
title: Meeting Notes Q3
type: note
created: 2026-09-01T14:00:00+07:00
modified: 2026-09-18T09:30:00+07:00
pinned: false
color: coral
tags: [meeting, Q3, 2026, tim]
aliases: [Notulensi Q3]
archived: false
trashed: false
---

# 📋 Meeting Notes — Review Q3 2026

**Tanggal:** 1 September 2026, 14:00 WIB
**Peserta:** Andi, Siti, Budi, Dewi
**Moderator:** Andi Pratama
**Lokasi:** Kantor WeWork, Sudirman

---

## Agenda

1. Review pencapaian Q3
2. Demo produk beta
3. Pembahasan timeline pitch investor
4. Evaluasi keuangan
5. Action items

## 1. Review Pencapaian Q3

> Siti mempresentasikan progress teknis:
>
> > "MVP backend sudah stabil. Seluruh inference pipeline berjalan di bawah
> > 3 detik per query pada hardware M2. Yang tersisa adalah polish UI dan
> > edge case handling."
> >
> > — Siti Rahayu, CTO

### Metrik Kunci

| KPI                    | Target Q3 | Aktual Q3 | Status |
|------------------------|-----------|-----------|--------|
| Fitur selesai          | 12        | 14        | ✅ 117% |
| Bug kritis             | < 5       | 2         | ✅      |
| Test coverage          | 80%       | 83.5%     | ✅      |
| Performa inference     | < 5s      | 2.8s      | ✅      |
| User testing feedback  | > 4.0/5   | 4.3/5     | ✅      |
| Burn rate              | < 60jt/bl | 52jt/bl   | ✅      |

> [!important] Semua KPI tercapai!
> Ini pertama kalinya tim mencapai semua target dalam satu quarter.
> Detail proyek di [[Proyek Startup AI]].

## 2. Demo Produk Beta

Fitur yang didemonstrasikan:

### 2.1 Hybrid Search Engine

```rust
// Contoh kode dari demo: Hybrid Search API
pub fn search(query: &str, top_k: usize) -> Result<Vec<SearchHit>> {
    let keyword_hits = bm25_search(query, top_k * 2)?;
    let semantic_hits = vector_search(query, top_k * 2)?;

    Ok(reciprocal_rank_fusion(keyword_hits, semantic_hits, top_k))
}
```

### 2.2 Evaluasi Model

```python
# Script evaluasi yang digunakan Budi
import pandas as pd
import numpy as np

results = pd.read_csv("eval_results.csv")
mrr = np.mean(1.0 / results["rank"])
recall_5 = np.mean(results["found_in_top5"])
print(f"MRR: {mrr:.4f}")       # Output: MRR: 0.8234
print(f"Recall@5: {recall_5:.1%}")  # Output: Recall@5: 89.7%
```

### 2.3 Hasil Benchmark

Lihat data lengkap di [[Jurnal Riset NLP#3. Hasil Evaluasi]].

## 3. Timeline Pitch Investor

> [!caution] Deadline Ketat
> Pitch deck harus final **20 Oktober 2026** — 5 hari sebelum presentasi.

| Milestone              | Deadline     | PIC   | Status |
|------------------------|-------------|-------|--------|
| Draft pitch deck v1    | 1 Okt 2026  | Andi  | ⏳     |
| Review internal        | 10 Okt 2026 | All   | ⏳     |
| Finalisasi deck        | 20 Okt 2026 | Dewi  | ⏳     |
| Rehearsal presentasi   | 22 Okt 2026 | All   | ⏳     |
| **Pitch Day**          | **25 Okt**  | Andi  | 🎯     |

Referensi milestone lengkap: [[Proyek Startup AI#Milestone]]

## 4. Evaluasi Keuangan

> Budi menunjukkan analisis dari data penjualan:

Lihat data di: [[data-penjualan.csv]]

> [!note] Catatan Keuangan
> Burn rate Q3 lebih rendah 13% dari target berkat efisiensi
> cloud cost dan negosiasi ulang vendor.

## 5. Action Items

- [x] @Siti: Optimasi cold-start inference → selesai 5 Sep ✅
- [x] @Budi: Finalisasi benchmark report → selesai 8 Sep ✅
- [x] @Budi: Update [[Jurnal Riset NLP]] dengan hasil terbaru ✅
- [ ] @Dewi: Redesign landing page mockup → deadline 25 Sep
- [ ] @Andi: Draft pitch deck v1 → deadline 1 Okt
- [ ] @All: Review pitch deck bersama → 10 Okt
- [ ] @Siti: Update [[Arsitektur Microservice]] diagram → 15 Okt
- [ ] @Budi: Siapkan live demo untuk pitch → 20 Okt

---

*Meeting berikutnya: 15 Oktober 2026, 14:00 WIB*
*Notulensi Q4: [[Meeting Notes Q4]] (dijadwalkan Desember 2026)*

#meeting #Q3 #2026 #tim
