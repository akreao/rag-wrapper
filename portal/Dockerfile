# The account web portal: one static Go binary with its pages built in,
# plus the Windows client patchers it hands out to players.
FROM golang:1.24-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /portal .

# Our own launcher (cmd/patcher), cross-compiled for Windows. go-winres adds
# the manifest that gives it Windows' current look for its buttons and
# progress bar, the desktop app's icon (a copy in cmd/patcher), and the file
# details Windows shows.
ARG GO_WINRES_VERSION=v0.3.3
RUN go run github.com/tc-hib/go-winres@${GO_WINRES_VERSION} simply --arch amd64 --manifest gui --icon cmd/patcher/icon.png \
      --out cmd/patcher/rsrc --product-name "rag-wrapper launcher" --file-description "rag-wrapper game launcher" \
      --original-filename rag-wrapper-patcher.exe \
 && mkdir /patcher \
 && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build -trimpath -ldflags="-s -w -H windowsgui" \
      -o /patcher/rag-wrapper-patcher.exe ./cmd/patcher \
 && cp "$(go env GOROOT)/LICENSE" /patcher/LICENSE-Go \
 && for m in gioui.org gioui.org/shader github.com/go-text/typesetting golang.org/x/exp golang.org/x/exp/shiny \
      golang.org/x/image golang.org/x/sys golang.org/x/text; do \
      printf '%s\n\n' "== $m ($(go list -m -f '{{.Version}}' $m)) =="; cat "$(go list -m -f '{{.Dir}}' $m)/LICENSE"; printf '\n\n'; \
    done > /patcher/LICENSE-third-party \
 && { printf '%s\n\n' "== Nunito font =="; cat cmd/patcher/fonts/OFL.txt; } >> /patcher/LICENSE-third-party

# rpatchur (github.com/L1nkZ/rpatchur, MIT/Apache-2.0), the patcher ours
# replaces, stays until ours is confirmed on players' PCs. It has no published
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
COPY --from=build /patcher/ /patcher/
COPY --from=patcher /out/ /patcher/
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/portal"]
