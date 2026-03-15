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

## App State

Never implement `Deref` (or `DerefMut`) on an app state struct to expose its config or any sub-field. All config access must go through an explicit field path (`state.config.field`). A `Deref` impl makes dependencies invisible — it becomes impossible to tell at a glance whether `state.ssh_key_path` is a field on the state struct or on something it derefs to.

```rust
// Good — explicit field path; dependency is obvious
let ssh_handle = connect_ssh(guest_ip, &state.config.ssh_key_path, ...).await?;
let vm_config = build_vm_config(&state.config.vm_build_config(), ...).await?;

// Bad — Deref on AppState hides where the field comes from
impl std::ops::Deref for AppState {
    type Target = AppConfig;
    fn deref(&self) -> &AppConfig { &self.config }
}
let ssh_handle = connect_ssh(guest_ip, &state.ssh_key_path, ...).await?;  // is this AppState or AppConfig?
```

## Authorization

When a handler looks up a resource by a user-provided ID, return 404 if the lookup fails — never fall back to a different resource. A fallback silently operates on a resource the user did not request and may not own.

```rust
// Good — strict 404 on miss; no fallback
let Some(guest_ip) = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)? else {
    return Ok((StatusCode::NOT_FOUND, "Session not found or expired").into_response());
};

// Bad — falls back to any VM owned by the user when the specific one is not found
let guest_ip = find_vm_guest_ip_for_user(&state.vms, &vm_id, db_user.id)?
    .or_else(|| find_any_vm_for_user(&state.vms, db_user.id))  // wrong VM, wrong resource
    .context("no VM found")?;
```

## Channel Sends

Always wrap `mpsc::Sender::send` with `tokio::time::timeout`. A send with no timeout will block forever if the receiver is alive but not consuming — which can happen on an unstable network where the TCP connection appears open but the client is stalled. Use a three-arm `match` because the two failure modes require different actions: a dropped receiver is a normal shutdown (`info`), a send timeout means the consumer is stuck (`error`). Chained `.context()?` collapses both into one error path and loses that distinction:

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

When senders are stored in a registry (e.g. a `HashMap`), lazily remove closed entries at the point of lookup. Do not rely on a separate cleanup pass. Check `sender.is_closed()` before cloning and returning the sender; if it is closed, remove it and return `None`.

```rust
// Good — closed sender removed on access; no separate cleanup needed
fn find_herd_relay_sender(state: &AppState, herd_id: &str) -> Option<mpsc::Sender<HerdMsg>> {
    let mut senders = state.herd_senders.lock().ok()?;
    let sender = senders.get(herd_id)?;
    if sender.is_closed() {
        senders.remove(herd_id);
        return None;
    }
    Some(sender.clone())
}

// Bad — stale closed senders accumulate in the map indefinitely
fn find_herd_relay_sender(state: &AppState, herd_id: &str) -> Option<mpsc::Sender<HerdMsg>> {
    state.herd_senders.lock().ok()?.get(herd_id).cloned()
}
```

## Async Operation Timeouts

For one-shot async operations that return `Result`, wrap with `tokio::time::timeout` and chain two `.context()` calls: one for the timeout expiry, one for the operation failure. `timeout` wraps the inner `Result`, producing `Result<Result<T, E>, Elapsed>` — each `?` unwraps one layer.

```rust
// Good — two context calls, one per error layer
let barn_channel = timeout(
    Duration::from_secs(BARN_OP_TIMEOUT_SECS),
    barn_handle.open_feed_session(),
)
.await
.context("feed session open timed out")?
.context("feed session open failed")?;

// Good — with_context when the message needs runtime values
timeout(
    Duration::from_secs(BARN_OP_TIMEOUT_SECS),
    barn_handle.relay_to_pasture(pasture_id),
)
.await
.context("pasture relay timed out")?
.with_context(|| format!("failed to relay to pasture {pasture_id}"))?;

// Bad — single context loses the distinction between timeout and operation failure
let barn_channel = timeout(
    Duration::from_secs(BARN_OP_TIMEOUT_SECS),
    barn_handle.open_feed_session(),
)
.await
.context("feed session failed")?;  // was it a timeout or the open itself?

// Bad — no timeout; hangs forever if the barn handle stalls
let barn_channel = barn_handle.open_feed_session().await.context("feed session open failed")?;
```

