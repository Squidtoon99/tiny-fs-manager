# generic lite rust container with musl & openssl
FROM --platform=linux/amd64 rust:alpine3.17 as rust-alpine
RUN apk add --no-cache musl-dev openssl-dev
WORKDIR /usr/src/app

# generic chef to minimize redundancy
FROM rust-alpine as chef
RUN cargo install cargo-chef

# generate a recipe file for dependencies
FROM chef as planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# build & cache dependencies
FROM chef as cacher
COPY --from=planner /usr/src/app/recipe.json recipe.json
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

# use rust:alpine3.17 docker image as builder
FROM rust-alpine as builder
# create user
ENV USER=dev
ENV UID=1337
RUN adduser \
    --disabled-password \
    --gecos "" \
    --home "/nonexistent" \
    --shell "/sbin/nologin" \
    --no-create-home \
    --uid "${UID}" \
    "${USER}"
# copy build image
COPY . .
COPY --from=cacher /usr/src/app/target target
COPY --from=cacher /usr/local/cargo /usr/local/cargo
RUN cargo build --release

# run executable from distroless build
FROM --platform=linux/amd64 gcr.io/distroless/cc-debian11 as development
# copy user from builder
COPY --from=builder /etc/passwd /etc/passwd
COPY --from=builder /etc/group /etc/group
# copy executable from builder
COPY --from=builder /usr/src/app/target/release/project_manager /usr/local/bin/project_manager
# set user
USER dev:dev
# run executable
CMD ["/usr/local/bin/project_manager"]