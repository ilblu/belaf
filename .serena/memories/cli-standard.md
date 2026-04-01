# Rust CLI Best Practices 2025 - Quick Reference

## 🏗️ Projekt-Struktur

### Directory Layout (Pflicht)
```
my-cli/
├── src/
│   ├── main.rs              # 20-40 Zeilen max: Setup + Orchestration
│   ├── cli.rs               # clap Definitionen
│   ├── commands/            # Command handlers
│   ├── core/                # Business logic
│   ├── config.rs
│   ├── error.rs
│   └── utils.rs
├── tests/                   # Integration tests
├── benches/                 # Optional: Benchmarks
└── examples/                # Usage examples
```

### Code-Organisation
- ❌ **Keine `mod.rs` Dateien** (obsolet seit Rust 2018)
- ✅ **Command Pattern**: Jeder Subcommand in eigenem File
- ✅ **`pub(crate)` für interne APIs**
- ✅ **Explizite Pfade** statt Re-Exports

---

## 🔧 Core Dependencies

### Argument Parsing
**clap 4.5+ mit Derive API** (einzige Empfehlung 2025)
- Features: `["derive", "env", "wrap_help"]`
- `ValueHint` für Shell-Completions nutzen
- Environment Variables via `env = "VAR_NAME"`
- Doc-Comments werden zu Help-Text

### Error Handling
- **anyhow**: Application-Layer (main.rs, top-level)
- **thiserror**: Library-Code (core/, commands/)
- **Regel**: Niemals beide in derselben Funktion mischen
- `.context()` für aussagekräftige Fehlermeldungen

### Terminal UI
- **owo-colors**: Colors/Styling (beste Performance, zero-allocation)
  - Features: `["supports-colors"]`
  - Auto-Detection nutzen
- **indicatif**: Progress Bars
  - Multi-Progress für parallele Tasks
  - Nur jedes N-te Update bei schnellen Loops
- **dialoguer**: Interactive Input
  - Validation einbauen
  - Non-interactive Mode mit Flags vorsehen

### Configuration
- **confy**: Simple Use Cases (auto path resolution)
- **config**: Complex Use Cases (layered, multiple sources)
- **Regel**: Niemals Secrets in Config-Files
- Environment Variable Overrides immer ermöglichen

### Logging
- **tracing** (nicht env_logger): Strukturiert, Spans, Performance
- Features: `["env-filter"]` via tracing-subscriber
- `#[instrument]` für automatisches Span-Tracking
- Verbosity via clap: `-v`, `-vv`, `-vvv`

---

## ✅ Testing

### Unit Tests
- In derselben Datei wie Implementation
- `#[cfg(test)]` Module

### CLI Integration Tests
**Option 1: trycmd** (Snapshot Testing, empfohlen)
- Beste Lösung für viele Test Cases
- `.toml` Files für Test Cases
- Inline Tests in README.md möglich
- `TRYCMD=overwrite` für Snapshot-Updates

**Option 2: assert_cmd + assert_fs**
- Für komplexe Test Cases mit Setup
- Datei-System Assertions
- predicates für flexible Checks

### Property-Based Tests
- **proptest**: Für Input-Validierung testen

---

## 📦 Distribution

