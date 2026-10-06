---
id: d4e5f6a7-b8c9-0123-defa-456789012345
title: Arsitektur Microservice
type: note
created: 2026-08-15T03:00:00+00:00
modified: 2026-09-19T13:10:29.835682+00:00
pinned: false
color: teal
tags:
- arsitektur
- backend
- microservice
aliases:
- System Architecture
archived: false
trashed: false
---

# 🏗️ Arsitektur Microservice Platform AI ^sgeysr

Dokumen arsitektur teknis untuk [[Proyek Startup AI]]. ^0u1tfv

## Diagram Utama ^8hzvg7

Arsitektur keseluruhan sistem: ^5wdr68

```mermaid
%%{init: {'theme': 'dark'}}%%
flowchart TB
    subgraph Client["🖥️ Client Layer"]
        Web["Web App<br/>React"]
        Mobile["Mobile App<br/>Flutter"]
        CLI["CLI Tool<br/>Rust"]
    end

    subgraph Gateway["🔐 API Gateway"]
        Kong["Kong Gateway"]
        Auth["Auth Service<br/>JWT + OAuth2"]
    end

    subgraph Core["⚙️ Core Services"]
        UserSvc["User Service"]
        NoteSvc["Note Service"]
        SearchSvc["Search Service"]
        AISvc["AI Service<br/>Local Inference"]
    end

    subgraph Data["💾 Data Layer"]
        PG[(PostgreSQL)]
        Redis[(Redis Cache)]
        S3["Object Storage<br/>MinIO"]
        Vec[(Vector DB<br/>pgvector)]
    end

    Client --> Gateway
    Kong --> Auth
    Auth --> Core
    UserSvc --> PG
    NoteSvc --> PG
    NoteSvc --> S3
    SearchSvc --> Vec
    SearchSvc --> Redis
    AISvc --> Vec
    AISvc --> Redis
```
^7pvdcy

## Alur Autentikasi ^byy4d9

```mermaid
sequenceDiagram
    participant U as 👤 User
    participant C as 📱 Client
    participant G as 🔐 Gateway
    participant A as 🔑 Auth Service
    participant D as 💾 Database

    U->>C: Login (email + password)
    C->>G: POST /auth/login
    G->>A: Forward request
    A->>D: Verify credentials
    D-->>A: User found ✅
    A->>A: Generate JWT + Refresh Token
    A-->>G: {access_token, refresh_token}
    G-->>C: 200 OK + tokens
    C->>C: Store tokens securely
    C-->>U: Dashboard loaded 🎉

    Note over U,D: Token Refresh Flow
    C->>G: GET /api/notes (expired token)
    G-->>C: 401 Unauthorized
    C->>G: POST /auth/refresh
    G->>A: Validate refresh token
    A-->>G: New access_token
    G-->>C: 200 + new token
```
^15g3rf

## Entity Relationship — Data Model ^hinida

```mermaid
erDiagram
    USER ||--o{ NOTE : creates
    USER ||--o{ TAG : owns
    NOTE ||--o{ CHUNK : "split into"
    NOTE }o--o{ TAG : "labeled with"
    NOTE ||--o{ ATTACHMENT : contains
    CHUNK ||--|| EMBEDDING : "vectorized as"
    NOTE ||--o{ CANVAS_NODE : "visualized in"

    USER {
        uuid id PK
        string email
        string name
        string locale
        datetime created_at
    }
    NOTE {
        uuid id PK
        uuid user_id FK
        string title
        text body
        string color
        boolean pinned
        boolean archived
        datetime created
        datetime modified
    }
    TAG {
        uuid id PK
        string name
        string color
    }
    CHUNK {
        uuid id PK
        uuid note_id FK
        int char_offset
        int page_num
        text content
    }
    EMBEDDING {
        uuid chunk_id FK
        vector384 vec
        string content_hash
    }
    ATTACHMENT {
        uuid id PK
        uuid note_id FK
        string filename
        string mime_type
        bigint size_bytes
    }
```
^tz73n4

## Status Deployment Pipeline ^5dzmey

```mermaid
stateDiagram-v2
    [*] --> Development
    Development --> CodeReview : PR Created
    CodeReview --> Development : Changes Requested
    CodeReview --> Staging : Approved ✅
    Staging --> QATesting : Auto Deploy
    QATesting --> Staging : Bug Found 🐛
    QATesting --> Production : All Tests Pass ✅
    Production --> Rollback : Critical Issue 🚨
    Rollback --> Staging : Hotfix Applied
    Production --> [*] : Stable Release 🎉
```
^kz2vxp

## Distribusi Trafik ^kiav5p

```mermaid
pie title Distribusi Request per Service (September 2026)
    "Note Service" : 42
    "Search Service" : 28
    "AI Assistant" : 18
    "Auth Service" : 8
    "File Upload" : 4
```
^2u7hun

## Class Diagram — Core Domain ^e32ehx

```mermaid
classDiagram
    class Note {
        +UUID id
        +String title
        +String body
        +NoteType type
        +Vec~Tag~ tags
        +save()
        +delete()
        +to_chunks()
    }
    class VaultService {
        +list_notes()
        +read_note()
        +search()
        +ask()
        +reindex()
    }
    class SearchEngine {
        +keyword_search()
        +vector_search()
        +hybrid_rrf()
    }
    class EmbeddingEngine {
        +embed_text()
        +embed_batch()
        +cosine_similarity()
    }
    class LLMEngine {
        +generate()
        +stream_tokens()
        +build_prompt()
    }

    VaultService --> Note
    VaultService --> SearchEngine
    VaultService --> LLMEngine
    SearchEngine --> EmbeddingEngine
    LLMEngine --> SearchEngine : uses retrieval
```
^rq7o4r

> [!note] Performa Rendering Mermaid ^q5927v
> Semua diagram di atas dirender **native dalam Rust** tanpa JavaScript/WebView. ^vdlgwc
> Flowchart 550 node dirender dalam ~20ms (release build). ^3cv8nf
> Diagram tipikal seperti di atas < 1ms. ^zpd9lp

Lihat juga: [[Database Schema]] untuk detail skema database. ^sfs2vz

#arsitektur #backend #microservice #diagram ^k8z3u0
