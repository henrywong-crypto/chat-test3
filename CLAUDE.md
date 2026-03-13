# CLAUDE.md

## Crates

- Use `anyhow` for error handling.
- Use `clap` for CLI argument parsing.

### `common` crate

Put utilities in `common` when they are used by two or more crates and carry no domain-specific logic. Import from `common` directly — do not re-export through a domain crate.

```rust
// Good — shared utility lives in common, each crate imports it directly
// common/src/lib.rs
pub fn parse_tag_number(raw: &str) -> Option<TagNumber> { ... }

// feeding/src/lib.rs
use common::parse_tag_number;

// habitat/src/lib.rs
use common::parse_tag_number;

// Bad — utility defined in one domain crate and imported by another
// feeding/src/lib.rs
pub fn parse_tag_number(raw: &str) -> Option<TagNumber> { ... }  // belongs in common

// habitat/src/lib.rs
use feeding::parse_tag_number;  // habitat should not depend on feeding
```

## Keyword Conflicts

When a field name conflicts with a Rust keyword, use a trailing underscore (`type_`), not a raw identifier (`r#type`) or a prefixed rename (`entry_type`, `block_type`):

```rust
// Good
#[serde(rename = "type")]
pub type_: String,

// Bad
pub r#type: String,  // raw identifier

#[serde(rename = "type")]
pub animal_type: String,  // prefixed rename

#[serde(rename = "type")]
pub cage_type: String,  // prefixed rename
```

## Error Handling

When an operation returns `Result`, propagate the error with `?` and add context with `.context("...")`. Never swallow errors by converting them to a default value.

```rust
// Good — fail with context
let cage_dir = path.parent().context("cage path has no parent")?;
let weight = metadata.weight.context("missing animal weight")?;
let tag = HeaderValue::from_str(&value).context("invalid cage tag header")?;

// Bad — silently substitute a default
let cage_dir = path.parent().unwrap_or(".");
let weight = metadata.weight.unwrap_or(0);
let tag = HeaderValue::from_str(&value).unwrap_or(HeaderValue::from_static("fallback"));
```

Use `Option` only for values that are genuinely absent as part of normal logic (e.g. "animal has no cage", "search found no match"). Use `Result` for anything that can fail due to I/O, missing data, or invalid input.

## Channel Sends

Always wrap `mpsc::Sender::send` with `tokio::time::timeout`. A send with no timeout will block forever if the receiver is alive but not consuming — which can happen on an unstable network where the TCP connection appears open but the client is stalled. Use a three-arm match to distinguish the two failure modes:

```rust
// Good — distinguishes receiver-dropped from consumer-stuck
match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(payload)).await {
    Ok(Ok(())) => {}
    Ok(Err(_)) => {
        info!("receiver dropped, ending relay");
        return;
    }
    Err(_) => {
        error!("send timed out, consumer likely stuck");
        return;
    }
}

// Bad — blocks forever if consumer stalls
tx.send(payload).await?;
```

Define the timeout duration as a named const at the top of the file (e.g. `const SEND_TIMEOUT_SECS: u64 = 30`).

## Code Conventions

Every convention exists to maximize readability — code should read like well-written prose where names, structure, and boundaries make intent obvious at a glance.

### Imports

Always import items at the top of the file with `use` statements:

1. No fully qualified paths inline
2. No reaching through an imported module to access a type; import the type directly

```rust
// Good
use foo::bar::{baz, Qux};
let x: Qux = baz();

// Bad
let x: foo::bar::Qux = foo::bar::baz();
```

```rust
// Good — import the type directly
use transport::conn::Connection;
fn open(handle: &mut Connection) {}

// Bad — reaches through an imported module
use transport::conn;
fn open(handle: &mut conn::Connection) {}
```

Exceptions — these are fine to use inline without a `use` import:

- `serde_json::to_string`, `serde_json::from_slice`, `serde_json::from_str`, `serde_json::to_vec`
- `serde_json::Value`
- `serde_json::json!`
- `tracing_subscriber::fmt::init()`
- `std::env::var`
- `aws_smithy_types::Document::*`
- `aws_config::load_defaults`