For operations that must retry until success (e.g. waiting for a service to become reachable), extract the retry loop into its own function that loops forever, then wrap the call site in `timeout`. The loop itself has no timeout — the outer `timeout` at the call site bounds the total retry window.

```rust
// Good — loop retries forever; caller bounds total time with timeout
async fn connect_barn_monitor(
    barn_addr: SocketAddr,
    barn_client: BarnMonitorClient,
) -> Result<BarnMonitorHandle> {
    let barn_handle = timeout(
        Duration::from_secs(BARN_CONNECT_TIMEOUT_SECS),
        connect_barn_monitor_handle(barn_addr, barn_client),
    )
    .await
    .context("barn monitor connect timed out")?;
    Ok(barn_handle)
}

async fn connect_barn_monitor_handle(
    barn_addr: SocketAddr,
    barn_client: BarnMonitorClient,
) -> BarnMonitorHandle {
    loop {
        if let Ok(barn_handle) = connect(barn_addr, barn_client.clone()).await {
            return barn_handle;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// Bad — timeout inside the loop; each attempt gets a full timeout budget
async fn connect_barn_monitor_handle(barn_addr: SocketAddr) -> Result<BarnMonitorHandle> {
    loop {
        if let Ok(barn_handle) = timeout(
            Duration::from_secs(BARN_CONNECT_TIMEOUT_SECS),
            connect(barn_addr, BarnMonitorClient::new()),
        )
        .await { return Ok(barn_handle); }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
```

Use `match timeout(...)` only when the two failure arms require different handling — for example, logging at different levels or taking different actions. When both failures should simply propagate as errors, use chained `.context()?` instead.

```rust
// Good — match because timeout and operation failure need different actions
match timeout(Duration::from_secs(BARN_OP_TIMEOUT_SECS), tx.send(feed_delivery)).await {
    Ok(Ok(())) => {}
    Ok(Err(_)) => {
        info!("feed relay receiver dropped, ending round");  // normal shutdown
        return;
    }
    Err(_) => {
        error!("feed relay send timed out, consumer likely stuck");  // abnormal stall
        return;
    }
}

// Good — chained context because both failures just propagate
let barn_channel = timeout(
    Duration::from_secs(BARN_OP_TIMEOUT_SECS),
    barn_handle.open_feed_session(),
)
.await
.context("feed session open timed out")?
.context("feed session open failed")?;
```

Define the timeout duration as a named const at the top of the file (e.g. `const BARN_OP_TIMEOUT_SECS: u64 = 30`).

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
- `russh::Error` (in `type Error = russh::Error;` associated type definitions)

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

When validating that a path stays within an allowed directory, always canonicalize the path first (e.g. via `sftp.canonicalize` or `std::fs::canonicalize`) before calling `validate_within_dir`. The `validate_within_dir` helper also rejects any path that contains a `..` component as defense-in-depth — callers must not rely on this alone and must canonicalize first.

```rust
// Good — canonicalize first, then validate; both layers active
let real_path = PathBuf::from(sftp.canonicalize(&query.path).await.context("failed to resolve remote path")?);
validate_within_dir(&real_path, &PathBuf::from(&state.config.upload_dir))?;

// Bad — validates a raw user-supplied path; "../../../etc/passwd" bypasses the prefix check
validate_within_dir(Path::new(&query.path), &PathBuf::from(&state.config.upload_dir))?;
```

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

### Network Addresses

Use `std::net::SocketAddr` for all socket addresses — never format them as strings. Parse IP strings into `IpAddr` at the entry boundary and construct a `SocketAddr` with an explicit port.

```rust
// Good — typed address, port explicit, parse error caught early
let guest_addr = SocketAddr::new(guest_ip.parse::<IpAddr>().context("invalid guest IP")?, 22);
connect(config, guest_addr, client).await?;

// Bad — raw string, malformed address only caught at connect time
let addr = format!("{guest_ip}:22");
connect(config, addr.as_str(), client).await?;
```
