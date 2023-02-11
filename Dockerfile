FROM lukemathwalker/cargo-chef:latest-rust-1.60 AS chef
WORKDIR project_manager

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /project_manager/recipe.json recipe.json
# Build dependencies - this is the caching Docker layer!
RUN cargo chef cook --release --recipe-path recipe.json
# Build project_managerlication
COPY . .
RUN cargo build --release --bin project_manager

# We do not need the Rust toolchain to run the binary!
FROM debian as runtime
RUN apt-get update && apt-get install -y ca-certificates
WORKDIR project_manager
COPY --from=builder /project_manager/target/release/project_manager /usr/local/bin
ENTRYPOINT ["/usr/local/bin/project_manager"]