Combine `use` statements that share the same top-level crate into a single `use` with nested paths:

```rust
// Good
use hyper::{
    body::Bytes,
    rt::{Read, Write},
    Uri,
};

// Bad
use hyper::body::Bytes;
use hyper::rt::{Read, Write};
use hyper::Uri;
```

Group imports into two blocks separated by one blank line:

1. **External** — `std`, third-party crates, workspace crates, `self::`, `super::` (no blank lines within this group)
2. **Crate-local** — everything starting with `crate::` (no blank lines within this group)

If only one group exists, there are no blank lines in the import section.

```rust
// Good
use std::collections::HashMap;
use actix_web::{web, HttpResponse};
use sqlx::SqlitePool;

use crate::pages;
use crate::Args;

// Bad — extra blank lines within the first group
use std::collections::HashMap;

use actix_web::{web, HttpResponse};
use sqlx::SqlitePool;

use crate::pages;
use crate::Args;
```

### Function Naming

Start every function name with a verb. The nouns in the name must match the type being returned or acted on.

```rust
// Good — verb first, noun matches return type
fn get_animal(id: &str) -> Option<Animal>;
fn list_wild_animals(region: &str) -> Vec<WildAnimal>;
fn count_animals() -> i64;
fn create_animal(params: &AnimalParams) -> Animal;
fn update_animal(id: &str, params: &AnimalParams) -> Result<()>;
fn delete_animal(id: &str) -> Result<()>;
fn clear_animals() -> Result<()>;

// Good — single-field setter names the entity and field
fn set_animal_name(id: &str, name: &str) -> Result<()>;
fn set_cage_temperature(id: &str, temp: f64) -> Result<()>;

// Good — transform / produce / convert
fn build_feed_schedule(animals: &[Animal]) -> FeedSchedule;
fn parse_tag_number(raw: &str) -> Option<TagNumber>;
fn validate_cage_size(cage: &Cage) -> Result<(), CageError>;
fn encode_payload(data: &Payload) -> Vec<u8>;
fn decode_payload(raw: &[u8]) -> Result<Payload>;
fn extract_metadata(raw: &[u8]) -> Metadata;
fn compute_feed_cost(schedule: &FeedSchedule) -> f64;
fn format_animal_report(animal: &Animal) -> String;
fn render_animals_view(animals: &[Animal]) -> String;
fn render_new_animal_form(species: &[Species]) -> String;

// Bad — noun doesn't match return type
fn list_animals() -> Vec<WildAnimal>;  // returns WildAnimal, not Animal
fn get_cage(id: &str) -> Option<CageStatus>;  // returns CageStatus, not Cage

// Bad — missing verb
fn animals(region: &str) -> Vec<Animal>;
fn animal_name(id: &str) -> String;

// Bad — ambiguous setter (which field?)
fn set_animal(id: &str, name: &str) -> Result<()>;  // use set_animal_name
```

### Variable Naming

Name variables and parameters after their type in snake_case. For primitives and generic wrappers, use a descriptive domain noun instead.

```rust
// Good — name matches the type
let feed_schedule: FeedSchedule = build_feed_schedule(&feed_request);
let cage_report: CageReport = build_cage_report(&cage);
let animals: Vec<Animal> = list_animals(db);
let cage: Cage = get_cage(cage_id)?;

// Good — primitives use a descriptive domain noun
let feed_cost: f64 = compute_feed_cost(&feed_schedule);
let animal_count: i64 = count_animals(db);
let cage_name: &str = extract_cage_name(&cage);

// Bad — generic names that don't reflect the type or domain
let schedule: FeedSchedule = build_feed_schedule(&feed_request);  // use feed_schedule
let result: CageReport = build_cage_report(&cage);  // use cage_report
let data: Vec<Animal> = list_animals(db);  // use animals
let n: i64 = count_animals(db);  // use animal_count
let val: f64 = compute_feed_cost(&feed_schedule);  // use feed_cost
```

