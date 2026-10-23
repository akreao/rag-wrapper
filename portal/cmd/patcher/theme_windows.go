package main

import (
	_ "embed"
	"image"
	"image/color"

	"gioui.org/f32"
	"gioui.org/font"
	"gioui.org/font/opentype"
	"gioui.org/io/pointer"
	"gioui.org/layout"
	"gioui.org/op"
	"gioui.org/op/clip"
	"gioui.org/op/paint"
	"gioui.org/text"
	"gioui.org/unit"
	"gioui.org/widget"
	"gioui.org/widget/material"
	"golang.org/x/sys/windows/registry"
)

// The launcher's look: the portal's "Game window" theme (portal/static/
// style.css), with its colours, the Nunito font and panels whose first
// line is a blue title bar, like the game client's own windows.

// Nunito, as fixed weights of the portal's variable font (Gio reads TTF,
// not WOFF2). Its licence is fonts/OFL.txt, in the download too.
var (
	//go:embed fonts/nunito-400.ttf
	nunitoRegular []byte
	//go:embed fonts/nunito-700.ttf
	nunitoBold []byte
	//go:embed fonts/nunito-800.ttf
	nunitoExtraBold []byte
)

// palette is style.css's colour tokens.
type palette struct {
	sky, panel, panelLine, panelShadow, inset, rowLine color.NRGBA
	title, titleDark, titleFg                          color.NRGBA
	fg, muted, link                                    color.NRGBA
	cta, ctaDark                                       color.NRGBA
	ok, okFg, bad, badFg, poring                       color.NRGBA
}

func rgb(c uint32) color.NRGBA {
	return color.NRGBA{R: uint8(c >> 16), G: uint8(c >> 8), B: uint8(c), A: 0xff}
}

var lightPalette = palette{
	sky: rgb(0xcfe6f5), panel: rgb(0xffffff), panelLine: rgb(0x8fb0d6), panelShadow: rgb(0xa9c6e4),
	inset: rgb(0xeef5fc), rowLine: rgb(0xe3edf8),
	title: rgb(0x4f7cc0), titleDark: rgb(0x3b62a0), titleFg: rgb(0xffffff),
	fg: rgb(0x23324a), muted: rgb(0x4a5a74), link: rgb(0x2c5aa0),
	cta: rgb(0xd9466f), ctaDark: rgb(0xa8304f),
	ok: rgb(0xdff3e6), okFg: rgb(0x1d6b3c), bad: rgb(0xfbe3e3), badFg: rgb(0xa12a2a), poring: rgb(0xf28bb0),
}

var darkPalette = palette{
	sky: rgb(0x0f1a2b), panel: rgb(0x172338), panelLine: rgb(0x2f4568), panelShadow: rgb(0x0a1220),
	inset: rgb(0x1e2d47), rowLine: rgb(0x24365a),
	title: rgb(0x2f5596), titleDark: rgb(0x24447a), titleFg: rgb(0xeef4ff),
	fg: rgb(0xe4ebf7), muted: rgb(0x9fb0cc), link: rgb(0x8ab4ff),
	cta: rgb(0xe25a82), ctaDark: rgb(0xa13a5a),
	ok: rgb(0x173826), okFg: rgb(0x8fdcaa), bad: rgb(0x3f1f24), badFg: rgb(0xf5a3a3), poring: rgb(0xf28bb0),
}

// windowsDark is true when Windows is set to dark mode for apps.
func windowsDark() bool {
	k, err := registry.OpenKey(registry.CURRENT_USER, `Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`, registry.QUERY_VALUE)
	if err != nil {
		return false
	}
	defer k.Close()
	v, _, err := k.GetIntegerValue("AppsUseLightTheme")
	return err == nil && v == 0
}

type theme struct {
	*material.Theme
	p palette
}

func newTheme() *theme {
	var faces []font.FontFace
	for _, f := range []struct {
		b []byte
		w font.Weight
	}{{nunitoRegular, font.Normal}, {nunitoBold, font.Bold}, {nunitoExtraBold, font.ExtraBold}} {
		fc, err := opentype.ParseCollection(f.b)
		if err != nil {
			continue
		}
		for _, face := range fc {
			face.Font = font.Font{Typeface: "Nunito", Weight: f.w}
			faces = append(faces, face)
		}
	}
	th := material.NewTheme()
	th.Shaper = text.NewShaper(text.NoSystemFonts(), text.WithCollection(faces))
	th.Face = "Nunito"
	th.TextSize = 15
	t := &theme{Theme: th}
	t.use(lightPalette)
	return t
}

