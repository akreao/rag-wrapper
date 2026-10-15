package main

import (
	"strconv"
	"strings"

	"github.com/yuin/goldmark/ast"
	extast "github.com/yuin/goldmark/extension/ast"
	"github.com/yuin/goldmark/text"
)

// latestNewsText is the newest news post as plain text, for the patcher's
// window, which has no HTML: its "## " heading as the title, then the
// post's paragraphs, lists and code as lines of text.
func (c connectInfo) latestNewsText() (title, body string) {
	src := c.newsSource()
	if src == nil {
		return "", ""
	}
	post := latestPost(src)
	doc := markdown.Parser().Parse(text.NewReader(post))
	var blocks []string
	for n := doc.FirstChild(); n != nil; n = n.NextSibling() {
		if h, ok := n.(*ast.Heading); ok && h.Level == 2 && title == "" && len(blocks) == 0 {
			title = inlineText(h, post)
			continue
		}
		if t := blockText(n, post, ""); t != "" {
			blocks = append(blocks, t)
		}
	}
	return title, strings.Join(blocks, "\n\n")
}

// blockText is one block's text, each line prefixed with indent.
func blockText(n ast.Node, src []byte, indent string) string {
	switch n := n.(type) {
	case *ast.List:
		var items []string
		i := n.Start
		for item := n.FirstChild(); item != nil; item = item.NextSibling() {
			bullet := "• "
			if n.IsOrdered() {
				bullet = strconv.Itoa(i) + ". "
				i++
			}
			var parts []string
			for c := item.FirstChild(); c != nil; c = c.NextSibling() {
				if t := blockText(c, src, indent+"   "); t != "" {
					parts = append(parts, t)
				}
			}
			items = append(items, indent+bullet+strings.TrimLeft(strings.Join(parts, "\n"), " "))
		}
		return strings.Join(items, "\n")
	case *ast.FencedCodeBlock, *ast.CodeBlock:
		var lines []string
		segs := n.Lines()
		for i := 0; i < segs.Len(); i++ {
			s := segs.At(i)
			lines = append(lines, indent+"    "+strings.TrimRight(string(s.Value(src)), "\r\n"))
		}
		return strings.Join(lines, "\n")
	case *ast.Blockquote:
		var parts []string
		for c := n.FirstChild(); c != nil; c = c.NextSibling() {
			if t := blockText(c, src, indent+"> "); t != "" {
				parts = append(parts, t)
			}
		}
		return strings.Join(parts, "\n")
	case *ast.ThematicBreak:
		return indent + "----"
	case *extast.TableHeader, *extast.TableRow:
		return indent + inlineText(n, src)
	}
	if n.Type() == ast.TypeBlock && n.FirstChild() != nil && n.FirstChild().Type() == ast.TypeBlock {
		// Tables and other containers: their blocks, one per line.
		var parts []string
		for c := n.FirstChild(); c != nil; c = c.NextSibling() {
			if t := blockText(c, src, indent); t != "" {
				parts = append(parts, t)
			}
		}
		return strings.Join(parts, "\n")
	}
	if t := inlineText(n, src); t != "" {
		return indent + t
	}
	return ""
}

// inlineText is the text inside a paragraph or heading: link text without
// the address, emphasis without the markers.
func inlineText(n ast.Node, src []byte) string {
	var b strings.Builder
	ast.Walk(n, func(c ast.Node, entering bool) (ast.WalkStatus, error) {
		if !entering {
			return ast.WalkContinue, nil
		}
		switch c := c.(type) {
		case *ast.Text:
			b.Write(c.Segment.Value(src))
			if c.HardLineBreak() {
				b.WriteString("\n")
			} else if c.SoftLineBreak() {
				b.WriteString(" ")
			}
		case *ast.String:
			b.Write(c.Value)
		case *ast.AutoLink:
			b.Write(c.URL(src))
			return ast.WalkSkipChildren, nil
		case *extast.TableCell:
			if c.PreviousSibling() != nil {
				b.WriteString(" | ")
			}
		}
		return ast.WalkContinue, nil
	})
	return strings.TrimSpace(b.String())
}
