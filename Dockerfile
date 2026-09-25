ARG FOLIOFORGE_VERSION=0.3.1

FROM rust:1.88-bookworm AS builder
ARG FOLIOFORGE_VERSION
WORKDIR /src
COPY . .
RUN workspace_version=$(awk '/^\[workspace\.package\]/{section=1;next} /^\[/{section=0} section && $1=="version"{gsub(/"/,"",$3);print $3;exit}' Cargo.toml) \
    && test "$workspace_version" = "$FOLIOFORGE_VERSION" \
    && cargo build --locked --release -p folio-service

FROM debian:bookworm-slim
ARG FOLIOFORGE_VERSION
LABEL org.opencontainers.image.title="FolioForge" \
      org.opencontainers.image.version="${FOLIOFORGE_VERSION}"
RUN groupadd --system folioforge && useradd --system --gid folioforge --home-dir /work --no-create-home folioforge \
    && mkdir -p /work \
    && chown -R folioforge:folioforge /work
COPY --from=builder /src/target/release/folio-service /usr/local/bin/folio-service
USER folioforge
WORKDIR /work
ENV FOLIOFORGE_BIND=0.0.0.0:8080
ENV FOLIOFORGE_WORK_DIR=/work
ENV FOLIOFORGE_MAX_BODY_BYTES=536870912
ENV FOLIOFORGE_MAX_FILES=256
ENV FOLIOFORGE_MAX_JOB_SECONDS=1800
ENV FOLIOFORGE_ONLINE_ENABLED=0
EXPOSE 8080
VOLUME ["/work"]
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s CMD ["/usr/local/bin/folio-service", "--healthcheck"]
ENTRYPOINT ["/usr/local/bin/folio-service"]
