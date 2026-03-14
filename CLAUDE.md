# CLAUDE.md

> This project models a cattle ranch. Ranchers manage **livestock** across **barns** and **pastures**, track animals by **brand number**, and schedule **feeding rounds**. All code examples use this domain.

## Crates

- Use `anyhow` for error handling.
- Use `clap` for CLI argument parsing.

### `common` crate

Put utilities in `common` when they are used by two or more crates and carry no domain-specific logic. Import from `common` directly — do not re-export through a domain crate.

```rust
// Good — shared utility lives in common, each crate imports it directly
// common/src/lib.rs
pub fn parse_brand_number(raw: &str) -> Option<BrandNumber> { ... }

// feeding/src/lib.rs
use common::parse_brand_number;

// pasture/src/lib.rs
use common::parse_brand_number;

// Bad — utility defined in one domain crate and imported by another
// feeding/src/lib.rs
pub fn parse_brand_number(raw: &str) -> Option<BrandNumber> { ... }  // belongs in common

// pasture/src/lib.rs
use feeding::parse_brand_number;  // pasture should not depend on feeding
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
pub livestock_type: String,  // prefixed rename

#[serde(rename = "type")]
pub barn_type: String,  // prefixed rename
```

## Error Handling

When an operation returns `Result`, propagate the error with `?` and add context with `.context("...")`. Never swallow errors by converting them to a default value.

```rust
// Good — fail with context
let barn_dir = path.parent().context("barn path has no parent")?;
let weight = livestock.weight.context("missing livestock weight")?;
let brand = BrandTag::from_str(&value).context("invalid brand tag")?;

// Bad — silently substitute a default
let barn_dir = path.parent().unwrap_or(".");
let weight = livestock.weight.unwrap_or(0);
let brand = BrandTag::from_str(&value).unwrap_or(BrandTag::default());
```

Use `Option` only for values that are genuinely absent as part of normal logic (e.g. "livestock has no barn", "search found no match"). Use `Result` for anything that can fail due to I/O, missing data, or invalid input.

## Channel Sends

Always wrap `mpsc::Sender::send` with `tokio::time::timeout`. A send with no timeout will block forever if the receiver is alive but not consuming — which can happen on an unstable network where the TCP connection appears open but the client is stalled. Use a three-arm match to distinguish the two failure modes:

```rust
// Good — distinguishes receiver-dropped from consumer-stuck
match timeout(Duration::from_secs(SEND_TIMEOUT_SECS), tx.send(feed_delivery)).await {
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
tx.send(feed_delivery).await?;
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
fn get_livestock(id: &str) -> Option<Livestock>;
fn list_open_range_cattle(pasture: &str) -> Vec<OpenRangeCattle>;
fn count_livestock() -> i64;
fn create_livestock(params: &LivestockParams) -> Livestock;
fn update_livestock(id: &str, params: &LivestockParams) -> Result<()>;
fn delete_livestock(id: &str) -> Result<()>;
fn clear_livestock() -> Result<()>;

// Good — single-field setter names the entity and field
fn set_livestock_name(id: &str, name: &str) -> Result<()>;
fn set_barn_temperature(id: &str, temp: f64) -> Result<()>;

// Good — transform / produce / convert
fn build_grazing_schedule(livestock: &[Livestock]) -> GrazingSchedule;
fn parse_brand_number(raw: &str) -> Option<BrandNumber>;
fn validate_barn_capacity(barn: &Barn) -> Result<(), BarnError>;
fn encode_herd_record(record: &HerdRecord) -> Vec<u8>;
fn decode_herd_record(raw: &[u8]) -> Result<HerdRecord>;
fn extract_brand_metadata(raw: &[u8]) -> BrandMetadata;
fn compute_feed_cost(schedule: &GrazingSchedule) -> f64;
fn format_livestock_report(livestock: &Livestock) -> String;
fn render_livestock_view(livestock: &[Livestock]) -> String;
fn render_new_livestock_form(breeds: &[Breed]) -> String;

// Bad — noun doesn't match return type
fn list_livestock() -> Vec<OpenRangeCattle>;  // returns OpenRangeCattle, not Livestock
fn get_barn(id: &str) -> Option<BarnStatus>;  // returns BarnStatus, not Barn

// Bad — missing verb
fn livestock(pasture: &str) -> Vec<Livestock>;
fn livestock_name(id: &str) -> String;

// Bad — ambiguous setter (which field?)
fn set_livestock(id: &str, name: &str) -> Result<()>;  // use set_livestock_name
```

### Variable Naming

Name variables and parameters after their type in snake_case. For primitives and generic wrappers, use a descriptive domain noun instead.

