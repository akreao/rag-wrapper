package main

import (
	"bytes"
	"compress/zlib"
	"fmt"
	"io"
	"testing"
)

// grfSection is a GRF entry's compressed content.
func grfSection(g *grfFile, e grfEntry) io.Reader {
	return io.NewSectionReader(g.f, int64(e.Offset)+grfHeaderSize, int64(e.Compressed))
}

func deflate(t *testing.T, b []byte) []byte {
	t.Helper()
	var buf bytes.Buffer
	z := zlib.NewWriter(&buf)
	z.Write(b)
	z.Close()
	return buf.Bytes()
}

func hex8(n uint32) string { return fmt.Sprintf("%08x", n) }
