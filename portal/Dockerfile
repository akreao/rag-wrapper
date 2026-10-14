# The account web portal: one static Go binary with its pages built in,
# plus the Windows client patcher it hands out to players.
FROM golang:1.24-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /portal .

# rpatchur (github.com/L1nkZ/rpatchur, MIT/Apache-2.0) has no published
# releases, so it's cross-compiled for Windows from a pinned commit. This
# stage is cached, so it only rebuilds when the commit changes. Rust 1.68
# is the newest that still builds its locked dependencies (ntapi 0.3.6
# fails on 1.69 and later).
FROM rust:1.68-bullseye AS patcher
ARG RPATCHUR_COMMIT=21a548277189fecf1c24f61565337158c4ab19a9
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add x86_64-pc-windows-gnu
WORKDIR /rpatchur
RUN git init -q \
 && git fetch -q --depth 1 https://github.com/L1nkZ/rpatchur.git "$RPATCHUR_COMMIT" \
 && git checkout -q FETCH_HEAD
RUN cargo build --locked --release --target x86_64-pc-windows-gnu -p rpatchur \
 && mkdir /out \
 && cp target/x86_64-pc-windows-gnu/release/rpatchur.exe LICENSE-MIT LICENSE-APACHE /out/

FROM scratch
COPY --from=build /portal /portal
COPY --from=patcher /out/ /patcher/
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/portal"]