func (t *theme) use(p palette) {
	t.p = p
	t.Palette.Bg = p.panel
	t.Palette.Fg = p.fg
	t.Palette.ContrastBg = p.title
	t.Palette.ContrastFg = p.titleFg
}

// --- Text ---

func (t *theme) text(size unit.Sp, weight font.Weight, c color.NRGBA, s string) material.LabelStyle {
	l := material.Label(t.Theme, size, s)
	l.Font.Weight = weight
	l.Color = c
	return l
}

func (t *theme) body(s string) material.LabelStyle  { return t.text(15, font.Normal, t.p.fg, s) }
func (t *theme) muted(s string) material.LabelStyle { return t.text(13.5, font.Normal, t.p.muted, s) }
func (t *theme) bold(s string) material.LabelStyle  { return t.text(15, font.ExtraBold, t.p.fg, s) }

// --- Drawing ---

func fillRRect(gtx layout.Context, r image.Rectangle, radius int, c color.NRGBA) {
	paint.FillShape(gtx.Ops, c, clip.UniformRRect(r, radius).Op(gtx.Ops))
}

// box draws a raised rounded box filling size: shadow below, border, fill.
func (t *theme) box(gtx layout.Context, size image.Point, fill color.NRGBA) {
	rad := gtx.Dp(10)
	sh := gtx.Dp(3)
	fillRRect(gtx, image.Rect(0, sh, size.X, size.Y), rad, t.p.panelShadow)
	fillRRect(gtx, image.Rect(0, 0, size.X, size.Y-sh), rad, t.p.panelLine)
	b := gtx.Dp(1)
	fillRRect(gtx, image.Rect(b, b, size.X-b, size.Y-sh-b), rad-b, fill)
}

// panel is a game window: a raised box whose top is a blue title bar
// (with three little window buttons on the right, as in the portal). With
// fill, it takes all the space it's given and body gets the rest.
func (t *theme) panel(gtx layout.Context, title string, fill bool, body layout.Widget) layout.Dimensions {
	barH := gtx.Dp(30)
	return layout.Stack{}.Layout(gtx,
		layout.Expanded(func(gtx layout.Context) layout.Dimensions {
			size := gtx.Constraints.Min
			t.box(gtx, size, t.p.panel)
			if title != "" {
				b := gtx.Dp(1)
				rad := gtx.Dp(10) - b
				bar := clip.RRect{Rect: image.Rect(b, b, size.X-b, barH), NW: rad, NE: rad}
				paint.FillShape(gtx.Ops, t.p.title, bar.Op(gtx.Ops))
				// The window buttons.
				dot := gtx.Dp(6)
				for i := 0; i < 3; i++ {
					x := size.X - gtx.Dp(16) - i*gtx.Dp(11) - dot
					y := (barH - dot) / 2
					c := t.p.titleFg
					c.A = 0x8c
					paint.FillShape(gtx.Ops, c, clip.Ellipse(image.Rect(x, y, x+dot, y+dot)).Op(gtx.Ops))
				}
			}
			return layout.Dimensions{Size: size}
		}),
		layout.Stacked(func(gtx layout.Context) layout.Dimensions {
			gtx.Constraints.Min.X = gtx.Constraints.Max.X
			if fill {
				gtx.Constraints.Min.Y = gtx.Constraints.Max.Y
			}
			var children []layout.FlexChild
			if title != "" {
				children = append(children, layout.Rigid(func(gtx layout.Context) layout.Dimensions {
					gtx.Constraints.Min.Y, gtx.Constraints.Max.Y = barH, barH
					return layout.W.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
						return layout.Inset{Top: 4, Left: 14, Right: 60}.Layout(gtx, t.text(13.5, font.ExtraBold, t.p.titleFg, title).Layout)
					})
				}))
			}
			in := layout.Inset{Top: 14, Bottom: 17, Left: 18, Right: 18}
			child := func(gtx layout.Context) layout.Dimensions { return in.Layout(gtx, body) }
			if fill {
				children = append(children, layout.Flexed(1, child))
			} else {
				children = append(children, layout.Rigid(child))
			}
			return layout.Flex{Axis: layout.Vertical}.Layout(gtx, children...)
		}),
	)
}

