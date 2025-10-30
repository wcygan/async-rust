# Repository Guidelines

## Project Structure & Module Organization
This workspace gathers a series of numbered example crates (`1-api-request`, `8-basic-counting-future`, etc), each living in its own directory with a dedicated `Cargo.toml`. Source lives under `src/`, typically exposing a `main.rs`; asynchronous helpers are split into lightweight modules when the example grows. Keep new experiments in numeric order and mirror the existing `<id>-<slug>` pattern so `cargo run -p <id>-<slug>` remains predictable. Shared dependencies are declared once in the root `Cargo.toml`.

## Adding New Subprojects
- Pick the next numeric prefix and descriptive slug (e.g., `9-task-runner`) so directory sorting matches execution order, then scaffold the crate with `cargo new --bin 9-task-runner --name task_runner`.
- Append the new directory to the `[workspace].members` list in the root `Cargo.toml`; this keeps `cargo check`, `cargo fmt`, and `cargo test` covering the example automatically.
- Depend on shared crates via the `[workspace.dependencies]` section in the root manifest—add any new versions there first, then reference them from the subproject with `workspace = true` (see `6-async-tail-f/Cargo.toml` for the `tokio`/`clap` pattern).
- Enable crate-specific features inline when pulling from the workspace (`tokio = { workspace = true, features = ["full"] }`) so each example opts into only what it needs without duplicating version pins.
- Run `cargo check -p <id>-<slug>` after wiring everything up to confirm the new example compiles before moving on to implementation details.

## Build, Test, and Development Commands
- `cargo fmt` — format the entire workspace; run before every commit.
- `cargo check` — fast validation of compiler errors across all crates.
- `cargo clippy --all-targets --all-features` — lint for common async pitfalls.
- `cargo test` — execute unit and integration tests; append `-p <id>-<slug>` to scope.
- `cargo run -p <id>-<slug>` — launch an example binary such as the `7-hello-server`.

## Async Rust Fundamentals
- `async fn` and `async { ... }` blocks return futures that stay inert until awaited; wrap entrypoints with a runtime like Tokio (`#[tokio::main]`) so the executor can poll them to completion.
- Use `.await` to cooperatively yield to the scheduler—keep awaits inside small helper functions and rely on async-aware I/O (`tokio::fs`, `reqwest`, `tokio::time`) rather than blocking calls that would stall the reactor.
- Spawn background work with `tokio::spawn` or `tokio::task::JoinSet` when tasks may outlive the caller; capture results with `.await` on the returned handle or by using `tokio::try_join!` when multiple operations must succeed together.
- Cancellation is cooperative: dropping the future stops progress, so ensure important cleanups happen in `Drop` or with `tokio::select!` branches that handle `AbortHandle`/shutdown signals explicitly.
- Attribute bounds matter—prefer `async fn foo(...) -> impl Future<Output = T> + Send` for library code and document when callers must stay on the current thread (`!Send` futures).

## Futures, Streams, and Traits
- A future implements `Future` with a single `poll` method; executors pin and poll it until returning `Poll::Ready`. Most async code works with the ergonomic `.await`, but understanding this contract helps when writing custom combinators.
- Pinning guarantees the memory address of a future stays stable while borrowed self-references exist. Almost all async functions return `!Unpin` futures, so use `Box::pin` or `tokio::pin!` when you need to name a future across awaits.
- Reach for the `futures` crate’s `FutureExt`, `TryFutureExt`, `StreamExt`, and adapters (`map`, `then`, `buffer_unordered`) to compose work without hand-writing loops. Combine multiple futures with macros like `futures::join!`, `select!`, or Tokio’s siblings.
- Streams act as asynchronous iterators (`Stream<Item = T>`); iterate via `while let Some(item) = stream.next().await` and convert between streams and sinks with helpers such as `futures::stream::iter`, `ReceiverStream`, or `tokio_stream`.
- Async I/O traits (`AsyncRead`, `AsyncWrite`, `AsyncBufRead`) pair with Tokio utilities and the `futures` crate to bridge between runtimes—enable the workspace features you need (e.g., `tokio = { workspace = true, features = ["fs","io-util"] }`) so traits are available on the types you use.
- Boxed futures (`type BoxFuture<'a, T> = futures::future::BoxFuture<'a, T>`) or named structs implementing `Future` help when returning different async branches; document lifetime requirements to keep call sites predictable.

## Coding Style & Naming Conventions
Use Rust’s default four-space indentation and rely on `cargo fmt` for canonical formatting. Crate directories follow the chronological `<index>-<slug>` scheme, while the internal crate name (`--name`) should match the slug portion (e.g., `cargo new --bin 9-task-runner --name task_runner`). Favor `snake_case` for functions and modules, `PascalCase` for types, and configuration constants in `SCREAMING_SNAKE_CASE`. Keep modules small and cohesive; rewrite long async blocks into helper functions when clarity suffers.

## Testing Guidelines
Prefer colocating unit tests with the code they cover under `#[cfg(test)] mod tests`, and reach for `#[tokio::test]` when exercising async flows (see `8-basic-counting-future/src/main.rs`). Use descriptive test names such as `async_tail_reads_latest_line`. When adding integration tests, place them under a crate’s `tests/` directory and favor deterministic fixtures over timing-based checks. Ensure new tests pass with `cargo test` before opening a review.

## Commit & Pull Request Guidelines
Commit history favors brief, imperative summaries (`make it a cargo workspace`, `counting future`). Mirror that voice: keep messages under 60 characters, present tense, and scoped to one logical change. Pull requests should include: a concise summary of the experiment, relevant commands (`cargo run -p ...`), and links to tracking issues or references. Mention any required environment variables or network access so reviewers can reproduce results confidently.
