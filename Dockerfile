# The builder installs the exact toolchain; Cargo.lock controls dependencies.
FROM rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p opensnow-mcp
# Keep only the executable and its actual ELF dependencies, plus notices.
# No shell, package manager, compiler, or source tree enters the runtime.
RUN set -eu; mkdir -p /runtime/usr/local/bin /runtime/warehouse /runtime/tmp /runtime/usr/share/doc; \
    chmod 1777 /runtime/tmp; \
    cp target/release/opensnow-mcp /runtime/usr/local/bin/; \
    ldd target/release/opensnow-mcp; \
    ldd target/release/opensnow-mcp | awk '$2 == "=>" {print $3} $1 ~ /^\// {print $1}' > /tmp/libs; \
    for lib in $(cat /tmp/libs); do \
      case "$(basename "$lib")" in libc.so.6|libm.so.6|libgcc_s.so.1|ld-linux-x86-64.so.2) ;; *) echo "Unaudited shared library: $lib" >&2; exit 1;; esac; \
      cp --parents -L "$lib" /runtime; \
    done; \
    cp --parents -L /lib64/ld-linux-x86-64.so.2 /runtime; \
    for pkg in libc6 libgcc-s1 ca-certificates; do \
      mkdir -p "/runtime/usr/share/doc/$pkg"; \
      cp -L "/usr/share/doc/$pkg/copyright" "/runtime/usr/share/doc/$pkg/"; \
    done; \
    mkdir -p /runtime/usr/share/common-licenses; \
    cp /usr/share/common-licenses/GPL-* /usr/share/common-licenses/LGPL-* /runtime/usr/share/common-licenses/; \
    mkdir -p /runtime/etc/ssl/certs; \
    cp /etc/ssl/certs/ca-certificates.crt /runtime/etc/ssl/certs/; \
    mkdir -p /runtime/usr/share/doc/opensnow; \
    cp deploy/dependency-licenses.tsv /runtime/usr/share/doc/opensnow/; \
    cp vendor/iceberg-0.11.0/LICENSE vendor/iceberg-0.11.0/NOTICE /runtime/usr/share/doc/opensnow/; \
    mkdir -p /runtime/usr/share/doc/opensnow/vega; \
    cp -r vendor/vega/*.LICENSE vendor/vega/dependency-licenses.tsv vendor/vega/third-party /runtime/usr/share/doc/opensnow/vega/; \
    mkdir -p /runtime/usr/share/doc/opensnow/third-party-licenses; \
    find "$CARGO_HOME/registry/src" -type f \( -iname 'license*' -o -iname 'notice*' -o -iname 'copying*' -o -iname 'unlicense*' \) \
      -exec cp --parents '{}' /runtime/usr/share/doc/opensnow/third-party-licenses/ \;; \
    chown 65532:65532 /runtime/warehouse

FROM scratch
LABEL org.opencontainers.image.title="opensnow" \
      org.opencontainers.image.licenses="Apache-2.0"
COPY --from=build /runtime/ /
USER 65532:65532
ENV OPENSNOW_WAREHOUSE=/warehouse
VOLUME ["/warehouse"]
EXPOSE 8080
# No shell or HTTP client in this image. See DEPLOYMENT.md for the
# external GET /health check used by the host and verify-container.sh.
ENTRYPOINT ["/usr/local/bin/opensnow-mcp"]
CMD ["--http", "0.0.0.0:8080"]
