# Rust Microservice Standards 2025

## Architektur (Pflicht)
- Hexagonale Workspace-Struktur: `api` | `core` | `infrastructure` | `shared`
- **Unified-Protocol-Pattern (Hyperswitch)**: Ein Service, alle Protokolle (REST/GraphQL/gRPC)
  - **138K req/s** (vs. 142K native gRPC)
  - 83% weniger Code-Duplikation
  - Keine separaten API Gateways
- Repository-Pattern mit Traits (strikte Entkopplung)

## Domain-Organisation (Pflicht)

### Bounded Contexts & Domain Separation
- **Jede Domain = Ein Bounded Context**: Vollständige Kapselung aller Domain-Aspekte
- **Domain-Owned Operations**: Alle Geschäftslogik einer Domain bleibt in ihrer Domain
- **Keine technischen Layer-Splits**: Domains nicht nach Funktionen (z.B. "Moderation") aufteilen
- **Prinzip**: "Ein Microservice handhabt alle Aspekte seiner Problem-Domain"

### Domain-Struktur innerhalb von `core/`
```
core/
├── {domain_name}/
│   ├── {entity}_service.rs      # Business Logic
│   ├── {entity}_repository.rs   # Trait Definition (KEINE Impl!)
│   ├── models.rs                # Domain Models
│   └── errors.rs                # Domain-spezifische Errors
```

### Service-Verantwortlichkeiten
- **Single Responsibility**: Ein Service = Eine klar definierte Geschäftslogik
- **Granularität**: Services nach Geschäftsprozessen trennen, nicht nach CRUD
- **Beispiele**:
  - `user_service.rs` → Profil, Authentifizierung, Preferences
  - `account_lifecycle_service.rs` → Ban, Delete, Suspend, Restore
  - `moderation_service.rs` → Restrictions, Mute, Permissions

### Verbotene Anti-Patterns
- ❌ God-Services mit zu vielen Verantwortlichkeiten
- ❌ Cross-Domain Services (z.B. zentraler "Moderation Service" für alle Domains)
- ❌ Technische Layer als Domains (z.B. `core/validation/`, `core/authorization/`)
- ❌ Domain-Logik in `shared/` (außer Cross-Cutting Concerns)

## Cross-Cutting Concerns (Pflicht)

### Definition
Cross-Cutting Concerns sind technische Funktionalitäten, die von **mehreren Domains** genutzt werden, aber **keine eigene Business-Domain** darstellen.

### Platzierung in `shared/`
**Gehören nach `shared/`:**
- Toxicity Detection & Content Analysis
- Audit Logging & Moderation History
- Rate Limiting & Throttling
- Observability Utilities (Custom Metrics)
- Common Validation Rules
- Encryption/Hashing Utilities

**Gehören NICHT nach `shared/`:**
- Domain Models
- Business Rules
- Aggregation Logic
- Domain-spezifische Services

### Struktur von Cross-Cutting Concerns
```
shared/
├── {concern_name}/
│   ├── traits.rs           # Trait-Definitionen
│   ├── {concern}_service.rs # Default-Implementierung
│   └── models.rs           # Shared Models
```

### Nutzung via Dependency Injection
- Core-Services erhalten Cross-Cutting Concerns via Constructor Injection
- Nur gegen Traits programmieren, nie gegen konkrete Impls
- Infrastructure-Layer liefert konkrete Implementierungen

### Beispiel-Zuordnung
| Concern | Platzierung | Grund |
|---------|-------------|-------|
| User Ban Logic | `core/roster/account_lifecycle_service.rs` | Domain-spezifische Business-Logik |
| Content Takedown | `core/moment/content_moderation_service.rs` | Domain-spezifische Business-Logik |
| Toxicity Detection | `shared/toxicity/toxicity_service.rs` | Von roster, moment, chat, crew genutzt |
| Audit Logging | `shared/audit/audit_service.rs` | Alle Domains loggen Moderation-Events |

## Dependency Management (Pflicht)

### Grundprinzip: Infrastructure-Isolation
**"KEINE external deps"** in `core/` und `shared/` bedeutet:
- ❌ Keine **Infrastructure-Dependencies** (konkrete DB-Clients, Cache-Clients, Message-Queues)
- ✅ **Utility-Crates sind erlaubt** (Observability, Async-Tools, Serialization)

