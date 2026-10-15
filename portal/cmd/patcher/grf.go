package main

import (
	"bytes"
	"compress/zlib"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"os"
)

// GRF is the client's archive format. The patcher writes version 0x200,
// the one rpatchur creates and every client since 2004 reads:
//   - a 46-byte header: the magic, a 14-byte key (zeros), the file table's
//     offset after the header (u32), a seed (u32, 0), the file count plus 7
//     (u32) and the version (u32)
//   - each file's zlib-compressed content
//   - the file table: its compressed and uncompressed sizes (u32 each),
//     then zlib data holding, per file, the name (client encoding, `\`
//     separators, NUL-terminated), the compressed size, the compressed size
//     aligned for encryption, the size (u32 each), flags (u8, 1 for a file)
//     and the content's offset after the header (u32)
//
// A patch is applied in place: new content and a new file table are added
// at the end of the file, and the header, rewritten last, switches to
// them. Until then the old table is still whole, so an interrupted patch
// leaves the GRF as it was. Replaced content stays behind as dead space
// until there is more dead space than live content; then the GRF is
// rewritten without it.

const (
	grfMagic      = "Master of Magic\x00"
	grfHeaderSize = 46
	grfVersion    = 0x200
	grfFlagFile   = 1
	// The client reads the file count as count+7 (a leftover from an old
	// scrambling scheme).
	grfCountSkew = 7
)

type grfEntry struct {
	Name       string
	Compressed uint32
	Aligned    uint32
	Size       uint32
	Flags      uint8
	Offset     uint32 // after the header
}

type grfFile struct {
	f       *os.File
	entries []grfEntry
	index   map[string]int // grfKey(name) -> entries index
	end     int64          // where new data goes: the end of the file
}

// openGRF opens a GRF for patching, creating an empty one if it's missing.
func openGRF(path string) (*grfFile, error) {
	f, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o644)
	if err != nil {
		return nil, err
	}
	g := &grfFile{f: f, index: map[string]int{}}
	if err := g.load(); err != nil {
		f.Close()
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	return g, nil
}

func (g *grfFile) load() error {
	st, err := g.f.Stat()
	if err != nil {
		return err
	}
	g.end = st.Size()
	if g.end == 0 {
		// A new GRF: an empty table after the header.
		g.end = grfHeaderSize
		return g.commit()
	}
	head := make([]byte, grfHeaderSize)
	if _, err := g.f.ReadAt(head, 0); err != nil {
		return errors.New("not a GRF (too short)")
	}
	if string(head[:16]) != grfMagic {
		return errors.New("not a GRF")
	}
	if v := binary.LittleEndian.Uint32(head[42:]); v != grfVersion {
		return fmt.Errorf("GRF version 0x%x can't be patched, only 0x200; delete it and start the patcher again to download it fresh", v)
	}
	tableAt := int64(binary.LittleEndian.Uint32(head[30:])) + grfHeaderSize
	count := int64(binary.LittleEndian.Uint32(head[38:])) - grfCountSkew
	sizes := make([]byte, 8)
	if _, err := g.f.ReadAt(sizes, tableAt); err != nil {
		return errors.New("the GRF's file table is missing")
	}
	compressed := int64(binary.LittleEndian.Uint32(sizes))
	size := int64(binary.LittleEndian.Uint32(sizes[4:]))
	if tableAt+8+compressed > g.end || size > maxTableSize {
		return errors.New("the GRF's file table is damaged")
	}
	table, err := inflate(io.NewSectionReader(g.f, tableAt+8, compressed), size)
	if err != nil {
		return fmt.Errorf("reading the GRF's file table: %w", err)
	}
	for i := 0; i < len(table); {
		nul := bytes.IndexByte(table[i:], 0)
		if nul < 0 || i+nul+1+17 > len(table) {
			return errors.New("the GRF's file table is damaged")
		}
		e := grfEntry{Name: string(table[i : i+nul])}
		i += nul + 1
		e.Compressed = binary.LittleEndian.Uint32(table[i:])
		e.Aligned = binary.LittleEndian.Uint32(table[i+4:])
		e.Size = binary.LittleEndian.Uint32(table[i+8:])
		e.Flags = table[i+12]
		e.Offset = binary.LittleEndian.Uint32(table[i+13:])
		i += 17
		g.index[grfKey(e.Name)] = len(g.entries)
		g.entries = append(g.entries, e)
	}
	if int64(len(g.entries)) != count {
		return fmt.Errorf("the GRF lists %d files but its header says %d", len(g.entries), count)
	}
	return nil
}