### Cross-Compilation
**cross** (empfohlen)
- Zero-setup via Docker
- Targets: `x86_64-unknown-linux-musl`, `x86_64-pc-windows-gnu`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin`, `aarch64-apple-darwin`

**Alternative: cargo-dist**
- Moderne Lösung mit installer generation
- Unterstützt shell, powershell, homebrew installers

### Binary Optimization (Pflicht)
```toml
[profile.release]
opt-level = "z"          # Size over speed
lto = true               # Link-time optimization
codegen-units = 1        # Better optimization
strip = true             # Strip symbols
panic = "abort"          # No unwinding
```

### Installation Methods
- cargo-binstall Support vorsehen
- GitHub Releases mit precompiled binaries
- Optional: Homebrew Formula, apt/rpm packages

---

## 🐚 Shell Completions (Pflicht)

### clap_complete
- **Runtime Generation** (empfohlen): `completions` Subcommand
- **Build-Time Generation** (alternative): build.rs
- **Shells**: Bash, Fish, Zsh, PowerShell, Elvish
- **ValueHint nutzen**: `FilePath`, `DirPath`, `Url`, `Username`, `Hostname`

---

## 🔒 Security

### Dependency Scanning
- **cargo-audit**: Regelmäßig ausführen
- In CI/CD Pipeline integrieren

### Best Practices
- ❌ Secrets niemals in Config-Files oder Code
- ✅ Input-Validierung für alle User-Inputs
- ✅ Path-Traversal Prevention (keine `..` Components)
- ✅ Command-Injection Prevention (nie `sh -c` mit user input)
- ✅ Sensitive Data aus Memory löschen (secrecy, zeroize)

---

## 🚀 Performance

### Binary Size
- Feature-Flags nutzen für optionale Dependencies
- Nur benötigte tokio features (`rt`, `macros` statt `full`)
- UPX Compression optional (50-70% Reduktion)

### Startup Time
- Lazy Initialization (OnceLock, lazy_static)
- Expensive Resources on-demand laden

### Memory
- `Cow<str>` statt String wo möglich
- `&[u8]` statt Vec<u8> für read-only
- String Interning für häufige Strings

---

## 📋 Go-Live Checklist

### Code Quality
- [ ] Tests laufen: `cargo test`
- [ ] Keine Warnings: `cargo clippy -- -D warnings`
- [ ] Formatierung: `cargo fmt -- --check`
- [ ] Docs vollständig: `cargo doc`

### CLI Quality
- [ ] Help-Text für alle Commands
- [ ] `--version` funktioniert
- [ ] Error-Messages hilfreich
- [ ] Exit-Codes korrekt (0 = success, 1+ = error)

### UX
- [ ] Colors mit Auto-Detection
- [ ] Progress-Bars bei langen Ops
- [ ] Confirmation bei destructive Ops
- [ ] `--color` Flag (always/auto/never)
- [ ] `-v` Verbosity Levels

### Distribution
- [ ] Cross-Compilation Setup
- [ ] Binary Size < 10 MB
- [ ] Shell Completions generiert
- [ ] CI/CD Pipeline (GitHub Actions)
- [ ] Release Automation

### Security
- [ ] `cargo audit` clean
- [ ] Keine Secrets im Code
- [ ] Input-Validierung implementiert

### Docs
- [ ] README mit Examples
- [ ] CHANGELOG.md
- [ ] LICENSE file

---

## ❌ Anti-Patterns

### Vermeiden
- **God Commands**: 50+ Arguments in einem Command
- **Silent Failures**: `let _ = ...` statt proper error handling
- **Blocking Prompts**: Ohne `--non-interactive` Flag
- **Random Exit Codes**: 0 für success, 1+ für errors
- **Large Binaries**: Unnötige Dependencies/Features
- **`unwrap()` in Production**: Immer `?` oder `.context()`
- **Secrets in Config**: Nur Environment Variables
- **Vendor Lock-in**: Platform-agnostische Lösungen bevorzugen

---

## 📚 Tech Stack 2025

### Must-Have
```toml
clap = { version = "4.5", features = ["derive", "env"] }
anyhow = "1.0"
thiserror = "1.0"
owo-colors = { version = "4.2", features = ["supports-colors"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

### Common Additions
```toml
indicatif = "0.17"           # Progress bars
dialoguer = "0.11"           # Interactive input
confy = "0.6"                # Config management
serde = { version = "1.0", features = ["derive"] }
clap_complete = "4.5"        # Shell completions
```

### Dev Dependencies
```toml
trycmd = "0.15"              # CLI testing
assert_cmd = "2.0"           # CLI testing
assert_fs = "1.1"            # Filesystem testing
proptest = "1.4"             # Property testing
criterion = "0.5"            # Benchmarking
```

---

## 🎯 Quick Decisions

| Entscheidung | Empfehlung 2025 |
|--------------|----------------|
| Argument Parsing | clap 4.5+ Derive API |
| Error Handling (App) | anyhow |
| Error Handling (Lib) | thiserror |
| Colors | owo-colors |
| Progress | indicatif |
| Interactive | dialoguer |
| Config (simple) | confy |
| Config (complex) | config |
| Logging | tracing + tracing-subscriber |
| Testing | trycmd oder assert_cmd |
| Cross-Compile | cross |
| Distribution | cargo-dist oder GitHub Actions |

---

## 📖 Essential Resources

- [Rust CLI Book](https://rust-cli.github.io/book/)
- [clap Docs](https://docs.rs/clap)
- [Rain's CLI Recommendations](https://rust-cli-recommendations.sunshowers.io/)

---

**Version**: 2025.1 (Compact)  
**Prinzip**: Weniger ist mehr - klare Standards, keine Experimente