### Erlaubte Dependencies nach Crate

| Dependency-Typ | `core/` | `shared/` | `infrastructure/` | `api/` |
|----------------|---------|-----------|-------------------|--------|
| **Observability** | | | | |
| tracing, opentelemetry | ✅ | ✅ | ✅ | ✅ |
| **Async Runtime** | | | | |
| tokio, async-trait | ✅ | ✅ | ✅ | ✅ |
| **Serialization** | | | | |
| serde, serde_json | ✅ | ✅ | ✅ | ✅ |
| **Common Types** | | | | |
| uuid, chrono | ✅ | ✅ | ✅ | ✅ |
| **Error Handling** | | | | |
| thiserror, anyhow | ✅ | ✅ | ✅ | ✅ |
| **Database Clients** | | | | |
| tokio-postgres, scylla | ❌ | ❌ | ✅ | ❌ |
| **Cache Clients** | | | | |
| redis, moka (in-mem OK) | ❌ | ❌ | ✅ | ❌ |
| **Message Queues** | | | | |
| async-nats, lapin | ❌ | ❌ | ✅ | ❌ |
| **Web Framework** | | | | |
| axum, tonic | ❌ | ❌ | ❌ | ✅ |
| **HTTP Clients** | | | | |
| reqwest, hyper | ❌ | ⚠️ | ✅ | ✅ |

**⚠️ HTTP Clients in `shared/`:** Nur für externe API-Integrationen (z.B. Toxicity-APIs), als optionales Feature-Flag

### Workspace-Dependencies (Best Practice)

**Root `Cargo.toml`:**
```toml
[workspace.dependencies]
# Utilities (überall verfügbar)
tracing = "0.4"
opentelemetry = "0.30"
tokio = { version = "1.40", features = ["full"] }
async-trait = "0.1"
uuid = { version = "1.0", features = ["v4", "serde"] }
serde = { version = "1.0", features = ["derive"] }
thiserror = "2.0"

# Infrastructure (NUR in infrastructure/)
tokio-postgres = "0.7"
deadpool-postgres = "0.14"
scylla = "1.2"
async-nats = "0.42"

# API Layer (NUR in api/)
axum = "0.8.0"
tonic = "0.12.0"
```

**Pro Crate:**
```toml
# shared/Cargo.toml
[dependencies]
tracing = { workspace = true }
opentelemetry = { workspace = true }
async-trait = { workspace = true }
tokio = { workspace = true }
```

### Dependency-Regeln Zusammenfassung

**`core/` und `shared/` dürfen:**
- ✅ Observability (tracing, opentelemetry, metrics)
- ✅ Async Abstractions (tokio, async-trait, futures)
- ✅ Serialization (serde)
- ✅ Common Types (uuid, chrono, url)
- ✅ Error Handling (thiserror, anyhow)
- ✅ Testing (mockall, proptest)
- ✅ In-Memory Caching (moka, lru)

**`core/` und `shared/` dürfen NICHT:**
- ❌ Database Clients (tokio-postgres, scylla)
- ❌ External Cache Clients (redis, memcached)
- ❌ Message Queue Clients (async-nats, lapin, rdkafka)
- ❌ Web Frameworks (axum, actix-web, warp)
- ❌ HTTP Clients (außer in shared/ mit Feature-Flag für APIs)

### Rationale
- **Testbarkeit**: Core & shared ohne externe Services testbar
- **Portabilität**: Infrastructure austauschbar (Postgres → MySQL)
- **Compilation Speed**: Weniger heavy dependencies in core
- **Hexagonale Architektur**: Strikte Trennung Port (core) vs. Adapter (infrastructure)

## Modul-Organisation (Best Practice 2025)
- **KEINE `mod.rs` Dateien** (seit Rust 2018 obsolet)
- **KEINE Re-Exports mit `pub use`** in internen Modulen
- **Explizite Pfade bevorzugen**: `crate::module::Type` statt Re-Export
- **`pub(crate)` für interne Sichtbarkeit** statt `pub` + Re-Export
- **Direktes Importieren**: `use crate::models::user::User` statt `pub use`
- **Nur für externe Crate-APIs**: Re-Exports sind akzeptabel für die öffentliche API eines Crates, aber nicht für interne Organisation

## Geschäftslogik vs. Datenbank (Pflicht)

