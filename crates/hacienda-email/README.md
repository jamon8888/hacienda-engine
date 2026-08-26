# hacienda-email: Email Processing Plugin

A hacienda plugin for email archiving, NER extraction, RAG indexing, and PII redaction using the [Pimalaya](https://github.com/pimalaya/pimalaya) email ecosystem.

## Features

- **Archive**: Fetch and store emails from IMAP, JMAP, Gmail, Microsoft Graph, or Maildir backends
- **NER Extraction**: Extract named entities (PERSON, ORG, EMAIL, PHONE, LOC, DATE, MONEY) using GLiNER via hacienda-core
- **RAG Indexing**: Chunk and index email content for retrieval-augmented generation via hacienda-rag
- **PII Redaction**: Redact sensitive data (emails, phones, SSNs, credit cards, IBANs) using hacienda-core's PII engine
- **Attachment Processing**: Extract text from attachments using xberg (97+ formats)
- **Full Pipeline**: Combine all operations in a single workflow

## Architecture

The plugin integrates at the `io-email` domain library layer of the Pimalaya stack:

```
Pimalaya Stack (what we integrate)
┌─────────────────────────────────────────────────────────────┐
│                    APPLICATION LAYER                         │
│  himalaya (CLI) ── himalaya-tui (TUI) ── neverest (sync)    │
├─────────────────────────────────────────────────────────────┤
│                    DOMAIN LAYER (io-email)                   │
│  Shared LCD types: Mailbox, Envelope, Address, Flag         │
│  Unified client: EmailClientStd (blocking dispatcher)       │
│  Backends: IMAP │ JMAP │ Gmail │ Graph │ Maildir │ m2dir │ SMTP │
├─────────────────────────────────────────────────────────────┤
│                 PROTOCOL/STORAGE LAYER                       │
│  io-imap │ io-jmap │ io-gmail │ io-msgraph │ io-maildir │  │
│  io-m2dir │ io-smtp │ io-managesieve                       │
├─────────────────────────────────────────────────────────────┤
│                    FOUNDATION LAYER                          │
│  pimalaya-stream (TLS, SASL, transport)                     │
│  pimalaya-cli (args, config, terminal)                      │
│  pimalaya-config (TOML, secrets)                            │
└─────────────────────────────────────────────────────────────┘
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
hacienda-email = { path = "../crates/hacienda-email" }
```

Enable features as needed:

```toml
hacienda-email = { path = "../crates/hacienda-email", features = ["imap", "maildir", "ner", "rag", "redact"] }
```

## Configuration

Create a `config.toml`:

```toml
[email]
backend = "imap"           # imap | maildir | jmap | gmail | msgraph
account = "work"

[email.imap]
server = "imaps://imap.gmail.com"
username = "user@gmail.com"
# password from HACIENDA_EMAIL_WORK_PASSWORD env var

[email.maildir]
path = "~/.local/share/himalaya/mail"

[email.ner]
model = "gliner-multi-v2.1"
confidence_threshold = 0.5
entity_types = ["PERSON", "ORG", "EMAIL", "PHONE", "LOC", "DATE", "MONEY"]
batch_size = 32

[email.rag]
chunk_size = 512
chunk_overlap = 50
embedding_model = "all-MiniLM-L6-v2"
index_name = "email-corpus"

[email.redaction]
rules = ["email", "phone", "ssn", "credit_card", "iban"]
replacement = "[REDACTED]"
preserve_format = true
```

Set secrets via environment variables:

```bash
export HACIENDA_EMAIL_WORK_PASSWORD="your-imap-password"
export HACIENDA_EMAIL_WORK_TOKEN="your-jmap-token"
export HACIENDA_EMAIL_WORK_CLIENT_SECRET="your-gmail-client-secret"
```

## Usage

### CLI Commands

```bash
# Archive emails
hacienda email archive --account work --since "30 days ago" --output ~/email-archive --include-attachments

# Extract named entities
hacienda email ner --input ~/email-archive --output ./entities.json --model gliner-multi-v2.1

# Index for RAG
hacienda email index --input ~/email-archive --index email-corpus --chunk-size 512

# Redact PII
hacienda email redact --input ~/email-archive --output ~/email-redacted --rules email,phone,ssn

# Full pipeline
hacienda email pipeline --account work --output ~/email-processed --steps archive,attachments,ner,rag,redact
```

### Rust API

```rust
use hacienda_email::{EmailConfig, EmailProcessor, EmailRequest, EmailOperation, EmailSource, EmailDestination};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = EmailConfig {
        backend: EmailBackend::Imap {
            server: "imaps://imap.gmail.com".to_string(),
            username: "user@gmail.com".to_string(),
        },
        account: "work".to_string(),
        ..Default::default()
    };

    let processor = EmailProcessor::new(config).await?;
    
    // Archive emails
    let request = EmailRequest {
        operation: EmailOperation::Archive {
            source: EmailSource::Account { name: "work".into(), mailbox: Some("INBOX".into()) },
            destination: EmailDestination::Maildir { path: "./email-archive".into() },
            since: Some("30 days ago".into()),
            before: None,
            mailbox: Some("INBOX".into()),
            include_attachments: true,
        },
        config_overrides: None,
    };
    
    let response = processor.process(request).await?;
    println!("Archived {} emails", response.processed_count);
    
    Ok(())
}
```

### HTTP API

When running `hacienda serve`, the following endpoints are available:

- `POST /v1/email/archive` - Archive emails
- `POST /v1/email/ner` - Extract named entities
- `POST /v1/email/index` - Index for RAG
- `POST /v1/email/redact` - Redact PII
- `POST /v1/email/pipeline` - Run full pipeline

### MCP Tools

When running `hacienda mcp serve`, the following tools are available:

- `email_archive` - Archive emails
- `email_ner` - Extract named entities
- `email_rag_index` - Index for RAG
- `email_redact` - Redact PII

## Features

| Feature | Description | Dependencies |
|---------|-------------|--------------|
| `imap` | IMAP backend | `io-imap`, `io-email/imap` |
| `jmap` | JMAP backend | `io-jmap`, `io-email/jmap` |
| `maildir` | Maildir backend | `io-maildir`, `io-email/maildir` |
| `m2dir` | m2dir backend | `io-m2dir`, `io-email/m2dir` |
| `smtp` | SMTP sending | `io-smtp`, `io-email/smtp` |
| `gmail` | Gmail REST API | `io-gmail` |
| `msgraph` | Microsoft Graph | `io-msgraph` |
| `ner` | NER extraction | `hacienda-core/ner-candle` |
| `rag` | RAG indexing | `hacienda-rag/chunking`, `hacienda-rag/embeddings` |
| `redact` | PII redaction | `xberg` |
| `rustls-ring` | Rustls with ring | `pimalaya-stream/rustls-ring` |
| `rustls-aws` | Rustls with AWS | `pimalaya-stream/rustls-aws` |
| `native-tls` | Native TLS | `pimalaya-stream/native-tls` |

Default features: `imap`, `maildir`, `rustls-ring`

## Backend Support Matrix

| Operation | IMAP | JMAP | Gmail | Graph | Maildir | m2dir | SMTP |
|-----------|:----:|:----:|:-----:|:-----:|:-------:|:-----:|:----:|
| list_mailboxes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| list_envelopes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| search_envelopes | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| get_message | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| add_message | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| copy_messages | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| move_messages | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| delete_messages | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| store_flags | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | |
| send_message | | ✓ | ✓ | ✓ | | | ✓ |

## Redaction Rules

| Rule | Pattern | Description |
|------|---------|-------------|
| `email` | `\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b` | Email addresses |
| `phone` | `\b(?:\+?1[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b` | Phone numbers |
| `ssn` | `\b\d{3}-\d{2}-\d{4}\b` | Social Security Numbers |
| `credit_card` | `\b\d{4}[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b` | Credit card numbers |
| `iban` | `\b[A-Z]{2}\d{2}[A-Z0-9]{4}\d{7}([A-Z0-9]?){0,16}\b` | IBAN numbers |
| `ip` | `\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b` | IP addresses |
| `custom` | User-defined regex | Custom patterns |

## Testing

Run tests with:

```bash
# Unit tests
cargo test -p hacienda-email

# Integration tests (requires mock IMAP)
cargo test -p hacienda-email --features mock

# All tests
cargo test -p hacienda-email --all-features
```

## License

Licensed under either of:
- Apache License, Version 2.0 (LICENSE-APACHE)
- MIT license (LICENSE-MIT)

at your option.

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines.

## Related Projects

- [himalaya](https://github.com/pimalaya/himalaya) - CLI to manage emails
- [io-email](https://github.com/pimalaya/io-email) - Email client library
- [hacienda-engine](https://github.com/jamon8888/hacienda-engine) - Document processing engine
- [xberg](https://github.com/xberg-io/xberg) - Document extraction library