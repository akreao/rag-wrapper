# The account web portal: one static Go binary with its pages built in.
FROM golang:1.24-alpine AS build
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /portal .

FROM scratch
COPY --from=build /portal /portal
USER 65534:65534
EXPOSE 8080
ENTRYPOINT ["/portal"]