// Button kinds.
const (
	btnPlain = iota // light, for most buttons
	btnBlue         // the portal's plain button
	btnCTA          // the one main action: Play
)

type buttonStyle struct {
	kind     int
	size     unit.Sp
	disabled bool
	wide     bool // stretch to the width it's given
	tall     bool // Play's size
}

// button draws a portal button: rounded, with a darker edge underneath
// that it sinks into while pressed.
func (t *theme) button(gtx layout.Context, c *widget.Clickable, label string, s buttonStyle) layout.Dimensions {
	if s.disabled {
		gtx = gtx.Disabled()
	}
	bg, edge, fg := t.p.inset, t.p.panelShadow, t.p.fg
	switch s.kind {
	case btnBlue:
		bg, edge, fg = t.p.title, t.p.titleDark, rgb(0xffffff)
	case btnCTA:
		bg, edge, fg = t.p.cta, t.p.ctaDark, rgb(0xffffff)
	}
	if c.Hovered() && !s.disabled {
		switch s.kind {
		case btnPlain:
			bg = t.p.rowLine
		default:
			bg = edge
		}
	}
	if s.disabled {
		bg, edge, fg = fade(bg), fade(edge), fade(fg)
	}
	size := s.size
	if size == 0 {
		size = 15
	}
	padV, padH := unit.Dp(8), unit.Dp(16)
	if s.tall {
		padV, padH = 14, 34
	}
	return c.Layout(gtx, func(gtx layout.Context) layout.Dimensions {
		if !s.disabled {
			pointer.CursorPointer.Add(gtx.Ops)
		}
		edgeH := gtx.Dp(3)
		down := 0
		if c.Pressed() && !s.disabled {
			down = gtx.Dp(2)
		}
		macro := op.Record(gtx.Ops)
		lbl := t.text(size, font.ExtraBold, fg, label)
		lbl.Alignment = text.Middle
		lbl.MaxLines = 1
		inner := gtx
		inner.Constraints.Min.Y = 0
		if !s.wide {
			inner.Constraints.Min.X = 0
		}
		dims := layout.Inset{Top: padV, Bottom: padV, Left: padH, Right: padH}.Layout(inner, func(gtx layout.Context) layout.Dimensions {
			if s.wide {
				return layout.Center.Layout(gtx, lbl.Layout)
			}
			return lbl.Layout(gtx)
		})
		call := macro.Stop()
		w, h := dims.Size.X, dims.Size.Y
		rad := gtx.Dp(8)
		fillRRect(gtx, image.Rect(0, edgeH, w, h+edgeH), rad, edge)
		if s.kind == btnPlain {
			fillRRect(gtx, image.Rect(0, down, w, h+down), rad, t.p.panelLine)
			b := gtx.Dp(1)
			fillRRect(gtx, image.Rect(b, down+b, w-b, h+down-b), rad-b, bg)
		} else {
			fillRRect(gtx, image.Rect(0, down, w, h+down), rad, bg)
		}
		o := op.Offset(image.Pt(0, down)).Push(gtx.Ops)
		call.Add(gtx.Ops)
		o.Pop()
		return layout.Dimensions{Size: image.Pt(w, h+edgeH)}
	})
}

func fade(c color.NRGBA) color.NRGBA {
	c.A = 0x80
	return c
}