### Function Boundaries

Keep each function at **one level of abstraction**. When a function has distinct sequential phases or repeated structural blocks, extract each into its own named function. A good rule of thumb: if you can give a block of code a meaningful verb-noun name that differs from the parent function, it should be its own function.

#### Sequential pipeline — extract each phase

```rust
// Good — each phase is a small, testable function
fn handle_feed_request(feed_request: &FeedRequest, db: &Db) -> Result<FeedResponse> {
    let feed_request = validate_feed_request(feed_request)?;
    let feed_schedule = build_feed_schedule(&feed_request);
    let feed_cost = compute_feed_cost(&feed_schedule);
    let feed_receipt = store_feed_receipt(db, &feed_schedule, feed_cost)?;
    build_feed_response(&feed_receipt)
}

fn validate_feed_request(feed_request: &FeedRequest) -> Result<FeedRequest> { /* 10–20 lines */ }
fn build_feed_schedule(feed_request: &FeedRequest) -> FeedSchedule { /* 10–20 lines */ }
fn compute_feed_cost(feed_schedule: &FeedSchedule) -> f64 { /* 5–10 lines */ }
fn store_feed_receipt(db: &Db, feed_schedule: &FeedSchedule, feed_cost: f64) -> Result<FeedReceipt> { /* 10 lines */ }
fn build_feed_response(feed_receipt: &FeedReceipt) -> Result<FeedResponse> { /* 5–10 lines */ }

// Bad — one giant function doing validation, building, costing, storing, responding
fn handle_feed_request(feed_request: &FeedRequest, db: &Db) -> Result<FeedResponse> {
    // ... 30 lines of validation ...
    // ... 20 lines building schedule ...
    // ... 15 lines computing cost ...
    // ... 10 lines storing to db ...
    // ... 10 lines building response ...
}
```

#### Loop with a complex body — extract the body

```rust
// Good — loop body is its own function
fn build_inspection_reports(cages: &[Cage], db: &Db) -> Vec<InspectionReport> {
    cages.iter().map(|cage| build_inspection_report(cage, db)).collect()
}

fn build_inspection_report(cage: &Cage, db: &Db) -> InspectionReport {
    let cage_temperature = measure_cage_temperature(cage);
    let cage_cleanliness = evaluate_cage_cleanliness(cage);
    let cage_animals = list_cage_animals(db, cage.id);
    InspectionReport { cage_temperature, cage_cleanliness, cage_animals }
}

// Bad — everything inlined inside the loop
fn build_inspection_reports(cages: &[Cage], db: &Db) -> Vec<InspectionReport> {
    let mut inspection_reports = Vec::new();
    for cage in cages {
        // ... 15 lines measuring temperature ...
        // ... 15 lines evaluating cleanliness ...
        // ... 10 lines querying animals ...
        // ... 10 lines building report ...
        inspection_reports.push(inspection_report);
    }
    inspection_reports
}
```

#### Rendering with distinct sections — extract each section

```rust
// Good — parent composes named section renderers
fn render_cage_detail_view(cage: &Cage, cage_animals: &[CageAnimal]) -> String {
    let cage_breadcrumb = render_cage_breadcrumb(cage);
    let cage_info_section = render_cage_info_section(cage);
    let cage_animal_list = render_cage_animal_list(cage_animals);
    let cage_controls = render_cage_controls(cage);
    format!("{cage_breadcrumb}{cage_info_section}{cage_animal_list}{cage_controls}")
}

fn render_cage_breadcrumb(cage: &Cage) -> String { /* 10 lines */ }
fn render_cage_info_section(cage: &Cage) -> String { /* 15 lines */ }
fn render_cage_animal_list(cage_animals: &[CageAnimal]) -> String { /* 20 lines */ }
fn render_cage_controls(cage: &Cage) -> String { /* 15 lines */ }

// Bad — one function with 80+ lines of concatenated HTML
fn render_cage_detail_view(cage: &Cage, cage_animals: &[CageAnimal]) -> String {
    let mut html = String::new();
    // ... 10 lines breadcrumb ...
    // ... 15 lines info section ...
    // ... 20 lines animal list ...
    // ... 15 lines controls ...
    html
}
```

