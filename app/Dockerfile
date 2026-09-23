# syntax=docker/dockerfile:1
# Preview build: version tags are explicit; release promotion must pin image digests
# and the exact Debian FFmpeg package/source snapshot (see docs/CROSS-PLATFORM.md).
ARG NODE_IMAGE=node:24.21.0-bookworm-slim
ARG RUST_IMAGE=rust:1.98.1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

FROM ${NODE_IMAGE} AS web
WORKDIR /build/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM ${RUST_IMAGE} AS rust
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
# Cargo resolves every workspace manifest, but only the server/core are compiled.
COPY desktop/Cargo.toml desktop/Cargo.toml
COPY desktop/build.rs desktop/build.rs
COPY desktop/src/ desktop/src/
COPY --from=web /build/web/dist/ web/dist/
RUN cargo build --release --locked -p u2bup-server

FROM ${RUNTIME_IMAGE} AS runtime
RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates ffmpeg curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 u2bup \
    && useradd --uid 10001 --gid 10001 --no-create-home u2bup \
    && mkdir -p /media/library /data /output /usr/share/u2bup \
    && chown 10001:10001 /data /output \
    && dpkg-query -W > /usr/share/u2bup/package-versions.txt
COPY --from=rust /build/target/release/u2bup-server /usr/local/bin/u2bup-server
COPY THIRD-PARTY-NOTICES.md /usr/share/u2bup/THIRD-PARTY-NOTICES.md
USER 10001:10001
WORKDIR /data
EXPOSE 4173
STOPSIGNAL SIGTERM
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl --silent --output /dev/null http://127.0.0.1:4173/ || exit 1
ENTRYPOINT ["u2bup-server"]
CMD ["--headless", "--bind", "0.0.0.0", "--public-origin", "http://127.0.0.1:4173", "--library", "/media/library", "--data", "/data", "--output", "/output", "--ffmpeg", "/usr/bin/ffmpeg", "--ffprobe", "/usr/bin/ffprobe"]