**Grundprinzip**: Alle Domain-Regeln, Berechnungen und Workflows **ausschließlich im Rust-Code**. Datenbank = Persistenz + Integrität.

### Rust-Backend: Domain-Logik
- **Alle Geschäftsregeln in Rust**:
  - Validierungen, Berechnungen, Transformationen
  - State-Maschinen und Workflows
  - Aggregationen und Filterlogik
  - Transaktionen im Service-Code
- **Testing-Pflicht**:
  - Unit-Tests (`#[cfg(test)]` in Quelldatei)
  - Integrationstests (`tests/` Ordner)
  - Property-Based Tests
- **Kurzlebiger State**:
  - Session-Data, Rate-Limits, Locks → **Cache-Layer**
  - NICHT in DB-Triggern oder Stored Procedures
- **Background-Jobs**: Asynchrone Task-Verarbeitung im Code

### Datenbank: Persistenz-Layer
- **Erlaubt**:
  - Tabellen, Partitionierung, Indexe
  - Constraints: `NOT NULL`, `CHECK`, `FOREIGN KEY`, `UNIQUE`
  - Einfache Defaults: `DEFAULT CURRENT_TIMESTAMP`
  - Seed-Daten in Migrationen
- **VERBOTEN**:
  - ❌ Stored Procedures
  - ❌ Trigger für Geschäftslogik
  - ❌ Komplexe Berechnungen in SQL
  - ❌ Proprietäre Extensions für Domain-Logik

### Architektur-Prinzipien
- **Repository-Pattern (Traits)** → Datenbank-agnostisch
- **Migrationen versioniert** im Code-Repository
- **ANSI-SQL bevorzugen** für Portabilität
- **Keine Vendor-Lock-Ins**
- **Clear Separation**: Rust = Algorithmen | DB = Datenhaltung

### Vorteile
- ✅ **100% Testbarkeit** (Unit-Tests ohne DB)
- ✅ **Typ-Safety** zur Compile-Zeit
- ✅ **Performance** durch Zero-Copy & Async
- ✅ **Portabilität** bei DB-Wechsel
- ✅ **Wartbarkeit** durch klare Verantwortlichkeiten

## Dependency Injection Pattern (Pflicht)

### Trait-basierte Abstraktion
- Alle externen Abhängigkeiten als Traits definieren
- Services akzeptieren Traits via Generics oder Arc<dyn Trait>
- Konkrete Implementierungen in `infrastructure/`

### Constructor Injection
- Alle Dependencies via Constructor übergeben
- Keine globalen Singletons oder Lazy Statics für Business-Logic
- Testing: Mock-Implementierungen der Traits

### Beispiel-Pattern (ohne Code)
- Service definiert: Welche Traits werden benötigt
- Constructor nimmt: Arc-wrapped Trait Objects
- Infrastructure Layer: Erstellt konkrete Implementierungen
- Startup: Wires alles zusammen (Dependency Graph)

---

## Startup & `main.rs` Strukturierung (Pflicht)

**Grundprinzip**: `main.rs` bleibt minimal – nur Orchestrierung. Detaillogik in separate Module auslagern.

### `main.rs`: Einstiegspunkt
- **NUR folgende Aufgaben**:
  - Logging/Tracing initialisieren
  - Konfiguration laden
  - Globale Ressourcen erstellen
  - `startup()` oder `run()` aufrufen
  - Top-Level Fehlerbehandlung
- **Keine Business-Logik**, Setup-Details oder Komponenten-Konfiguration
- **Maximal 30-50 Zeilen** Code

### Module-Struktur (Pflicht)
```
src/
├── main.rs           # Einstiegspunkt (minimal)
├── startup.rs        # Komponenten-Initialisierung & DI-Wiring
├── config/           # Konfiguration laden
├── logging/          # Observability-Setup
└── ...               # Domain-Module
```

### Startup-Modul: Komponenten-Init
- **Kapselung aller Setup-Logik**:
  - Ressourcen-Pools (DB, Cache, Queues)
  - Middleware-Registrierung
  - Service-Komponenten mit DI
  - Externe Verbindungen
- **Fehlerbehandlung lokal**, nicht in `main.rs`
- **Testbar ohne Runtime**: Unit-Tests für jede `init_*`-Funktion
- **Dependency Graph Wiring**: Alle Trait-Implementierungen zusammenführen