```rust
// Good — name matches the type
let grazing_schedule: GrazingSchedule = build_grazing_schedule(&feed_order);
let barn_report: BarnReport = build_barn_report(&barn);
let livestock: Vec<Livestock> = list_livestock(db);
let barn: Barn = get_barn(barn_id)?;

// Good — primitives use a descriptive domain noun
let feed_cost: f64 = compute_feed_cost(&grazing_schedule);
let livestock_count: i64 = count_livestock(db);
let barn_name: &str = extract_barn_name(&barn);

// Bad — generic names that don't reflect the type or domain
let schedule: GrazingSchedule = build_grazing_schedule(&feed_order);  // use grazing_schedule
let result: BarnReport = build_barn_report(&barn);  // use barn_report
let data: Vec<Livestock> = list_livestock(db);  // use livestock
let n: i64 = count_livestock(db);  // use livestock_count
let val: f64 = compute_feed_cost(&grazing_schedule);  // use feed_cost
```

### Function Boundaries

Keep each function at **one level of abstraction**. When a function has distinct sequential phases or repeated structural blocks, extract each into its own named function. A good rule of thumb: if you can give a block of code a meaningful verb-noun name that differs from the parent function, it should be its own function.

#### Sequential pipeline — extract each phase

```rust
// Good — each phase is a small, testable function
fn handle_feed_order(feed_order: &FeedOrder, db: &Db) -> Result<FeedConfirmation> {
    let feed_order = validate_feed_order(feed_order)?;
    let grazing_schedule = build_grazing_schedule(&feed_order);
    let feed_cost = compute_feed_cost(&grazing_schedule);
    let feed_receipt = store_feed_receipt(db, &grazing_schedule, feed_cost)?;
    build_feed_confirmation(&feed_receipt)
}

fn validate_feed_order(feed_order: &FeedOrder) -> Result<FeedOrder> { /* 10–20 lines */ }
fn build_grazing_schedule(feed_order: &FeedOrder) -> GrazingSchedule { /* 10–20 lines */ }
fn compute_feed_cost(grazing_schedule: &GrazingSchedule) -> f64 { /* 5–10 lines */ }
fn store_feed_receipt(db: &Db, grazing_schedule: &GrazingSchedule, feed_cost: f64) -> Result<FeedReceipt> { /* 10 lines */ }
fn build_feed_confirmation(feed_receipt: &FeedReceipt) -> Result<FeedConfirmation> { /* 5–10 lines */ }

// Bad — one giant function doing validation, building, costing, storing, responding
fn handle_feed_order(feed_order: &FeedOrder, db: &Db) -> Result<FeedConfirmation> {
    // ... 30 lines of validation ...
    // ... 20 lines building schedule ...
    // ... 15 lines computing cost ...
    // ... 10 lines storing to db ...
    // ... 10 lines building confirmation ...
}
```

#### Loop with a complex body — extract the body

```rust
// Good — loop body is its own function
fn build_barn_inspection_reports(barns: &[Barn], db: &Db) -> Vec<BarnInspectionReport> {
    barns.iter().map(|barn| build_barn_inspection_report(barn, db)).collect()
}

fn build_barn_inspection_report(barn: &Barn, db: &Db) -> BarnInspectionReport {
    let barn_temperature = measure_barn_temperature(barn);
    let barn_cleanliness = evaluate_barn_cleanliness(barn);
    let barn_livestock = list_barn_livestock(db, barn.id);
    BarnInspectionReport { barn_temperature, barn_cleanliness, barn_livestock }
}

// Bad — everything inlined inside the loop
fn build_barn_inspection_reports(barns: &[Barn], db: &Db) -> Vec<BarnInspectionReport> {
    let mut barn_inspection_reports = Vec::new();
    for barn in barns {
        // ... 15 lines measuring temperature ...
        // ... 15 lines evaluating cleanliness ...
        // ... 10 lines querying livestock ...
        // ... 10 lines building report ...
        barn_inspection_reports.push(barn_inspection_report);
    }
    barn_inspection_reports
}
```

#### Rendering with distinct sections — extract each section

```rust
// Good — parent composes named section renderers
fn render_barn_detail_view(barn: &Barn, barn_livestock: &[BarnLivestock]) -> String {
    let barn_breadcrumb = render_barn_breadcrumb(barn);
    let barn_info_section = render_barn_info_section(barn);
    let barn_livestock_list = render_barn_livestock_list(barn_livestock);
    let barn_controls = render_barn_controls(barn);
    format!("{barn_breadcrumb}{barn_info_section}{barn_livestock_list}{barn_controls}")
}

fn render_barn_breadcrumb(barn: &Barn) -> String { /* 10 lines */ }
fn render_barn_info_section(barn: &Barn) -> String { /* 15 lines */ }
fn render_barn_livestock_list(barn_livestock: &[BarnLivestock]) -> String { /* 20 lines */ }
fn render_barn_controls(barn: &Barn) -> String { /* 15 lines */ }

// Bad — one function with 80+ lines of concatenated HTML
fn render_barn_detail_view(barn: &Barn, barn_livestock: &[BarnLivestock]) -> String {
    let mut html = String::new();
    // ... 10 lines breadcrumb ...
    // ... 15 lines info section ...
    // ... 20 lines livestock list ...
    // ... 15 lines controls ...
    html
}
```

#### Branching on variant — extract each branch

