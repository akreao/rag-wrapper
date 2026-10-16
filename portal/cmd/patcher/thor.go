package main

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"errors"
	"fmt"
	"hash/crc32"
	"io"
	"strconv"
	"strings"
)

// THOR is the patch format the desktop app's "Publish client patch" button
// writes (desktop/src-tauri/src/thor.rs, after rpatchur's own writer).
// Little-endian throughout:
//   - header: the magic, use_grf_merging (u8), the entry count (u32), the
//     mode (i16, 48 for "multiple files"), the target GRF's name (u8 length
//     and bytes), then the file table's compressed size and offset (u32 each)
//   - each file's zlib-compressed content
//   - the zlib-compressed file table: per entry the path (u8 length and
//     bytes, Windows separators), flags (u8, 1 for a removal), and for a
//     file its offset, compressed size and size (u32 each)
//
// Only the "multiple files" mode is read: it's the only one the app writes.

const (
	thorMagic         = "ASSF (C) 2007 Aeomin DEV"
	thorMultipleFiles = 48
	integrityFile     = "data.integrity"
	// maxTableSize caps the inflated file table, so a damaged patch can't
	// make the patcher allocate without limit.
	maxTableSize = 64 << 20
)

type thorArchive struct {
	r          io.ReaderAt
	GRFMerging bool   // the files go into a GRF, not the client folder
	GRF        string // the target GRF's name; empty means the default
	Entries    []thorEntry
}

type thorEntry struct {
	Path       string // as stored: client encoding, `\` separators
	Remove     bool
	Offset     int64
	Compressed uint32
	Size       uint32
}

var errNotThor = errors.New("not a THOR patch")

func readThor(r io.ReaderAt, size int64) (*thorArchive, error) {
	head := make([]byte, 32+255+8)
	n, err := r.ReadAt(head, 0)
	if n < 32 && err != nil {
		return nil, errNotThor
	}
	head = head[:n]
	if string(head[:24]) != thorMagic {
		return nil, errNotThor
	}
	a := &thorArchive{r: r, GRFMerging: head[24] == 1}
	count := binary.LittleEndian.Uint32(head[25:])
	if mode := int16(binary.LittleEndian.Uint16(head[29:])); mode != thorMultipleFiles {
		return nil, fmt.Errorf("THOR mode %d isn't supported, only multiple-file patches", mode)
	}
	nameLen := int(head[31])
	if len(head) < 32+nameLen+8 {
		return nil, errNotThor
	}
	a.GRF = string(head[32 : 32+nameLen])
	at := 32 + nameLen
	tableLen := int64(binary.LittleEndian.Uint32(head[at:]))
	tableOffset := int64(binary.LittleEndian.Uint32(head[at+4:]))
	if tableOffset+tableLen > size {
		return nil, errors.New("the patch is cut short (its file table is past the end)")
	}

	table, err := inflate(io.NewSectionReader(r, tableOffset, tableLen), maxTableSize)
	if err != nil {
		return nil, fmt.Errorf("reading the patch's file table: %w", err)
	}
	for i := 0; i < len(table); {
		l := int(table[i])
		if i+1+l+1 > len(table) {
			return nil, errors.New("the patch's file table is damaged")
		}
		e := thorEntry{Path: string(table[i+1 : i+1+l])}
		i += 1 + l
		e.Remove = table[i]&1 == 1
		i++
		if !e.Remove {
			if i+12 > len(table) {
				return nil, errors.New("the patch's file table is damaged")
			}
			e.Offset = int64(binary.LittleEndian.Uint32(table[i:]))
			e.Compressed = binary.LittleEndian.Uint32(table[i+4:])
			e.Size = binary.LittleEndian.Uint32(table[i+8:])
			i += 12
			if e.Offset+int64(e.Compressed) > size {
				return nil, fmt.Errorf("the patch is cut short (%s is past the end)", e.Path)
			}
		}
		a.Entries = append(a.Entries, e)
	}
	if uint32(len(a.Entries)) != count {
		return nil, fmt.Errorf("the patch lists %d files but its header says %d", len(a.Entries), count)
	}
	return a, nil
}

// raw is an entry's zlib-compressed content, as stored.
func (a *thorArchive) raw(e thorEntry) io.Reader {
	return io.NewSectionReader(a.r, e.Offset, int64(e.Compressed))
}

// content is an entry's content, checked against its stated size.
func (a *thorArchive) content(e thorEntry) ([]byte, error) {
	b, err := inflate(a.raw(e), int64(e.Size))
	if err != nil {
		return nil, fmt.Errorf("%s: %w", e.Path, err)
	}
	if len(b) != int(e.Size) {
		return nil, fmt.Errorf("%s: %d bytes instead of %d", e.Path, len(b), e.Size)
	}
	return b, nil
}

// checkIntegrity compares each file with the CRC32 that data.integrity
// lists for it ("<path>=0x<crc>" lines), so a damaged download is never
// applied, GRF entries included. The desktop app lists every file, so a
// patch without data.integrity, or a file it doesn't list, is refused.
func (a *thorArchive) checkIntegrity() error {
	var sums map[string]uint32
	for _, e := range a.Entries {
		if !e.Remove && strings.EqualFold(e.Path, integrityFile) {
			b, err := a.content(e)
			if err != nil {
				return err
			}
			if sums, err = parseIntegrity(b); err != nil {
				return err
			}
		}
	}
	if sums == nil {
		return errors.New("the patch has no data.integrity, so the patcher can't check it")
	}
	for _, e := range a.Entries {
		if e.Remove || strings.EqualFold(e.Path, integrityFile) {
			continue
		}
		want, ok := sums[e.Path]
		if !ok {
			return fmt.Errorf("%s isn't listed in the patch's data.integrity, so the patcher can't check it", e.Path)
		}
		b, err := a.content(e)
		if err != nil {
			return err
		}
		if got := crc32.ChecksumIEEE(b); got != want {
			return fmt.Errorf("%s is damaged (checksum %08x, expected %08x)", e.Path, got, want)
		}
	}
	return nil
}

func parseIntegrity(b []byte) (map[string]uint32, error) {
	sums := map[string]uint32{}
	sc := bufio.NewScanner(bytes.NewReader(b))
	sc.Buffer(nil, 1<<20)
	for sc.Scan() {
		line := strings.TrimRight(sc.Text(), "\r")
		i := strings.LastIndexByte(line, '=')
		if i < 0 {
			continue
		}
		v, err := strconv.ParseUint(strings.TrimPrefix(strings.ToLower(line[i+1:]), "0x"), 16, 32)
		if err != nil {
			return nil, fmt.Errorf("data.integrity: bad line %q", line)
		}
		sums[line[:i]] = uint32(v)
	}
	return sums, sc.Err()
}

// inflate reads zlib data, refusing more than max bytes of output.
func inflate(r io.Reader, max int64) ([]byte, error) {
	z, err := zlib.NewReader(r)
	if err != nil {
		return nil, err
	}
	defer z.Close()
	b, err := io.ReadAll(io.LimitReader(z, max+1))
	if err != nil {
		return nil, err
	}
	if int64(len(b)) > max {
		return nil, errors.New("more data than expected")
	}
	return b, nil
}
