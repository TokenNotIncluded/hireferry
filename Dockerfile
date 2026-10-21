FROM rust:1.99-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static
COPY legal ./legal
RUN cargo build --locked --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 hireferry \
    && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /app \
       --shell /usr/sbin/nologin hireferry \
    && install -d -m 0700 -o 10001 -g 10001 /app/data \
    && install -d -m 0750 -o 10001 -g 10001 /app/config
WORKDIR /app
COPY --from=builder /build/target/release/hireferry /usr/local/bin/hireferry
COPY LICENSE /usr/share/licenses/hireferry/LICENSE
COPY static/runtime/NOTICE /usr/share/licenses/hireferry/particle-sculpture-NOTICE
COPY legal /usr/share/hireferry/legal
USER 10001:10001
EXPOSE 3000
STOPSIGNAL SIGINT
ENTRYPOINT ["/usr/local/bin/hireferry"]
CMD ["--config", "/app/config/config.toml", "serve"]