// tile is a small inset box with a label above a value, like the home
// page's "Playing now".
func (t *theme) tile(gtx layout.Context, label, value string, valueColor color.NRGBA) layout.Dimensions {
	return layout.Stack{}.Layout(gtx,
		layout.Expanded(func(gtx layout.Context) layout.Dimensions {
			fillRRect(gtx, image.Rectangle{Max: gtx.Constraints.Min}, gtx.Dp(8), t.p.inset)
			return layout.Dimensions{Size: gtx.Constraints.Min}
		}),
		layout.Stacked(func(gtx layout.Context) layout.Dimensions {
			return layout.UniformInset(12).Layout(gtx, func(gtx layout.Context) layout.Dimensions {
				gtx.Constraints.Min.X = gtx.Constraints.Max.X
				return layout.Flex{Axis: layout.Vertical}.Layout(gtx,
					layout.Rigid(t.text(13, font.Bold, t.p.muted, label).Layout),
					layout.Rigid(func(gtx layout.Context) layout.Dimensions {
						l := t.text(17, font.ExtraBold, valueColor, value)
						l.MaxLines = 1
						return l.Layout(gtx)
					}),
				)
			})
		}),
	)
}

// rowLine is the thin line between list rows.
func (t *theme) rowLine(gtx layout.Context) layout.Dimensions {
	h := gtx.Dp(1)
	paint.FillShape(gtx.Ops, t.p.rowLine, clip.Rect{Max: image.Pt(gtx.Constraints.Max.X, h)}.Op())
	return layout.Dimensions{Size: image.Pt(gtx.Constraints.Max.X, h)}
}

// progress is the update's progress bar.
func (t *theme) progress(gtx layout.Context, fraction float32, done bool) layout.Dimensions {
	w, h := gtx.Constraints.Max.X, gtx.Dp(12)
	rad := h / 2
	fillRRect(gtx, image.Rect(0, 0, w, h), rad, t.p.panelLine)
	b := gtx.Dp(1)
	fillRRect(gtx, image.Rect(b, b, w-b, h-b), rad-b, t.p.inset)
	fraction = max(0, min(1, fraction))
	if fw := int(float32(w-2*b) * fraction); fw > 0 {
		c := t.p.title
		if done {
			c = t.p.okFg
		}
		fillRRect(gtx, image.Rect(b, b, b+max(fw, h-2*b), h-b), rad-b, c)
	}
	return layout.Dimensions{Size: image.Pt(w, h)}
}

// poring draws the portal's logo (static/poring.svg, 36x32) at height h.
func (t *theme) poring(gtx layout.Context, h unit.Dp) layout.Dimensions {
	s := float32(gtx.Dp(h)) / 32
	pt := func(x, y float32) f32.Point { return f32.Pt(x*s, y*s) }
	// The body: M18 2 C11 9 4 15 4 21 c0 6 6 9 14 9 s14-3 14-9 C32 15 25 9 18 2z
	body := func() clip.PathSpec {
		var p clip.Path
		p.Begin(gtx.Ops)
		p.MoveTo(pt(18, 2))
		p.CubeTo(pt(11, 9), pt(4, 15), pt(4, 21))
		p.CubeTo(pt(4, 27), pt(10, 30), pt(18, 30))
		p.CubeTo(pt(26, 30), pt(32, 27), pt(32, 21))
		p.CubeTo(pt(32, 15), pt(25, 9), pt(18, 2))
		p.Close()
		return p.End()
	}
	paint.FillShape(gtx.Ops, t.p.poring, clip.Outline{Path: body()}.Op())
	paint.FillShape(gtx.Ops, rgb(0xc9577f), clip.Stroke{Path: body(), Width: 1.2 * s}.Op())
	eye := rgb(0x23324a)
	for _, x := range []float32{13, 23} {
		r := image.Rect(int((x-2)*s), int(18*s), int((x+2)*s), int(22*s))
		paint.FillShape(gtx.Ops, eye, clip.Ellipse(r).Op(gtx.Ops))
	}
	var m clip.Path
	m.Begin(gtx.Ops)
	m.MoveTo(pt(15, 25))
	m.QuadTo(pt(18, 27), pt(21, 25))
	paint.FillShape(gtx.Ops, eye, clip.Stroke{Path: m.End(), Width: 1.6 * s}.Op())
	shine := color.NRGBA{R: 0xff, G: 0xff, B: 0xff, A: 0xb3}
	paint.FillShape(gtx.Ops, shine, clip.Ellipse(image.Rect(int(9.5*s), int(11.5*s), int(14.5*s), int(14.5*s))).Op(gtx.Ops))
	return layout.Dimensions{Size: image.Pt(int(36*s), int(32*s))}
}