// add stores a file from its zlib-compressed content.
func (g *grfFile) add(name string, compressed io.Reader, compressedSize, size uint32) error {
	if g.end-grfHeaderSize+int64(compressedSize) > 0xffffffff {
		return errors.New("the GRF would be larger than 4 GB")
	}
	if _, err := g.f.Seek(g.end, io.SeekStart); err != nil {
		return err
	}
	if n, err := io.CopyN(g.f, compressed, int64(compressedSize)); err != nil {
		return fmt.Errorf("%s: wrote %d of %d bytes: %w", name, n, compressedSize, err)
	}
	e := grfEntry{
		Name:       name,
		Compressed: compressedSize,
		Aligned:    compressedSize,
		Size:       size,
		Flags:      grfFlagFile,
		Offset:     uint32(g.end - grfHeaderSize),
	}
	g.end += int64(compressedSize)
	if i, ok := g.index[grfKey(name)]; ok {
		g.entries[i] = e
		return nil
	}
	g.index[grfKey(name)] = len(g.entries)
	g.entries = append(g.entries, e)
	return nil
}

func (g *grfFile) remove(name string) {
	i, ok := g.index[grfKey(name)]
	if !ok {
		return
	}
	g.entries = append(g.entries[:i], g.entries[i+1:]...)
	g.index = map[string]int{}
	for j, e := range g.entries {
		g.index[grfKey(e.Name)] = j
	}
}

// commit writes the file table at the end and then the header that points
// at it.
func (g *grfFile) commit() error {
	var table bytes.Buffer
	for _, e := range g.entries {
		table.WriteString(e.Name)
		table.WriteByte(0)
		for _, n := range []uint32{e.Compressed, e.Aligned, e.Size} {
			binary.Write(&table, binary.LittleEndian, n)
		}
		table.WriteByte(e.Flags)
		binary.Write(&table, binary.LittleEndian, e.Offset)
	}
	var z bytes.Buffer
	zw := zlib.NewWriter(&z)
	zw.Write(table.Bytes())
	zw.Close()

	tableAt := g.end
	if tableAt-grfHeaderSize > 0xffffffff {
		return errors.New("the GRF would be larger than 4 GB")
	}
	var block bytes.Buffer
	binary.Write(&block, binary.LittleEndian, uint32(z.Len()))
	binary.Write(&block, binary.LittleEndian, uint32(table.Len()))
	block.Write(z.Bytes())
	if _, err := g.f.WriteAt(block.Bytes(), tableAt); err != nil {
		return err
	}
	if err := g.f.Truncate(tableAt + int64(block.Len())); err != nil {
		return err
	}
	if err := g.f.Sync(); err != nil {
		return err
	}
	head := make([]byte, grfHeaderSize)
	copy(head, grfMagic)
	binary.LittleEndian.PutUint32(head[30:], uint32(tableAt-grfHeaderSize))
	binary.LittleEndian.PutUint32(head[38:], uint32(len(g.entries)+grfCountSkew))
	binary.LittleEndian.PutUint32(head[42:], grfVersion)
	if _, err := g.f.WriteAt(head, 0); err != nil {
		return err
	}
	return g.f.Sync()
}

// wasteful is true when replaced content takes more room than live
// content (and at least a megabyte).
func (g *grfFile) wasteful() bool {
	var live int64
	for _, e := range g.entries {
		live += int64(e.Aligned)
	}
	dead := g.end - grfHeaderSize - live
	return dead > 1<<20 && dead > live
}

// compact rewrites the GRF at path without dead space, through a temporary
// file that replaces it only once it's complete. g is closed.
func (g *grfFile) compact(path string) error {
	tmp := path + ".tmp"
	out, err := os.OpenFile(tmp, os.O_RDWR|os.O_CREATE|os.O_TRUNC, 0o644)
	if err != nil {
		g.close()
		return err
	}
	n := &grfFile{f: out, end: grfHeaderSize, index: map[string]int{}}
	err = func() error {
		if _, err := out.Seek(grfHeaderSize, io.SeekStart); err != nil {
			return err
		}
		for _, e := range g.entries {
			if _, err := io.Copy(out, io.NewSectionReader(g.f, int64(e.Offset)+grfHeaderSize, int64(e.Aligned))); err != nil {
				return err
			}
			e.Offset = uint32(n.end - grfHeaderSize)
			n.end += int64(e.Aligned)
			n.entries = append(n.entries, e)
		}
		return n.commit()
	}()
	out.Close()
	g.close()
	if err != nil {
		os.Remove(tmp)
		return err
	}
	return os.Rename(tmp, path)
}

func (g *grfFile) close() error { return g.f.Close() }

// grfKey is how the client matches names: `/` and `\` alike, ASCII
// letters in any case. The second byte of a CP949 character is left alone,
// since it can be a letter.
func grfKey(name string) string {
	b := []byte(name)
	for i := 0; i < len(b); i++ {
		switch c := b[i]; {
		case c >= 0x81:
			i++ // a lead byte: skip its trail byte
		case c == '/':
			b[i] = '\\'
		case c >= 'A' && c <= 'Z':
			b[i] = c + 'a' - 'A'
		}
	}
	return string(b)
}