```rust
// Good — each variant handled by its own function
fn render_pasture_block(pasture_block: &PastureBlock) -> String {
    match pasture_block {
        PastureBlock::Barn(barn) => render_barn_block(barn),
        PastureBlock::WaterTrough(water_trough) => render_water_trough_block(water_trough),
        PastureBlock::GrazingObservation(observation) => render_grazing_observation_block(observation),
    }
}

fn render_barn_block(barn: &Barn) -> String { /* 15 lines */ }
fn render_water_trough_block(water_trough: &WaterTrough) -> String { /* 20 lines */ }
fn render_grazing_observation_block(observation: &GrazingObservation) -> String { /* 15 lines */ }

// Bad — all branches inlined in one long match
fn render_pasture_block(pasture_block: &PastureBlock) -> String {
    match pasture_block {
        PastureBlock::Barn(barn) => {
            // ... 15 lines ...
        }
        PastureBlock::WaterTrough(water_trough) => {
            // ... 20 lines ...
        }
        PastureBlock::GrazingObservation(observation) => {
            // ... 15 lines ...
        }
    }
}
```

### Function Arguments

Prefer references (`&`) over owned values in function arguments. Do not use `mut` on parameters unless the function body actually mutates the value.

```rust
// Good — borrows where possible, no unnecessary mut
fn build_grazing_schedule(feed_order: &FeedOrder) -> GrazingSchedule;
fn apply_brand_filters(herd: &mut HerdRecord, filters: &[String]);  // mut needed: modifies herd in place

// Bad — takes ownership or uses mut unnecessarily
fn build_grazing_schedule(feed_order: FeedOrder) -> GrazingSchedule;  // use &FeedOrder
fn compute_feed_cost(mut schedule: GrazingSchedule) -> f64;  // use &GrazingSchedule if not mutated
```

### Return Values

Never return a tuple to bundle multiple values. Split into separate focused functions instead — one per value.

```rust
// Good — two focused functions
fn load_livestock_brand(brand_path: &Path) -> Result<Option<Brand>>;
fn load_barn_key(key_path: &Path) -> Result<Arc<Key>>;

// Bad — tuple bundles multiple return values
fn load_barn_data(brand_path: &Path, key_path: &Path) -> Result<(Option<Brand>, Arc<Key>)>;
```

### Streaming Multipart Uploads

When handling a multipart file upload, stream the file field directly to its destination — do not buffer it into `Bytes` first. Wrap the `Field` in a `StreamReader` and pipe it with `tokio::io::copy`.

The file data flows: multipart TCP socket → `Field` stream → `StreamReader` → `tokio::io::copy` → destination writer.

```rust
// Good — file streamed directly to destination
async fn stream_livestock_import_file(multipart: &mut Multipart, sftp: SftpSession, barn_path: &Path, pasture_dir: &Path) -> Result<()> {
    while let Some(field) = multipart.next_field().await.context("failed to read multipart field")? {
        if field.name().unwrap_or("") == "file" {
            let mut reader = StreamReader::new(
                field.map_err(|e| IoError::new(ErrorKind::Other, e)),
            );
            return write_livestock_file_via_sftp(sftp, barn_path, pasture_dir, &mut reader).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}

// Bad — buffers entire file into memory before writing
async fn stream_livestock_import_file(multipart: &mut Multipart, sftp: SftpSession, barn_path: &Path, pasture_dir: &Path) -> Result<()> {
    while let Some(field) = multipart.next_field().await.context("failed to read multipart field")? {
        if field.name().unwrap_or("") == "file" {
            let data = field.bytes().await.context("failed to read file")?;  // entire file in memory
            return write_livestock_file_via_sftp(sftp, barn_path, pasture_dir, &data).await;
        }
    }
    Err(anyhow!("missing 'file' field"))
}
```

### Path Handling

Use `Path`/`PathBuf` for all path operations — no string manipulation for paths. Functions that receive paths take `&Path`; functions that construct a new path return `PathBuf`. Convert strings to `&Path` once at the entry boundary.

```rust
// Good — &Path in, PathBuf out; Path::new() once at the entry point
fn register_livestock_handler(...) {
    let livestock_path_str = /* read from request */;
    store_livestock_record(Path::new(&livestock_path_str), Path::new(&state.barn_dir));
}

fn store_livestock_record(livestock_path: &Path, barn_dir: &Path) { ... }

fn resolve_livestock_path(livestock_path: &Path, barn_dir: &Path) -> Result<PathBuf> {
    let resolved = PathBuf::from(canonical).join(livestock_path.file_name().context("livestock path has no filename")?);
    validate_within_barn_dir(&resolved, barn_dir)?;
    Ok(resolved)
}

// Bad — &str throughout, string manipulation, Path::new() buried inside helpers
fn store_livestock_record(livestock_path: &str, barn_dir: &str) { ... }

fn resolve_livestock_path(livestock_path: &str, barn_dir: &str) -> Result<String> {
    let resolved = format!("{}/{}", canonical.trim_end_matches('/'), livestock_name);
    if !Path::new(&resolved).starts_with(barn_dir) { ... }
    Ok(resolved)
}
```

### Versioning

All crate versions use 3-part semver (e.g. `0.1.0`).
