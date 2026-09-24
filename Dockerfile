FROM rust:1.98.1-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS build
ARG OWNPG_BUILD_COMMIT=unknown
ARG SOURCE_DATE_EPOCH=
ENV OWNPG_BUILD_COMMIT=$OWNPG_BUILD_COMMIT \
    SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH
RUN cargo install --locked cargo-auditable@0.7.5 cargo-about@0.9.2
RUN install -d -m 0700 /out/var/lib/ownpg /out/var/lib/ownpg/config /out/var/lib/ownpg/data /out/var/lib/ownpg/state
WORKDIR /src
COPY Cargo.toml Cargo.lock about.toml about.hbs LICENSE-MIT LICENSE-APACHE ./
COPY crates ./crates
RUN cargo auditable build --profile dist --locked -p ownpg \
    && cargo about generate about.hbs -o THIRD-PARTY.txt \
    && install -m 0755 target/dist/ownpg /ownpg \
    && mkdir -p /doc \
    && install -m 0644 THIRD-PARTY.txt LICENSE-MIT LICENSE-APACHE /doc/

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:54df941ed0d06a1bd95ef5e0ce391fd8d9f94b64782dc9a60062727849ee3f97
ARG OWNPG_VERSION=0.0.0-local
ARG OWNPG_BUILD_COMMIT=unknown
LABEL org.opencontainers.image.title="OwnPG" \
      org.opencontainers.image.description="OwnPG MCP server for PostgreSQL in remote mode" \
      org.opencontainers.image.source="https://github.com/devops-infinity/ownpg-releases" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.version="$OWNPG_VERSION" \
      org.opencontainers.image.revision="$OWNPG_BUILD_COMMIT"
COPY --from=build /ownpg /usr/local/bin/ownpg
COPY --from=build /doc/ /usr/share/doc/ownpg/
COPY --from=build --chown=65532:65532 /out/var/lib/ownpg /var/lib/ownpg
ENV OWNPG_BIND=0.0.0.0:8765 \
    OWNPG_NO_INPUT=true \
    XDG_CONFIG_HOME=/var/lib/ownpg/config \
    XDG_DATA_HOME=/var/lib/ownpg/data \
    XDG_STATE_HOME=/var/lib/ownpg/state
USER 65532:65532
WORKDIR /var/lib/ownpg
EXPOSE 8765
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
  CMD ["/usr/local/bin/ownpg", "health", "--quiet"]
ENTRYPOINT ["/usr/local/bin/ownpg"]
CMD ["serve", "--http"]