#### Branching on variant — extract each branch

```rust
// Good — each variant handled by its own function
fn render_enclosure_block(enclosure_block: &EnclosureBlock) -> String {
    match enclosure_block {
        EnclosureBlock::Habitat(habitat) => render_habitat_block(habitat),
        EnclosureBlock::FeedStation(feed_station) => render_feed_station_block(feed_station),
        EnclosureBlock::Observation(observation) => render_observation_block(observation),
    }
}

fn render_habitat_block(habitat: &Habitat) -> String { /* 15 lines */ }
fn render_feed_station_block(feed_station: &FeedStation) -> String { /* 20 lines */ }
fn render_observation_block(observation: &Observation) -> String { /* 15 lines */ }

// Bad — all branches inlined in one long match
fn render_enclosure_block(enclosure_block: &EnclosureBlock) -> String {
    match enclosure_block {
        EnclosureBlock::Habitat(habitat) => {
            // ... 15 lines ...
        }
        EnclosureBlock::FeedStation(feed_station) => {
            // ... 20 lines ...
        }
        EnclosureBlock::Observation(observation) => {
            // ... 15 lines ...
        }
    }
}
```

### Function Arguments

Prefer references (`&`) over owned values in function arguments. Do not use `mut` on parameters unless the function body actually mutates the value.

```rust
// Good — borrows where possible, no unnecessary mut
fn build_feed_schedule(feed_request: &FeedRequest) -> FeedSchedule;
fn apply_filters(data: &mut Value, filters: &[String]);  // mut needed: modifies data in place

// Bad — takes ownership or uses mut unnecessarily
fn build_feed_schedule(feed_request: FeedRequest) -> FeedSchedule;  // use &FeedRequest
fn compute_feed_cost(mut schedule: FeedSchedule) -> f64;  // use &FeedSchedule if not mutated
```

### Return Values

Never return a tuple to bundle multiple values. Split into separate focused functions instead — one per value.

```rust
// Good — two focused functions
fn load_animal_tag(tag_path: &PathBuf) -> Result<Option<Tag>>;
fn load_cage_key(key_path: &PathBuf) -> Result<Arc<Key>>;

// Bad — tuple bundles multiple return values
fn load_cage_data(tag_path: &PathBuf, key_path: &PathBuf) -> Result<(Option<Tag>, Arc<Key>)>;
```

### Streaming Multipart Uploads

When handling a multipart file upload, stream the file field directly to its destination — do not buffer it into `Bytes` first. Wrap the `Field` in a `StreamReader` and pipe it with `tokio::io::copy`.

The file data flows: multipart TCP socket → `Field` stream → `StreamReader` → `tokio::io::copy` → destination writer.

```rust
// Good — file streamed directly to destination
async fn stream_animal_import_file(multipart: &mut Multipart, sftp: SftpSession, cage_path: &str) -> Result<()> {
    while let Some(field) = multipart.next_field().await.context("failed to read multipart field")? {
        if field.name().unwrap_or("") == "file" {
            let mut reader = StreamReader::new(
                field.map_err(|e| IoError::new(ErrorKind::Other, e)),
            );
            return write_animal_file_via_sftp(sftp, cage_path, &mut reader).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}

// Bad — buffers entire file into memory before writing
async fn stream_animal_import_file(multipart: &mut Multipart, sftp: SftpSession, cage_path: &str) -> Result<()> {
    while let Some(field) = multipart.next_field().await.context("failed to read multipart field")? {
        if field.name().unwrap_or("") == "file" {
            let data = field.bytes().await.context("failed to read file")?;  // entire file in memory
            return write_animal_file_via_sftp(sftp, cage_path, &data).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}
```

### Versioning

All crate versions use 3-part semver (e.g. `0.1.0`).