### Feature-Steuerung
- **ENV-Variablen** für optionale Features
- **Conditional Compilation**: `#[cfg(feature = "...")]`
- **Configuration-Driven**: Verhalten via Config steuern

### Vorteile
- ✅ **Lesbarkeit**: Ablauf auf einen Blick in `main.rs`
- ✅ **Testbarkeit**: Komponenten isoliert testbar
- ✅ **Wartbarkeit**: Neue Features ohne `main.rs` Änderung
- ✅ **Kapselung**: Fehler und Abhängigkeiten bleiben lokal
- ✅ **Erweiterbarkeit**: Module hinzufügen, nicht Code aufblähen

---

## Tech Stack

### Runtime
- axum 0.8.0 + tokio 1.40

### APIs
- async-graphql 7.0.15, tonic 0.12.0, utoipa 5.0

### Datenbanken
- tokio-postgres 0.7 + deadpool-postgres 0.14 + postgres-types 0.2
  - **30-50% schneller als SQLx**
  - **Query Pipelining** → 20% Latency-Reduktion
  - **Prepared Statements** Pflicht
  - Pool: max. 20 Connections
- scylla 1.2 (Prepared Statements Pflicht)

### Messaging
- async-nats 0.42 (**sync Version deprecated!**)

### Caching
- moka 0.12 (85% hit rate, LFU-Admission)

### Security & Limits
- jsonwebtoken 9.2 (JWT/HS256)
- tower-governor 0.4 (IP + User Rate Limiting)

### Observability
- opentelemetry 0.30 + tracing 0.4
- OTLP → SigNoz
- **5% Sampling** (nur 5-10% CPU Overhead)
- Batch: 512 spans, 2 concurrent exports, 500ms delay

## Performance-Optimierungen
- **Zero-Copy Patterns** (BytesMut) → sub-ms Latenz
- Query Pipelining (20% Latency ↓)
- Connection Pooling (deadpool)

## Deployment
- Docker Multi-Stage (**cargo-chef** Pflicht)
- Distroless Runtime (non-root, read-only FS)
- K8s: Liveness + Readiness + Startup Probes
- HPA + Resource Limits
- **mTLS: Linkerd** (40-400% weniger Latency vs. Istio)

## Testing
- Unit Tests (in derselben Quelldatei):
  - Verwende `#[cfg(test)]` mit einem `mod tests`-Block
  - Kompilation nur mit `cargo test`
- Integrationstests:
  - Im Root-Ordner `tests/`
  - Jede Datei ist ein eigener Test-Crate
  - Keine `cfg`-Annotation erforderlich
- Doc-Tests:
  - In `///`-Kommentaren unter "Examples"
  - Automatisch mit `cargo test`
- Benchmarks:
  - Im Ordner `benches/` mit Criterion
  - Ausführung via `cargo bench`
- Property-Based Tests: `proptest` 1.4
- Contract Tests: `pact_consumer` 1.3
- Load Tests: `drill` 0.8

## CI/CD Pflicht
- cargo-audit
- Dependency Caching
- Automated Security Scans
- Multi-Stage Deployments + Canary

---

## Go-Live Checklist
- [ ] Workspace-Struktur implementiert (`api | core | infrastructure | shared`)
- [ ] Domains korrekt als Bounded Contexts strukturiert
- [ ] Cross-Cutting Concerns in `shared/` platziert
- [ ] Services nach Single Responsibility aufgeteilt
- [ ] Dependency Injection via Traits implementiert
- [ ] Dependency Management: Nur erlaubte Crates in core/shared
- [ ] Workspace-Dependencies konfiguriert
- [ ] Unified-Protocol (Hyperswitch) aktiv
- [ ] Repository-Traits vorhanden (keine DB-Impls in `core/`)
- [ ] Alle Crates + Versionen korrekt
- [ ] tokio-postgres mit Query Pipelining
- [ ] Prepared Statements für alle DBs
- [ ] async-nats (NICHT sync!)
- [ ] JWT + Rate Limiting in allen Endpoints
- [ ] Observability: 5% Sampling, Batch-Config
- [ ] mTLS via Linkerd aktiv
- [ ] cargo-chef Build-Optimierung
- [ ] Security-Scans in Pipeline
- [ ] Keine `mod.rs` Dateien
- [ ] Startup-Logik in separatem Modul

---
